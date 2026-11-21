//! The game without a window: the session, the fixed step, the player's controls, the
//! interpolated poses, the cameras and what the scenario tells. The window feeds it keys,
//! mouse movements, control lines and frame times, and draws what it shows.
//!
//! The session is the source of truth (AGENTS.md rule 1): the game starts the simulation with
//! `sim.start`, stores the player's input with `sim.input` whenever it changes and advances it
//! with `sim.step`, one tick per call so the picture can be interpolated between the last two.

use crate::clock::FixedStep;
use crate::hud::{self, AMMO_SHOW_S, Ammo, Death, HIT_FLASH_S, HudState, Killer};
use crate::input::{Action, Controls, InputRecord, Key};
use crate::notices::Notices;
use crate::pad::{PadConfig, PadMapper, PadState};
use crate::pose::{Pose, Poses};
use crate::rig::{self, ViewMode};
use gamecraft_automation::{Access, Host, rpc};
use gamecraft_engine::Session;
use gamecraft_render::effects::IMPACT_LIFE_S;
use gamecraft_render::hud::Hud;
use gamecraft_render::{Camera, Effects, Impact, Tracer, ViewModelAim};
use gamecraft_sim::{DT, Handling, Projectile};
use gamecraft_world::{EntityId, HeightField, World};
use serde_json::{Value, json};

/// How much of its flight a tracer shows behind its head, seconds: at 800 m/s about 32 m.
pub const TRACER_STREAK_S: f64 = 0.04;

/// A world being played in a window.
pub struct Game {
    host: Host,
    controls: Controls,
    pad: PadMapper,
    step: FixedStep,
    poses: Poses,
    /// The view chosen; [`Game::view`] is what the world's rules allow of it.
    view: ViewMode,
    /// The player's own vertical field of view (`--fov`, `?fov=`), over the world's.
    fov_override: Option<f64>,
    paused: bool,
    /// The input record last stored for the player: a record is sent only when it changes.
    sent: Option<(EntityId, InputRecord)>,
    notices: Notices,
    /// Whether the notices were read at least once since the start.
    noticed: bool,
    /// Lines for the console, taken by [`Game::take_notices`].
    told: Vec<String>,
    /// The last error told, so a failing command each frame is told once.
    last_error: Option<String>,
    /// Where projectiles hit lately: (point, normal, simulation time of the hit).
    hits: Vec<([f64; 3], [f64; 3], f64)>,
    /// Asset files waiting for the view ([`Game::take_assets`]): what the browser fetched.
    assets: Vec<(String, Vec<u8>)>,
    /// How far the player was up to the sights at the tick before the last and at the last,
    /// from the simulation's `Handling::ads_amount`: the view model shows them interpolated
    /// like the camera.
    ads_before: f64,
    ads_last: f64,
    /// Seconds the ammunition stays up still.
    ammo_timer_s: f64,
    /// What the ammunition showed last: (shots fired, selector, reloading); a change shows it.
    ammo_seen: Option<(u64, String, bool)>,
    /// Seconds of the red flash of a hit left.
    hit_flash_s: f64,
    /// Who shot the player down, noted from the `down` event.
    killer: Option<Killer>,
    /// Seconds since the player went down; `None` while alive.
    dead_s: Option<f64>,
}

/// Seconds over which a projectile stops following the lag of its shooter's displayed pose: a
/// fresh shot leaves the muzzle where the camera shows it, then flies on in the world.
pub const TRACER_LAG_FADE_S: f64 = 0.05;

/// The streak of `p` as drawn `alpha` of the way between the last two ticks, like the
/// characters: the projectile is shown where it was at that time (its position is the tick's
/// end), and `lag` (its shooter's displayed position minus its simulated one) moves a fresh
/// shot so it starts at the muzzle the camera shows, fading out over [`TRACER_LAG_FADE_S`].
/// `None` while it is not yet fired at that time or has no length.
pub fn tracer_of(p: &Projectile, alpha: f64, lag: [f64; 3]) -> Option<Tracer> {
    let alpha = if alpha.is_nan() {
        1.0
    } else {
        alpha.clamp(0.0, 1.0)
    };
    // The camera shows the time `(1 - alpha)` of a tick before the last tick's end, and so
    // does the projectile.
    let ahead = (alpha - 1.0) * DT;
    let age_s = p.age_s + ahead;
    if !age_s.is_finite() || age_s <= 0.0 {
        return None;
    }
    let [vx, vy, vz] = p.velocity;
    let [px, py, pz] = p.position;
    let shifted = |t: f64, age_s: f64| {
        let w = (1.0 - age_s / TRACER_LAG_FADE_S).clamp(0.0, 1.0);
        [
            px + vx * t + lag[0] * w,
            py + vy * t + lag[1] * w,
            pz + vz * t + lag[2] * w,
        ]
    };
    let shown = TRACER_STREAK_S.min(age_s);
    let head = shifted(ahead, age_s);
    let tail = shifted(ahead - shown, age_s - shown);
    (head != tail).then_some(Tracer {
        from: tail,
        to: head,
        age_s,
    })
}

/// The steepest the camera tilts with the recoil on top of the aim, degrees.
const RECOIL_PITCH_LIMIT_DEG: f64 = 89.0;

impl Game {
    /// A game of `world`: the simulation is started at once (`sim.start`), and the first
    /// character in id order is the player. Automation may not read or write files
    /// (`Access::default()`).
    pub fn new(world: World) -> Game {
        Game::with_host(Host::new(world, Access::default()))
    }

    /// A game of the project in the folder `dir` (natively): its packs are read with
    /// `project.open`, its models' files go to the view ([`Game::take_assets`]), and the
    /// simulation starts. Automation may read the folder that holds the project (where a
    /// dependency beside it lies) and write nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_project(dir: &std::path::Path) -> Result<Game, String> {
        let dir = std::fs::canonicalize(dir)
            .map_err(|e| format!("the project {} cannot be opened: {e}", dir.display()))?;
        let read_root = dir
            .parent()
            .map_or_else(|| dir.clone(), std::path::Path::to_path_buf);
        let mut host = Host::new(
            World::new("Untitled", 0),
            Access {
                read_root: Some(read_root),
                write_root: None,
            },
        );
        host.execute("project.open", &json!({"path": dir.display().to_string()}))?;
        Ok(Game::with_host(host))
    }

    /// A game on `host`'s document, the simulation started, in the first view the world
    /// allows.
    fn with_host(host: Host) -> Game {
        let view = ViewMode::first_allowed(&host.session().current_world().view);
        let mut game = Game {
            host,
            controls: Controls::default(),
            pad: PadMapper::default(),
            step: FixedStep::new(DT),
            poses: Poses::default(),
            view,
            fov_override: None,
            paused: false,
            sent: None,
            notices: Notices::default(),
            noticed: false,
            told: Vec::new(),
            last_error: None,
            hits: Vec::new(),
            assets: Vec::new(),
            ads_before: 0.0,
            ads_last: 0.0,
            ammo_timer_s: 0.0,
            ammo_seen: None,
            hit_flash_s: 0.0,
            killer: None,
            dead_s: None,
        };
        game.start();
        game
    }

    /// Hands over asset files for the view (the browser fetched them), by their paths inside
    /// a pack's `assets/` folder.
    pub fn provide_assets(&mut self, files: Vec<(String, Vec<u8>)>) {
        self.assets.extend(files);
    }

    /// The asset files the view has not been given yet: those handed over, and (natively) the
    /// files of the models the world shows from the open project. Call it before every frame.
    pub fn take_assets(&mut self) -> Vec<(String, Vec<u8>)> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut files = std::mem::take(&mut self.assets);
            self.host
                .hand_over_assets(&mut |path, bytes| files.push((path.to_string(), bytes)));
            files
        }
        #[cfg(target_arch = "wasm32")]
        std::mem::take(&mut self.assets)
    }

    /// The session (read only).
    pub fn session(&self) -> &Session {
        self.host.session()
    }

    /// The session, to run commands on.
    pub fn session_mut(&mut self) -> &mut Session {
        self.host.session_mut()
    }

    /// The player: the first character in id order of the world shown.
    pub fn player(&self) -> Option<EntityId> {
        self.session()
            .current_world()
            .entities()
            .find(|e| e.character().is_some())
            .map(|e| e.id)
    }

    /// The camera that follows the player: the one chosen, if the world's view rules allow it
    /// (they may change while playing), else the first they allow.
    pub fn view(&self) -> ViewMode {
        self.view.allowed(&self.session().current_world().view)
    }

    /// The player's own vertical field of view in degrees (30 to 120, checked by the caller:
    /// `args::parse_fov`), over the world's `view.fov_y_deg`; `None` uses the world's.
    pub fn set_fov(&mut self, fov_y_deg: Option<f64>) {
        self.fov_override = fov_y_deg.filter(|f| f.is_finite());
    }

    /// The vertical field of view of the player's camera, degrees: the player's own, else the
    /// world's.
    pub fn fov_y_deg(&self) -> f64 {
        self.fov_override
            .unwrap_or(self.session().current_world().view.fov_y_deg)
    }

    /// Whether free look is held (Left Alt): the camera may look away from the aim.
    pub fn is_freelooking(&self) -> bool {
        self.controls.is_freelooking()
    }

    /// Whether the simulation is paused (P).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// A key went down or up. Window keys act here (F1, F5, P; Left Alt holds free look); the
    /// action is returned so the
    /// window can do its part (Esc releases the pointer).
    pub fn key(&mut self, key: Key, pressed: bool) -> Option<Action> {
        if pressed && matches!(key, Key::Reload | Key::SwitchMode) {
            self.ammo_timer_s = AMMO_SHOW_S;
        }
        let action = self.controls.key(key, pressed);
        self.act(action);
        action
    }

    /// A gamepad snapshot for a frame of `dt_s` seconds: sticks and buttons go into the same
    /// controls as the keys (start pauses, select switches the view).
    pub fn pad(&mut self, state: &PadState, dt_s: f64) {
        for action in self.pad.update(state, dt_s, &mut self.controls) {
            self.act(Some(action));
        }
    }

    /// How gamepads are read (dead zone, look speed, inverted look).
    pub fn set_pad_config(&mut self, config: PadConfig) {
        self.pad.config = config;
    }

    /// Does what a window action asks of the game itself.
    fn act(&mut self, action: Option<Action>) {
        match action {
            Some(Action::ToggleView) => {
                let rules = self.session().current_world().view.clone();
                self.view = self.view().next_allowed(&rules);
            }
            Some(Action::Restart) => self.restart(),
            Some(Action::Confirm) => {
                if self.is_dead() {
                    self.restart();
                }
            }
            Some(Action::CheckAmmo) => self.ammo_timer_s = AMMO_SHOW_S,
            Some(Action::Pause) => {
                self.paused = !self.paused;
                self.step.reset();
                self.told
                    .push(if self.paused { "Paused" } else { "Resumed" }.to_string());
            }
            Some(Action::Release) | None => {}
        }
    }

    /// The mouse moved by `dx`, `dy` pixels while the pointer is locked.
    pub fn look(&mut self, dx: f64, dy: f64) {
        self.controls.look(dx, dy);
    }

    /// The fire button (left mouse) went down or up: the launcher's trigger.
    pub fn fire(&mut self, held: bool) {
        self.controls.set_fire(held && !self.is_dead());
    }

    /// The sights button (right mouse) went down or up: aim down the sights while held.
    pub fn aim_down_sights(&mut self, held: bool) {
        self.controls.set_ads(held);
    }

    /// Whether the player aims down the sights (right mouse or the left trigger held).
    pub fn is_aiming(&self) -> bool {
        self.controls.is_aiming()
    }

    /// How far the held model is up to the sights: 0 at the hip, 1 at the sights. It is the
    /// simulation's `Handling::ads_amount` (it moves over the held model's `ads_time_s` while
    /// the sights are held and the weapon is not reloading), interpolated between the last two
    /// ticks like the camera; 0 outside first person.
    pub fn ads_blend(&self) -> f64 {
        if self.view() != ViewMode::FirstPerson {
            return 0.0;
        }
        let alpha = self.step.alpha();
        let blend = self.ads_before + (self.ads_last - self.ads_before) * alpha;
        if blend.is_finite() {
            blend.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The simulation's sights amount of the player now, 0 without a simulation.
    fn sim_ads(&self) -> f64 {
        self.player()
            .and_then(|id| self.session().live()?.handling(id))
            .map_or(0.0, |h| h.ads_amount)
    }

    /// The player's held model, if it has one.
    fn held_model(&self) -> Option<&gamecraft_world::HeldModel> {
        let id = self.player()?;
        self.session().current_world().entity(id)?.held_model()
    }

    /// What recoil and sway add to the player's aim now: (recoil, recoil plus sway), each as
    /// (yaw, pitch) degrees; zero without a launcher or a simulation.
    fn aim_offsets_deg(&self) -> ((f64, f64), (f64, f64)) {
        let handling = self.player().and_then(|id| {
            self.session()
                .live()
                .and_then(|sim| sim.handling(id))
                .copied()
        });
        match handling {
            Some(h) => (
                Handling::to_deg(h.recoil_mrad),
                Handling::to_deg(h.offset_mrad()),
            ),
            None => ((0.0, 0.0), (0.0, 0.0)),
        }
    }

    /// Whether the player's character is down (its health is 0): the death screen shows and
    /// Enter restarts.
    pub fn is_dead(&self) -> bool {
        self.dead_s.is_some()
    }

    /// The player's launcher as the HUD tells it, while playing with one.
    fn ammo(&self) -> Option<Ammo> {
        let id = self.player()?;
        let sim = self.session().live()?;
        let magazine = sim.magazine(id)?;
        let launcher = sim.world().entity(id)?.launcher()?;
        Some(Ammo {
            mode: magazine.mode.name().to_string(),
            left: magazine.left,
            capacity: launcher.magazine,
            spare: magazine.spare,
            reloading: magazine.reloading,
            pouch: self.pouch_rounds(),
        })
    }

    /// The rounds in each of the player's spare magazines, fullest first, from the
    /// simulation's magazine; `None` for an unlimited pouch or without a simulation.
    fn pouch_rounds(&self) -> Option<Vec<u32>> {
        let id = self.player()?;
        self.session().live()?.magazine(id)?.pouch_rounds()
    }

    /// What the window shows over the world: the ammunition for a few seconds after the weapon
    /// did something, the red flash of a hit and the death screen.
    pub fn hud(&self) -> Hud {
        hud::build(&HudState {
            ammo: self.ammo(),
            ammo_visible: self.ammo_timer_s > 0.0,
            hit_flash_s: self.hit_flash_s,
            death: self.dead_s.map(|dead_s| Death {
                killer: self.killer.clone(),
                dead_s,
            }),
        })
    }

    /// Counts the HUD's clocks down by `frame_s`, shows the ammunition when the weapon changed
    /// and notes the player's death.
    fn tick_hud(&mut self, frame_s: f64) {
        let step = if frame_s.is_finite() {
            frame_s.max(0.0)
        } else {
            0.0
        };
        self.ammo_timer_s = (self.ammo_timer_s - step).max(0.0);
        self.hit_flash_s = (self.hit_flash_s - step).max(0.0);
        let seen = self.session().live().and_then(|sim| {
            let m = sim.magazine(self.player()?)?;
            Some((m.shots, m.mode.name().to_string(), m.reloading))
        });
        if seen.is_some() && seen != self.ammo_seen {
            if self.ammo_seen.is_some() {
                self.ammo_timer_s = AMMO_SHOW_S;
            }
            self.ammo_seen = seen;
        }
        let down = self.session().is_playing()
            && self
                .player()
                .and_then(|id| self.session().current_world().entity(id))
                .and_then(|e| e.health())
                .is_some_and(gamecraft_world::Health::is_down);
        self.dead_s = match (down, self.dead_s) {
            (true, Some(s)) => Some(s + step),
            (true, None) => {
                self.controls.set_fire(false);
                Some(0.0)
            }
            (false, _) => None,
        };
    }

    /// The window lost the keyboard: every key is let go.
    pub fn release_keys(&mut self) {
        self.controls.release_all();
    }

    /// A frame of `frame_s` seconds passed: stores a changed input, runs the ticks that are
    /// due (none while paused or stopped) and reads what the scenario tells. Returns the ticks
    /// run.
    pub fn frame(&mut self, frame_s: f64) -> u32 {
        if !self.session().is_playing() || self.paused {
            self.tick_hud(if self.paused { 0.0 } else { frame_s });
            self.step.reset();
            let world = self.host.session().current_world();
            self.poses.observe(world);
            return 0;
        }
        self.send_input();
        let ticks = self.step.advance(frame_s);
        let mut ran = 0;
        for _ in 0..ticks {
            match self.session_mut().run("sim.step", &json!({"ticks": 1})) {
                Ok(answer) => self.note_hits(&answer),
                Err(e) => {
                    self.error(e.to_string());
                    break;
                }
            }
            ran += 1;
            let world = self.host.session().current_world();
            self.poses.ticked(world);
            self.ads_before = self.ads_last;
            self.ads_last = self.sim_ads();
            self.controls.ticked();
            self.send_input();
        }
        self.tick_hud(frame_s);
        if ran == 0 {
            let world = self.host.session().current_world();
            self.poses.observe(world);
        }
        if ran > 0 || !self.noticed {
            self.read_notices();
        }
        ran
    }

    /// Runs one line of the control protocol (`gamecraft-cli serve`'s JSON lines) on the
    /// session and returns the reply line; `None` for a blank line. Never panics.
    pub fn control(&mut self, line: &str) -> Option<String> {
        rpc::handle_line(&mut self.host, line)
    }

    /// Stops the simulation if it runs and starts it again from the document (F5).
    pub fn restart(&mut self) {
        if self.session().is_playing()
            && let Err(e) = self.session_mut().run("sim.stop", &json!({}))
        {
            self.error(e.to_string());
        }
        self.start();
        self.told.push("Restarted".to_string());
    }

    /// The player's interpolated pose and eye height, if there is a player.
    fn player_pose(&self) -> Option<(Pose, f64)> {
        let world = self.session().current_world();
        let id = self.player()?;
        let entity = world.entity(id)?;
        let character = entity.character()?;
        let pose = self.poses.at(id, self.step.alpha()).unwrap_or(Pose {
            position: entity.position,
            yaw_deg: entity.rotation_y_deg,
        });
        Some((pose, character.eye_height_m))
    }

    /// The camera of the view, for a picture of width / height `aspect`: the player's eyes or
    /// boom at its interpolated pose, looking where the head looks (the aim, unless free look
    /// is held), as wide as [`Game::fov_y_deg`]; without a player, a view of the whole world.
    pub fn camera(&self, aspect: f64) -> Camera {
        let session = self.session();
        let world = session.current_world();
        let heights = session.current_heights();
        let Some((pose, eye_height_m)) = self.player_pose() else {
            return Camera::framing(world, heights, aspect);
        };
        // The recoil climbs the view on top of where the player looks; the player pulls it
        // back with the mouse, and it settles by itself.
        let (yaw, pitch) = self.controls.view_deg();
        let ((kick_yaw, kick_pitch), _) = self.aim_offsets_deg();
        let (yaw, pitch) = (
            yaw + kick_yaw,
            (pitch + kick_pitch).clamp(-RECOIL_PITCH_LIMIT_DEG, RECOIL_PITCH_LIMIT_DEG),
        );
        let mut camera = match self.view() {
            ViewMode::FirstPerson => rig::first_person(&pose, eye_height_m, yaw, pitch),
            ViewMode::ThirdPerson => rig::third_person(&pose, eye_height_m, yaw, pitch, heights),
        };
        let narrowing = self
            .held_model()
            .map_or(1.0, |h| h.fov_factor_at(self.ads_blend()));
        camera.fov_y_deg = self.fov_y_deg() * narrowing;
        if camera.validate().is_ok() {
            camera
        } else {
            Camera::framing(world, heights, aspect)
        }
    }

    /// Where the player's held model (a weapon's view model) is drawn: in first person, at the
    /// eyes along the aim (the input record's yaw and pitch plus the recoil and the sway, so
    /// the sights wander and climb), which free look does not turn, as far up to the sights as
    /// [`Game::ads_blend`]; `None` in third person or without a player.
    pub fn view_model_aim(&self) -> Option<ViewModelAim> {
        if self.view() != ViewMode::FirstPerson {
            return None;
        }
        let (pose, eye_height_m) = self.player_pose()?;
        let [x, y, z] = pose.position;
        let (_, (yaw, pitch)) = self.aim_offsets_deg();
        Some(ViewModelAim {
            eye: [x, y + eye_height_m, z],
            yaw_deg: self.controls.yaw_deg() + yaw,
            pitch_deg: (self.controls.pitch_deg() + pitch)
                .clamp(-RECOIL_PITCH_LIMIT_DEG, RECOIL_PITCH_LIMIT_DEG),
            ads: self.ads_blend(),
            reload: self.reload_progress(),
        })
    }

    /// How far through a reload the player's launcher is, 0 to 1, `None` when it is not
    /// reloading: the simulation's own `Magazine::reload_progress`.
    pub fn reload_progress(&self) -> Option<f64> {
        let id = self.player()?;
        let magazine = self.session().live()?.magazine(id)?;
        magazine.reloading.then(|| magazine.reload_progress())
    }

    /// Whether the world's view rules let the window draw a crosshair.
    pub fn crosshair_allowed(&self) -> bool {
        self.session().current_world().view.crosshair
    }

    /// What is in flight and what just hit, for the renderer: a streak behind the head of each
    /// projectile (between its last two ticks, like the characters) and a puff for each hit of
    /// the last second.
    pub fn effects(&self) -> Effects {
        let Some(sim) = self.session().live() else {
            return Effects::default();
        };
        let alpha = self.step.alpha();
        let tracers = sim
            .projectiles()
            .iter()
            .filter_map(|p| {
                let lag = self.shooter_lag(p.shooter, alpha);
                tracer_of(p, alpha, lag)
            })
            .collect();
        let now = self.time_s();
        let impacts = self
            .hits
            .iter()
            .map(|&(point, normal, at)| Impact {
                point,
                normal,
                age_s: (now - at).max(0.0),
            })
            .collect();
        Effects { tracers, impacts }
    }

    /// How far behind its simulated position a character is shown: its interpolated position
    /// minus where the last tick left it (zero without a pose).
    fn shooter_lag(&self, id: EntityId, alpha: f64) -> [f64; 3] {
        let Some(pose) = self.poses.at(id, alpha) else {
            return [0.0; 3];
        };
        let Some(entity) = self.session().current_world().entity(id) else {
            return [0.0; 3];
        };
        let [a, b] = [pose.position, entity.position];
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// The world as shown: the running world (or the document) with every character between
    /// its last two ticks, the player turned as the mouse points. For display only.
    pub fn display_world(&self) -> World {
        let mut world = self.session().current_world().clone();
        self.poses.apply(&mut world, self.step.alpha());
        if self.session().is_playing()
            && let Some(e) = self.player().and_then(|id| world.entity_mut(id))
        {
            e.rotation_y_deg = self.controls.yaw_deg();
        }
        world
    }

    /// The character not to draw: the player in first person (the camera is inside it).
    pub fn hidden_character(&self) -> Option<EntityId> {
        match self.view() {
            ViewMode::FirstPerson => self.player(),
            ViewMode::ThirdPerson => None,
        }
    }

    /// The height field of the world shown.
    pub fn heights(&self) -> Option<&HeightField> {
        self.session().current_heights()
    }

    /// The animation clock for the picture: the simulation's time between its last two ticks,
    /// 0 when stopped.
    pub fn time_s(&self) -> f64 {
        self.session()
            .live()
            .map_or(0.0, |sim| sim.time_s() + self.step.alpha() * DT)
    }

    /// The window's title: the world's name, the state and the newest message.
    pub fn title(&self) -> String {
        let mut title = format!("GameCraft: {}", self.session().current_world().name);
        if !self.session().is_playing() {
            title.push_str(" (stopped; F5 starts)");
        } else if self.paused {
            title.push_str(" (paused)");
        }
        if let Some(message) = self.notices.message() {
            title.push_str(" | ");
            title.push_str(message);
        }
        title
    }

    /// The lines to print since the last call: messages, objective changes, errors.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.told)
    }

    /// Keeps the hits of a `sim.step` answer (its `hit` events) for the puffs, and forgets those
    /// older than a puff lasts.
    fn note_hits(&mut self, answer: &Value) {
        let now = self
            .session()
            .live()
            .map_or(0.0, gamecraft_sim::Sim::time_s);
        let vec3 = |v: Option<&Value>| -> Option<[f64; 3]> {
            let list = v?.as_array()?;
            let mut it = list.iter().map(Value::as_f64);
            let out = [it.next()??, it.next()??, it.next()??];
            it.next().is_none().then_some(out)
        };
        let events = answer.get("events").and_then(Value::as_array);
        for e in events.into_iter().flatten() {
            self.note_player_event(e);
            if e.get("event").and_then(Value::as_str) != Some("hit") {
                continue;
            }
            if let (Some(point), Some(normal)) = (vec3(e.get("point")), vec3(e.get("normal"))) {
                self.hits.push((point, normal, now));
            }
        }
        self.hits.retain(|&(_, _, at)| now - at < IMPACT_LIFE_S);
    }

    /// Notes a hit on the player (the red flash) and the shot that downed it (who and from how
    /// far), from a `hit` or `down` event of a step.
    fn note_player_event(&mut self, event: &Value) {
        let Some(player) = self.player() else {
            return;
        };
        let id_of = |key: &str| event.get(key).and_then(Value::as_u64).map(EntityId);
        if id_of("target") != Some(player) {
            return;
        }
        let shooter = id_of("shooter").or_else(|| id_of("who"));
        match event.get("event").and_then(Value::as_str) {
            Some("hit") if shooter != Some(player) => self.hit_flash_s = HIT_FLASH_S,
            Some("down") => {
                let world = self.session().current_world();
                self.killer = shooter.and_then(|who| {
                    let (a, b) = (world.entity(who)?, world.entity(player)?);
                    let distance_m = a
                        .position
                        .iter()
                        .zip(&b.position)
                        .map(|(x, y)| (x - y) * (x - y))
                        .sum::<f64>()
                        .sqrt();
                    Some(Killer {
                        name: a.name.clone(),
                        distance_m,
                    })
                });
            }
            _ => {}
        }
    }

    /// Starts the simulation and puts the controls, the step and the poses at its start.
    fn start(&mut self) {
        if let Err(e) = self.session_mut().run("sim.start", &json!({})) {
            self.error(e.to_string());
        }
        let yaw = self
            .player()
            .and_then(|id| self.session().current_world().entity(id))
            .map_or(0.0, |e| e.rotation_y_deg);
        self.controls.release_all();
        self.controls.face(yaw);
        self.step.reset();
        self.poses = Poses::new(self.host.session().current_world());
        self.paused = false;
        self.notices.clear();
        self.noticed = false;
        self.hits.clear();
        self.ads_before = 0.0;
        self.ads_last = 0.0;
        self.ammo_timer_s = 0.0;
        self.ammo_seen = None;
        self.hit_flash_s = 0.0;
        self.killer = None;
        self.dead_s = None;
        // A character's first input starts from standing still facing its rotation: that is
        // what the controls ask for now, so nothing is sent until they change (and an agent's
        // input is not overwritten meanwhile).
        self.sent = self.player().map(|id| (id, self.controls.record()));
    }

    /// Stores the controls' record for the player when it differs from the last one sent.
    fn send_input(&mut self) {
        let Some(player) = self.player() else {
            return;
        };
        let record = self.controls.record();
        if self.sent == Some((player, record)) {
            return;
        }
        match record.send(self.host.session_mut(), player) {
            Ok(_) => self.sent = Some((player, record)),
            Err(e) => self.error(e),
        }
    }

    /// Reads `sim.state` and the objectives for new things to tell.
    fn read_notices(&mut self) {
        let state = match self.session_mut().run("sim.state", &json!({})) {
            Ok(state) => state,
            Err(e) => {
                self.error(e.to_string());
                Value::Null
            }
        };
        let objectives = self.session().current_world().objectives.clone();
        let lines = self.notices.update(&state, &objectives);
        self.told.extend(lines);
        self.noticed = true;
    }

    /// Tells an error once (until another one comes).
    fn error(&mut self, e: String) {
        if self.last_error.as_ref() != Some(&e) {
            self.told.push(format!("Error: {e}"));
            self.last_error = Some(e);
        }
    }
}
