//! Keys and the mouse turned into an input record (`sim.input`), and the window's own keys.
//!
//! W and S walk forward and back, A and D strafe, Shift runs, Space jumps, E interacts; the
//! mouse turns the character (the record's `yaw_deg`) and tilts the camera; the left button
//! fires the character's launcher along that tilt (the record's `fire` and `pitch_deg`). F1 toggles first and third person, F5 restarts the simulation, P pauses
//! and Esc releases the pointer: those are [`Action`]s for the window, not input.

use gamecraft_engine::Session;
use gamecraft_math::wrap_deg;
use gamecraft_world::EntityId;
use serde_json::{Value, json};

/// How far the view turns per pixel of mouse movement, degrees.
pub const LOOK_DEG_PER_PIXEL: f64 = 0.12;
/// How far the camera tilts up or down at most, degrees.
pub const PITCH_LIMIT_DEG: f64 = 85.0;
/// The largest mouse movement taken from one event, pixels: a browser can report a wild jump
/// when the pointer locks, which must not spin the view.
pub const MAX_LOOK_PIXELS: f64 = 400.0;

/// A key the window knows, whatever device it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// W: walk forward.
    Forward,
    /// S: walk back.
    Back,
    /// A: strafe left.
    Left,
    /// D: strafe right.
    Right,
    /// Shift: run.
    Run,
    /// Space: jump.
    Jump,
    /// E: interact.
    Interact,
    /// F1: first or third person.
    ToggleView,
    /// F5: restart the simulation.
    Restart,
    /// P: pause.
    Pause,
    /// Esc: release the pointer.
    Release,
}

/// What a window key asks of the window, on its press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Switch between first and third person.
    ToggleView,
    /// Stop and start the simulation again.
    Restart,
    /// Pause or resume the simulation.
    Pause,
    /// Release the pointer lock.
    Release,
}

/// What a character is asked to do: the fields of `sim.input`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputRecord {
    /// -1 (back) to 1 (forward).
    pub forward: f64,
    /// -1 (left) to 1 (right).
    pub right: f64,
    /// The facing, degrees counter-clockwise from north seen from above.
    pub yaw_deg: f64,
    /// Run instead of walk.
    pub run: bool,
    /// Jump whenever on the ground.
    pub jump: bool,
    /// Interact once when it turns true.
    pub interact: bool,
    /// The aim's tilt, degrees up from level (the camera's pitch), for a launcher.
    pub pitch_deg: f64,
    /// Fire the launcher while held.
    pub fire: bool,
}

impl InputRecord {
    /// The parameters of `sim.input` for `entity`.
    pub fn params(&self, entity: EntityId) -> Value {
        json!({
            "entity": entity.0,
            "forward": self.forward,
            "right": self.right,
            "yaw_deg": self.yaw_deg,
            "run": self.run,
            "jump": self.jump,
            "interact": self.interact,
            "pitch_deg": self.pitch_deg,
            "fire": self.fire,
        })
    }

    /// Stores this record for `entity` through the session (`sim.input`).
    pub fn send(&self, session: &mut Session, entity: EntityId) -> Result<Value, String> {
        session
            .run("sim.input", &self.params(entity))
            .map_err(|e| e.to_string())
    }
}

/// The keys held, the taps not yet seen by a tick, and where the mouse points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Controls {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    run: bool,
    jump: bool,
    interact: bool,
    /// Jump and interact pressed since the last tick: a tap shorter than a tick still
    /// reaches one.
    jump_tapped: bool,
    interact_tapped: bool,
    /// The left mouse button is held.
    fire: bool,
    yaw_deg: f64,
    pitch_deg: f64,
    /// The gamepad's left stick, and whether it moved after the walking keys last changed
    /// (the last device used owns the walking axes).
    pad_forward: f64,
    pad_right: f64,
    pad_owns_walk: bool,
    /// The gamepad's buttons, kept apart from the keys' so neither lets go of the other's.
    pad_run: bool,
    pad_jump: bool,
    pad_interact: bool,
    pad_fire: bool,
}

impl Controls {
    /// No keys held, facing `yaw_deg`, looking level.
    pub fn new(yaw_deg: f64) -> Controls {
        Controls {
            yaw_deg: if yaw_deg.is_finite() {
                wrap_deg(yaw_deg)
            } else {
                0.0
            },
            ..Controls::default()
        }
    }

    /// A key went down (`pressed`) or up. Returns the window's action for a window key's press.
    pub fn key(&mut self, key: Key, pressed: bool) -> Option<Action> {
        if pressed && matches!(key, Key::Forward | Key::Back | Key::Left | Key::Right) {
            self.pad_owns_walk = false;
        }
        let held = match key {
            Key::Forward => &mut self.forward,
            Key::Back => &mut self.back,
            Key::Left => &mut self.left,
            Key::Right => &mut self.right,
            Key::Run => &mut self.run,
            Key::Jump => {
                self.jump_tapped |= pressed;
                &mut self.jump
            }
            Key::Interact => {
                self.interact_tapped |= pressed;
                &mut self.interact
            }
            Key::ToggleView => return pressed.then_some(Action::ToggleView),
            Key::Restart => return pressed.then_some(Action::Restart),
            Key::Pause => return pressed.then_some(Action::Pause),
            Key::Release => return pressed.then_some(Action::Release),
        };
        *held = pressed;
        None
    }

    /// The mouse moved by `dx`, `dy` pixels (right and down positive): right turns right
    /// (clockwise seen from above), up looks up. The pitch stays within 85 degrees.
    pub fn look(&mut self, dx: f64, dy: f64) {
        let clean = |v: f64| {
            if v.is_finite() {
                v.clamp(-MAX_LOOK_PIXELS, MAX_LOOK_PIXELS)
            } else {
                0.0
            }
        };
        self.yaw_deg = wrap_deg(self.yaw_deg - clean(dx) * LOOK_DEG_PER_PIXEL);
        self.pitch_deg = (self.pitch_deg - clean(dy) * LOOK_DEG_PER_PIXEL)
            .clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG);
    }

    /// The fire button (left mouse) went down or up.
    pub fn set_fire(&mut self, held: bool) {
        self.fire = held;
    }

    /// The gamepad's left stick, already past its dead zone: `forward` and `right`, -1 to 1.
    /// A stick off centre owns the walking axes until a walking key is pressed; at rest it
    /// leaves them to the keys.
    pub fn pad_move(&mut self, forward: f64, right: f64) {
        let clean = |v: f64| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        self.pad_forward = clean(forward);
        self.pad_right = clean(right);
        self.pad_owns_walk = self.pad_forward != 0.0 || self.pad_right != 0.0;
    }

    /// The gamepad's buttons as they are held now. A jump or interact that just went down is a
    /// tap, like a key's.
    pub fn pad_buttons(&mut self, run: bool, jump: bool, interact: bool, fire: bool) {
        self.jump_tapped |= jump && !self.pad_jump;
        self.interact_tapped |= interact && !self.pad_interact;
        self.pad_run = run;
        self.pad_jump = jump;
        self.pad_interact = interact;
        self.pad_fire = fire;
    }

    /// Turns the view by `dyaw_deg` (counter-clockwise positive) and tilts it by `dpitch_deg`
    /// (up positive); the pitch stays within 85 degrees.
    pub fn turn(&mut self, dyaw_deg: f64, dpitch_deg: f64) {
        if dyaw_deg.is_finite() {
            self.yaw_deg = wrap_deg(self.yaw_deg + dyaw_deg);
        }
        if dpitch_deg.is_finite() {
            self.pitch_deg = (self.pitch_deg + dpitch_deg).clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG);
        }
    }

    /// Faces `yaw_deg` (a restart puts the character back as it stood).
    pub fn face(&mut self, yaw_deg: f64) {
        if yaw_deg.is_finite() {
            self.yaw_deg = wrap_deg(yaw_deg);
        }
    }

    /// The facing, degrees, 0 to 360.
    pub fn yaw_deg(&self) -> f64 {
        self.yaw_deg
    }

    /// The camera's tilt, degrees, up positive.
    pub fn pitch_deg(&self) -> f64 {
        self.pitch_deg
    }

    /// The input record of the keys held and tapped and the facing.
    pub fn record(&self) -> InputRecord {
        let axis = |plus: bool, minus: bool| f64::from(u8::from(plus)) - f64::from(u8::from(minus));
        let (forward, right) = if self.pad_owns_walk {
            (self.pad_forward, self.pad_right)
        } else {
            (axis(self.forward, self.back), axis(self.right, self.left))
        };
        InputRecord {
            forward,
            right,
            yaw_deg: self.yaw_deg,
            run: self.run || self.pad_run,
            jump: self.jump || self.pad_jump || self.jump_tapped,
            interact: self.interact || self.pad_interact || self.interact_tapped,
            pitch_deg: self.pitch_deg,
            fire: self.fire || self.pad_fire,
        }
    }

    /// A tick ran with the current record: taps have been seen.
    pub fn ticked(&mut self) {
        self.jump_tapped = false;
        self.interact_tapped = false;
    }

    /// Lets go of every key (the window lost the focus and will not hear them come up).
    pub fn release_all(&mut self) {
        *self = Controls {
            yaw_deg: self.yaw_deg,
            pitch_deg: self.pitch_deg,
            ..Controls::default()
        };
    }
}
