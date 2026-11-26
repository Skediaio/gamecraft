//! The game without a window: the session, the fixed step, the player's controls, the
//! interpolated poses, the cameras and what the scenario tells. The window feeds it keys,
//! mouse movements, control lines and frame times, and draws what it shows.
//!
//! The session is the source of truth (AGENTS.md rule 1): the game starts the simulation with
//! `sim.start`, stores the player's input with `sim.input` whenever it changes and advances it
//! with `sim.step`, one tick per call so the picture can be interpolated between the last two.

use crate::clock::FixedStep;
use crate::input::{Action, Controls, InputRecord, Key};
use crate::notices::Notices;
use crate::pad::{PadConfig, PadMapper, PadState};
use crate::pose::{Pose, Poses};
use crate::rig::{self, ViewMode};
use gamecraft_automation::{Access, Host, rpc};
use gamecraft_engine::Session;
use gamecraft_render::effects::IMPACT_LIFE_S;
use gamecraft_render::{Camera, Effects, Impact, Tracer};
use gamecraft_sim::DT;
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
    view: ViewMode,
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
}

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
                max_world_bytes: None,
                cache_beside_files: false,
            },
        );
        host.execute("project.open", &json!({"path": dir.display().to_string()}))?;
        Ok(Game::with_host(host))
    }

    /// A game of the world file `path` (natively), opened as `file.open` opens it: its models
    /// come from the pack `pack` names (a folder with a `pack.json`), else from the nearest
    /// `pack.json` above the file; their files go to the view ([`Game::take_assets`]) and
    /// their bounds to the simulation. Automation may read the folder that holds the file and
    /// the folder that holds the pack (where a dependency beside it lies) and writes nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_world_file(
        path: &std::path::Path,
        pack: Option<&std::path::Path>,
    ) -> Result<Game, String> {
        let path = std::fs::canonicalize(path)
            .map_err(|e| format!("the world {} cannot be opened: {e}", path.display()))?;
        let folder = |p: &std::path::Path| {
            p.parent().map_or_else(
                || std::path::PathBuf::from("/"),
                std::path::Path::to_path_buf,
            )
        };
        let pack_dir = match pack {
            Some(dir) => {
                if !dir.is_dir() {
                    return Err(format!(
                        "the pack `{}` does not exist; --pack names the folder of a pack (it \
                         has a pack.json whose assets/ holds the world's models)",
                        dir.display()
                    ));
                }
                Some(
                    std::fs::canonicalize(dir)
                        .map_err(|e| format!("the pack {} cannot be opened: {e}", dir.display()))?,
                )
            }
            None => gamecraft_automation::find_pack_above(&folder(&path), None),
        };
        let read_root = match &pack_dir {
            Some(pack) => gamecraft_automation::common_ancestor(&folder(&path), &folder(pack)),
            None => folder(&path),
        };
        let mut host = Host::new(
            World::new("Untitled", 0),
            Access {
                read_root: Some(read_root),
                write_root: None,
                max_world_bytes: None,
                cache_beside_files: false,
            },
        );
        // Only a pack given (--pack) is the host's own; one found above the file is that
        // file's.
        if let (Some(dir), Some(_)) = (&pack_dir, pack) {
            host = host.with_pack(dir.clone());
        }
        host.execute("file.open", &json!({"path": path.display().to_string()}))
            .map_err(|e| format!("the world {}: {e}", path.display()))?;
        Ok(Game::with_host(host))
    }

    /// A game on `host`'s document, the simulation started.
    fn with_host(host: Host) -> Game {
        let mut game = Game {
            host,
            controls: Controls::default(),
            pad: PadMapper::default(),
            step: FixedStep::new(DT),
            poses: Poses::default(),
            view: ViewMode::default(),
            paused: false,
            sent: None,
            notices: Notices::default(),
            noticed: false,
            told: Vec::new(),
            last_error: None,
            hits: Vec::new(),
            assets: Vec::new(),
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

    /// The camera that follows the player.
    pub fn view(&self) -> ViewMode {
        self.view
    }

    /// Whether the simulation is paused (P).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// A key went down or up. Window keys act here (F1, F5, P); the action is returned so the
    /// window can do its part (Esc releases the pointer).
    pub fn key(&mut self, key: Key, pressed: bool) -> Option<Action> {
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
            Some(Action::ToggleView) => self.view = self.view.toggled(),
            Some(Action::Restart) => self.restart(),
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

    /// The fire button (left mouse) went down or up; the launcher fires while it is held.
    pub fn fire(&mut self, held: bool) {
        self.controls.set_fire(held);
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
            self.controls.ticked();
            self.send_input();
        }
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

    /// The camera of the view, for a picture of width / height `aspect`: the player's eyes or
    /// boom at its interpolated pose; without a player, a view of the whole world.
    pub fn camera(&self, aspect: f64) -> Camera {
        let session = self.session();
        let world = session.current_world();
        let heights = session.current_heights();
        let alpha = self.step.alpha();
        let player = self
            .player()
            .and_then(|id| world.entity(id).map(|e| (id, e)))
            .and_then(|(id, e)| e.character().map(|c| (id, e, c)));
        let Some((id, entity, character)) = player else {
            return Camera::framing(world, heights, aspect);
        };
        let pose = self.poses.at(id, alpha).unwrap_or(Pose {
            position: entity.position,
            yaw_deg: entity.rotation_y_deg,
        });
        let (yaw, pitch) = (self.controls.yaw_deg(), self.controls.pitch_deg());
        let camera = match self.view {
            ViewMode::FirstPerson => rig::first_person(&pose, character.eye_height_m, yaw, pitch),
            ViewMode::ThirdPerson => {
                rig::third_person(&pose, character.eye_height_m, yaw, pitch, heights)
            }
        };
        if camera.validate().is_ok() {
            camera
        } else {
            Camera::framing(world, heights, aspect)
        }
    }

    /// What is in flight and what just hit, for the renderer: a streak behind the head of each
    /// projectile (between its last two ticks, like the characters) and a puff for each hit of
    /// the last second.
    pub fn effects(&self) -> Effects {
        let Some(sim) = self.session().live() else {
            return Effects::default();
        };
        let ahead = self.step.alpha() * DT;
        let tracers = sim
            .projectiles()
            .iter()
            .filter_map(|p| {
                let [vx, vy, vz] = p.velocity;
                let [px, py, pz] = p.position;
                let at = |t: f64| [px + vx * t, py + vy * t, pz + vz * t];
                let head = at(ahead);
                let shown = TRACER_STREAK_S.min(p.age_s + ahead);
                let tail = at(ahead - shown);
                (head != tail).then_some(Tracer {
                    from: tail,
                    to: head,
                    age_s: p.age_s + ahead,
                })
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
        match self.view {
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
            if e.get("event").and_then(Value::as_str) != Some("hit") {
                continue;
            }
            if let (Some(point), Some(normal)) = (vec3(e.get("point")), vec3(e.get("normal"))) {
                self.hits.push((point, normal, now));
            }
        }
        self.hits.retain(|&(_, _, at)| now - at < IMPACT_LIFE_S);
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
