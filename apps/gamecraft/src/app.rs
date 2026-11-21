//! The window: winit around a [`Game`] and its [`GameView`], natively and in the browser
//! (WebGPU only). Keys and the mouse go to the game, frames to the surface, the scenario's
//! messages to the console and the newest one into the title.
//!
//! Natively, `--stdin` reads the control protocol's JSON lines on stdin, runs each through the
//! same session between two frames and answers on stdout (AGENTS.md rules 1 and 5: an agent
//! drives the window a person is watching). Everything else the window says goes to stderr,
//! so stdout carries only replies.

use crate::game::Game;
use crate::gamepads::Pads;
use crate::input::{Action, Key};
use crate::pad::PadConfig;
use crate::view::GameView;
use gamecraft_render::Quality;
use std::sync::Arc;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Frames after the pointer locks during which mouse movement is ignored: a browser can
/// report the jump to the locked position as one large movement.
const SETTLE_FRAMES: u8 = 2;
/// The window's first size natively, logical pixels.
#[cfg(not(target_arch = "wasm32"))]
const START_SIZE: (f64, f64) = (1280.0, 720.0);

/// The game key of a physical key: WASD, Shift, Space, E, B, R, Left Alt, F1, F5, P and Esc.
pub fn key_of(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyW => Key::Forward,
        KeyCode::KeyS => Key::Back,
        KeyCode::KeyA => Key::Left,
        KeyCode::KeyD => Key::Right,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Key::Run,
        KeyCode::Space => Key::Jump,
        KeyCode::KeyE => Key::Interact,
        KeyCode::KeyB => Key::SwitchMode,
        KeyCode::KeyR => Key::Reload,
        KeyCode::AltLeft => Key::Freelook,
        KeyCode::F1 => Key::ToggleView,
        KeyCode::F5 => Key::Restart,
        KeyCode::KeyP => Key::Pause,
        KeyCode::Escape => Key::Release,
        _ => return None,
    })
}

/// Tells a line to the person: stderr natively, the browser's console on the web.
pub fn tell(line: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::web::console(line);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "{line}");
    }
}

/// The device, the window's surface and the game's view.
struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view: GameView,
}

impl Gpu {
    /// A device for `window`'s surface, the surface configured, the renderer made.
    async fn new(window: Arc<Window>, quality: Quality) -> Result<Gpu, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("the window has no drawing surface: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no graphics adapter can draw into the window: {e}"))?;
        let required_features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gamecraft"),
                required_features,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("the graphics adapter gave no device: {e}"))?;
        // A GPU error is told, never a crash (wgpu's default handler panics).
        device.on_uncaptured_error(Arc::new(|e| tell(&format!("GPU error: {e}"))));
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| "the window's surface offers no format to draw in".to_string())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: Vec::new(),
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let view = GameView::new(
            device.clone(),
            queue.clone(),
            quality,
            (config.width, config.height),
        );
        Ok(Gpu {
            surface,
            device,
            queue,
            config,
            view,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.view.resize(width, height);
    }

    /// Draws a frame of `game` into the surface and presents it.
    fn draw(&mut self, game: &Game) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gamecraft frame"),
            });
        match self.view.draw(game, &target, &mut encoder) {
            Ok(problems) => problems.iter().for_each(|p| tell(p)),
            Err(e) => tell(&e.to_string()),
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

/// What the browser's start-up hands the window once it is done.
#[cfg(target_arch = "wasm32")]
type Pending = std::rc::Rc<std::cell::RefCell<Option<Result<(Game, Gpu), String>>>>;

/// The window application.
struct App {
    quality: Quality,
    game: Option<Game>,
    /// The gamepads; opened with the window.
    pads: Option<Pads>,
    /// How a pad is read (`--invert-look`).
    pad_config: PadConfig,
    gpu: Option<Gpu>,
    window: Option<Arc<Window>>,
    /// Whether the pointer is locked to the window (mouse look).
    captured: bool,
    /// Frames left in which mouse movement is ignored after locking ([`SETTLE_FRAMES`]).
    settling: u8,
    last_frame: Option<Instant>,
    title: String,
    /// Why the window could not start (natively: the exit's reason).
    failure: Option<String>,
    /// Lines read from stdin by the reader thread (`--stdin`), or why a line was refused.
    #[cfg(not(target_arch = "wasm32"))]
    stdin: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    #[cfg(target_arch = "wasm32")]
    pending: Pending,
}

impl App {
    fn new(quality: Quality, game: Option<Game>, pad_config: PadConfig) -> App {
        App {
            quality,
            game,
            pads: None,
            pad_config,
            gpu: None,
            window: None,
            captured: false,
            settling: 0,
            last_frame: None,
            title: String::new(),
            failure: None,
            #[cfg(not(target_arch = "wasm32"))]
            stdin: None,
            #[cfg(target_arch = "wasm32")]
            pending: Pending::default(),
        }
    }

    fn capture(&mut self) {
        let Some(window) = &self.window else { return };
        let locked = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
        if locked.is_ok() {
            window.set_cursor_visible(false);
            self.captured = true;
            self.settling = SETTLE_FRAMES;
        }
    }

    fn release(&mut self) {
        if let Some(window) = &self.window {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.captured = false;
        if let Some(game) = &mut self.game {
            game.fire(false);
            game.aim_down_sights(false);
        }
    }

    /// One frame: control lines, the game's ticks, what it tells, the title and the picture.
    fn frame(&mut self) {
        #[cfg(target_arch = "wasm32")]
        self.adopt();
        self.settling = self.settling.saturating_sub(1);
        let now = Instant::now();
        let frame_s = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);
        #[cfg(not(target_arch = "wasm32"))]
        self.serve_stdin();
        let Some(game) = &mut self.game else { return };
        // Opened on the first frame: the browser reports pads only once the page is running.
        let pads = self.pads.get_or_insert_with(Pads::new);
        game.set_pad_config(self.pad_config);
        game.pad(&pads.poll(), frame_s);
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            game.frame(frame_s);
        }));
        if stepped.is_err() {
            tell("Error: internal error (a bug) while stepping the game; it carries on.");
        }
        for line in game.take_notices() {
            tell(&line);
        }
        let title = game.title();
        if title != self.title {
            if let Some(window) = &self.window {
                window.set_title(&title);
            }
            // winit's web window leaves the page's title alone.
            #[cfg(target_arch = "wasm32")]
            crate::web::set_title(&title);
            self.title = title;
        }
        let files = game.take_assets();
        if let Some(gpu) = &mut self.gpu {
            gpu.view.provide_assets(files);
            gpu.draw(game);
        }
    }

    /// Runs the control lines that arrived since the last frame and answers each on stdout.
    #[cfg(not(target_arch = "wasm32"))]
    fn serve_stdin(&mut self) {
        use std::io::Write;
        let (Some(lines), Some(game)) = (&self.stdin, &mut self.game) else {
            return;
        };
        while let Ok(line) = lines.try_recv() {
            let reply = match line {
                Ok(line) => game.control(&line),
                Err(why) => {
                    Some(serde_json::json!({"id": null, "ok": false, "error": why}).to_string())
                }
            };
            if let Some(reply) = reply {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{reply}");
                let _ = out.flush();
            }
        }
    }

    /// Takes over the game and the GPU once the browser's start-up finished.
    #[cfg(target_arch = "wasm32")]
    fn adopt(&mut self) {
        let Some(result) = self.pending.borrow_mut().take() else {
            return;
        };
        match result {
            Ok((game, gpu)) => {
                self.game = Some(game);
                self.gpu = Some(gpu);
                crate::web::ready();
            }
            Err(why) => {
                tell(&why);
                crate::web::fail("GameCraft could not start", &why);
                self.failure = Some(why);
            }
        }
    }

    /// The window's attributes on this platform.
    fn attributes() -> winit::window::WindowAttributes {
        let attributes = Window::default_attributes().with_title("GameCraft");
        #[cfg(not(target_arch = "wasm32"))]
        let attributes =
            attributes.with_inner_size(winit::dpi::LogicalSize::new(START_SIZE.0, START_SIZE.1));
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes
                .with_canvas(crate::web::canvas())
                .with_prevent_default(true)
        };
        attributes
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || self.failure.is_some() {
            return;
        }
        let window = match event_loop.create_window(App::attributes()) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                let why = format!("the window could not be opened: {e}");
                tell(&why);
                #[cfg(target_arch = "wasm32")]
                crate::web::fail("GameCraft could not start", &why);
                self.failure = Some(why);
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window.clone());
        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(Gpu::new(window.clone(), self.quality)) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(why) => {
                self.failure = Some(why);
                event_loop.exit();
                return;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let pending = self.pending.clone();
            let quality = self.quality;
            let window = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let started = async {
                    crate::web::wait_for_webgpu().await?;
                    let mut world = crate::web::load_world().await?;
                    let files = crate::web::load_assets(&mut world).await;
                    let gpu = Gpu::new(window.clone(), quality).await?;
                    let mut game = Game::new(world);
                    game.set_fov(web_fov());
                    game.provide_assets(files);
                    Ok((game, gpu))
                }
                .await;
                *pending.borrow_mut() = Some(started);
                window.request_redraw();
            });
        }
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let (Some(key), Some(game)) = (key_of(code), &mut self.game) else {
                    return;
                };
                if event.repeat {
                    return;
                }
                let pressed = event.state == ElementState::Pressed;
                if game.key(key, pressed) == Some(Action::Release) {
                    self.release();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                if pressed && !self.captured {
                    // The click that locks the pointer does not fire.
                    self.capture();
                } else if let Some(game) = &mut self.game {
                    game.fire(pressed && self.captured);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                if let Some(game) = &mut self.game {
                    game.aim_down_sights(state == ElementState::Pressed && self.captured);
                }
            }
            WindowEvent::Focused(false) => {
                if let Some(game) = &mut self.game {
                    game.release_keys();
                    game.fire(false);
                    game.aim_down_sights(false);
                }
                self.release();
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta: (dx, dy) }, true, 0, Some(game)) =
            (event, self.captured, self.settling, &mut self.game)
        {
            game.look(dx, dy);
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Opens the window natively on `world` and runs until it is closed. With `stdin`, a thread
/// reads control lines (at most the protocol's request size each) for the frames to run.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native(mut game: Game, args: &crate::args::Args) -> Result<(), String> {
    game.set_fov(args.fov);
    let event_loop = EventLoop::new()
        .map_err(|e| format!("no window can be opened here ({e}); GameCraft needs a display"))?;
    let mut app = App::new(
        args.quality,
        Some(game),
        PadConfig {
            invert_look: args.invert_look,
            ..PadConfig::default()
        },
    );
    if args.stdin {
        app.stdin = Some(read_stdin());
    }
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("the window stopped: {e}"))?;
    match app.failure {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// Reads stdin's lines on a thread until it ends.
#[cfg(not(target_arch = "wasm32"))]
fn read_stdin() -> std::sync::mpsc::Receiver<Result<String, String>> {
    use gamecraft_automation::{Line, read_line};
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        loop {
            let line = match read_line(&mut input, gamecraft_automation::MAX_REQUEST_BYTES) {
                Ok(Line::End) | Err(_) => break,
                Ok(Line::Bytes(bytes)) => String::from_utf8(bytes)
                    .map_err(|_| "the request is not valid UTF-8".to_string()),
                Ok(Line::TooLong) => Err(format!(
                    "the request is longer than {} bytes; send a shorter line",
                    gamecraft_automation::MAX_REQUEST_BYTES
                )),
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Reads a `.gamecraft` file; the error names the file and what is wrong.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_world_file(path: &std::path::Path) -> Result<gamecraft_world::World, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("the world {} cannot be read: {e}", path.display()))?;
    gamecraft_world::World::from_json(&text)
        .map_err(|e| format!("the world {} is not a valid world: {e}", path.display()))
}

/// The player's own field of view of `?fov=<degrees>` (30 to 120); a bad value is told on the
/// console and the world's is used.
#[cfg(target_arch = "wasm32")]
fn web_fov() -> Option<f64> {
    let text = crate::web::query("fov")?;
    crate::args::parse_fov(&text)
        .map_err(|e| tell(&format!("?fov={e}; the world's field of view is used")))
        .ok()
}

/// Starts the window in the browser: the canvas `gamecraft_canvas`, the world of `?world=<url>`
/// (else the example world), the quality of `?quality=` (else medium), the field of view of
/// `?fov=` (else the world's).
#[cfg(target_arch = "wasm32")]
pub fn run_web() {
    use winit::platform::web::EventLoopExtWebSys;
    let quality = match crate::web::query("quality") {
        Some(name) => Quality::parse(&name).unwrap_or_else(|e| {
            tell(&e);
            Quality::default()
        }),
        None => Quality::default(),
    };
    match EventLoop::new() {
        Ok(event_loop) => event_loop.spawn_app(App::new(
            quality,
            None,
            PadConfig {
                invert_look: crate::web::query("invert_look").is_some_and(|v| v != "0"),
                ..PadConfig::default()
            },
        )),
        Err(e) => crate::web::fail("GameCraft could not start", &e.to_string()),
    }
}
