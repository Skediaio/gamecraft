//! A gamepad turned into the same input records as the keyboard.
//!
//! Pure mapping, no device: [`PadState`] is a snapshot of one pad (what a backend such as
//! `gilrs` reads), [`PadMapper`] turns snapshots into [`Controls`] changes and window
//! [`Action`]s. The left stick walks (with a radial dead zone), the right stick looks at a
//! speed in degrees per second, whatever the frame rate; the left stick's click runs (and holds
//! the breath while aiming), the south button jumps, west interacts, the right trigger fires,
//! the left trigger aims down the sights, north switches the fire mode, east reloads, start
//! pauses and select switches the view.

use crate::input::{Action, Controls};

/// Stick deflection below this is ignored (a stick never rests at exactly zero).
pub const DEAD_ZONE: f64 = 0.2;
/// How fast a fully deflected right stick turns the view, degrees per second.
pub const LOOK_DEG_PER_S: f64 = 150.0;
/// How far a trigger must be pulled to count as held.
pub const TRIGGER_THRESHOLD: f64 = 0.5;
/// The longest frame the look speed is integrated over, seconds: a stall must not spin the view.
pub const MAX_LOOK_STEP_S: f64 = 0.1;

/// How a pad is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadConfig {
    /// Stick deflection below this is ignored, 0 to 0.95.
    pub dead_zone: f64,
    /// The turn speed of a fully deflected right stick, degrees per second.
    pub look_deg_per_s: f64,
    /// Pushing the right stick up looks down.
    pub invert_look: bool,
    /// How far a trigger must be pulled to count as held, 0 to 1.
    pub trigger_threshold: f64,
}

impl Default for PadConfig {
    fn default() -> PadConfig {
        PadConfig {
            dead_zone: DEAD_ZONE,
            look_deg_per_s: LOOK_DEG_PER_S,
            invert_look: false,
            trigger_threshold: TRIGGER_THRESHOLD,
        }
    }
}

/// One snapshot of a pad. Sticks are -1 to 1 with right and up positive; triggers 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// Left stick, right positive.
    pub left_x: f64,
    /// Left stick, up positive.
    pub left_y: f64,
    /// Right stick, right positive.
    pub right_x: f64,
    /// Right stick, up positive.
    pub right_y: f64,
    /// Left trigger, 0 to 1.
    pub left_trigger: f64,
    /// Right trigger, 0 to 1.
    pub right_trigger: f64,
    /// The left stick is clicked.
    pub left_thumb: bool,
    /// South button (A, cross).
    pub south: bool,
    /// West button (X, square).
    pub west: bool,
    /// North button (Y, triangle).
    pub north: bool,
    /// East button (B, circle).
    pub east: bool,
    /// Start.
    pub start: bool,
    /// Select (back, view).
    pub select: bool,
}

/// A stick with the radial dead zone taken out: inside it nothing, outside it rescaled so the
/// edge of the zone is 0 and full deflection is 1, direction kept. Not finite counts as 0.
pub fn dead_zone(x: f64, y: f64, zone: f64) -> (f64, f64) {
    if !(x.is_finite() && y.is_finite()) {
        return (0.0, 0.0);
    }
    let zone = if zone.is_finite() {
        zone.clamp(0.0, 0.95)
    } else {
        DEAD_ZONE
    };
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= zone || magnitude <= 0.0 {
        return (0.0, 0.0);
    }
    let scaled = ((magnitude - zone) / (1.0 - zone)).min(1.0);
    (x / magnitude * scaled, y / magnitude * scaled)
}

/// The left stick as `(forward, right)`, each -1 to 1.
pub fn move_axes(x: f64, y: f64, config: &PadConfig) -> (f64, f64) {
    let (right, forward) = dead_zone(x, y, config.dead_zone);
    (forward, right)
}

/// The turn of the right stick over `dt_s` seconds as `(yaw_delta_deg, pitch_delta_deg)`: right
/// turns right (yaw down), up looks up (unless inverted). A gentle curve gives fine aim near
/// the dead zone; full deflection turns `look_deg_per_s`.
pub fn look_step(x: f64, y: f64, dt_s: f64, config: &PadConfig) -> (f64, f64) {
    if !dt_s.is_finite() || dt_s <= 0.0 {
        return (0.0, 0.0);
    }
    let (x, y) = dead_zone(x, y, config.dead_zone);
    // Scaling by the deflection once more (direction kept) gives fine aim for small pushes.
    let curve = (x * x + y * y).sqrt();
    let step = config.look_deg_per_s * dt_s.min(MAX_LOOK_STEP_S) * curve;
    let pitch = if config.invert_look { -y } else { y };
    (-x * step, pitch * step)
}

/// Turns pad snapshots into controls changes and window actions.
#[derive(Clone, Debug, Default)]
pub struct PadMapper {
    /// How the pad is read.
    pub config: PadConfig,
    start_was: bool,
    select_was: bool,
}

impl PadMapper {
    /// A mapper reading pads with `config`.
    pub fn new(config: PadConfig) -> PadMapper {
        PadMapper {
            config,
            ..PadMapper::default()
        }
    }

    /// Applies `state` for a frame of `dt_s` seconds to `controls`; returns the window actions
    /// (start, select) that were pressed since the last snapshot.
    pub fn update(&mut self, state: &PadState, dt_s: f64, controls: &mut Controls) -> Vec<Action> {
        let c = &self.config;
        let (forward, right) = move_axes(state.left_x, state.left_y, c);
        controls.pad_move(forward, right);
        controls.pad_buttons(
            state.left_thumb,
            state.south,
            state.west,
            state.right_trigger >= c.trigger_threshold,
        );
        controls.pad_weapon(
            state.left_trigger >= c.trigger_threshold,
            state.north,
            state.east,
        );
        let (yaw, pitch) = look_step(state.right_x, state.right_y, dt_s, c);
        controls.turn(yaw, pitch);
        let mut actions = Vec::new();
        if state.start && !self.start_was {
            actions.push(Action::Pause);
        }
        if state.select && !self.select_was {
            actions.push(Action::ToggleView);
        }
        self.start_was = state.start;
        self.select_was = state.select;
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_dead_zone_swallows_small_deflections_and_rescales_the_rest() {
        assert_eq!(dead_zone(0.1, 0.1, 0.2), (0.0, 0.0));
        assert_eq!(dead_zone(0.0, 0.0, 0.2), (0.0, 0.0));
        let (x, y) = dead_zone(1.0, 0.0, 0.2);
        assert!(close(x, 1.0) && close(y, 0.0));
        let (x, _) = dead_zone(0.6, 0.0, 0.2);
        assert!(close(x, 0.5), "{x}");
        let (x, y) = dead_zone(-0.6, 0.0, 0.2);
        assert!(close(x, -0.5) && close(y, 0.0));
        // The corner of a square stick is clamped to the circle's edge.
        let (x, y) = dead_zone(1.0, 1.0, 0.2);
        assert!(close((x * x + y * y).sqrt(), 1.0));
        assert_eq!(dead_zone(f64::NAN, 1.0, 0.2), (0.0, 0.0));
    }

    #[test]
    fn the_left_stick_gives_forward_and_right() {
        let c = PadConfig::default();
        assert_eq!(move_axes(0.0, 1.0, &c), (1.0, 0.0));
        assert_eq!(move_axes(0.0, -1.0, &c), (-1.0, 0.0));
        assert_eq!(move_axes(1.0, 0.0, &c), (0.0, 1.0));
        assert_eq!(move_axes(-1.0, 0.0, &c), (0.0, -1.0));
        assert_eq!(move_axes(0.1, 0.15, &c), (0.0, 0.0));
    }

    #[test]
    fn the_look_speed_is_per_second_not_per_frame() {
        let c = PadConfig::default();
        let (yaw, pitch) = look_step(0.0, 1.0, 1.0 / 60.0, &c);
        assert!(close(yaw, 0.0) && close(pitch, c.look_deg_per_s / 60.0));
        // One second of frames at 30 and at 120 per second turns the same.
        let turn = |fps: u32| {
            (0..fps)
                .map(|_| look_step(-1.0, 0.0, 1.0 / f64::from(fps), &c).0)
                .sum::<f64>()
        };
        assert!(close(turn(30), c.look_deg_per_s));
        assert!(close(turn(120), c.look_deg_per_s));
        // Right turns right: the yaw goes down.
        assert!(look_step(1.0, 0.0, 0.1, &c).0 < 0.0);
    }

    #[test]
    fn the_look_can_be_inverted_and_a_stall_does_not_spin_the_view() {
        let inverted = PadConfig {
            invert_look: true,
            ..PadConfig::default()
        };
        assert!(look_step(0.0, 1.0, 0.01, &inverted).1 < 0.0);
        let c = PadConfig::default();
        let (_, long) = look_step(0.0, 1.0, 5.0, &c);
        assert!(close(long, c.look_deg_per_s * MAX_LOOK_STEP_S));
        assert_eq!(look_step(0.0, 1.0, f64::NAN, &c), (0.0, 0.0));
        assert_eq!(look_step(0.0, 1.0, -1.0, &c), (0.0, 0.0));
    }

    #[test]
    fn the_buttons_map_to_the_input_record() {
        let mut mapper = PadMapper::default();
        let mut controls = Controls::new(0.0);
        let state = PadState {
            left_y: 1.0,
            left_thumb: true,
            south: true,
            west: true,
            right_trigger: 0.9,
            ..PadState::default()
        };
        mapper.update(&state, 0.016, &mut controls);
        let r = controls.record();
        assert!(close(r.forward, 1.0) && close(r.right, 0.0));
        assert!(r.run && r.jump && r.interact && r.fire);
        // The left trigger aims down the sights, and the stick's click then holds the breath;
        // north and east switch the mode and reload. Letting go of everything stops it all.
        mapper.update(
            &PadState {
                left_trigger: 1.0,
                left_thumb: true,
                north: true,
                east: true,
                ..PadState::default()
            },
            0.016,
            &mut controls,
        );
        let r = controls.record();
        assert!(r.ads && r.hold_breath && !r.run && r.forward == 0.0);
        assert!(r.switch_mode && r.reload);
        mapper.update(&PadState::default(), 0.016, &mut controls);
        controls.ticked();
        let r = controls.record();
        assert!(!r.run && !r.jump && !r.interact && !r.fire);
        assert!(!r.ads && !r.hold_breath && !r.switch_mode && !r.reload);
    }

    #[test]
    fn the_right_stick_turns_the_controls() {
        let mut mapper = PadMapper::default();
        let mut controls = Controls::new(0.0);
        let left = PadState {
            right_x: -1.0,
            ..PadState::default()
        };
        for _ in 0..60 {
            mapper.update(&left, 1.0 / 60.0, &mut controls);
        }
        assert!(close(controls.yaw_deg(), LOOK_DEG_PER_S));
        assert!(close(controls.pitch_deg(), 0.0));
        let up = PadState {
            right_y: 1.0,
            ..PadState::default()
        };
        for _ in 0..60 {
            mapper.update(&up, 1.0 / 60.0, &mut controls);
        }
        assert!(close(controls.pitch_deg(), 85.0));
    }

    #[test]
    fn start_and_select_act_on_their_press_only() {
        let mut mapper = PadMapper::default();
        let mut controls = Controls::new(0.0);
        let start = PadState {
            start: true,
            ..PadState::default()
        };
        assert_eq!(mapper.update(&start, 0.016, &mut controls), [Action::Pause]);
        assert!(mapper.update(&start, 0.016, &mut controls).is_empty());
        mapper.update(&PadState::default(), 0.016, &mut controls);
        let select = PadState {
            select: true,
            ..PadState::default()
        };
        assert_eq!(
            mapper.update(&select, 0.016, &mut controls),
            [Action::ToggleView]
        );
    }

    #[test]
    fn the_keyboard_and_the_pad_share_an_axis_and_the_last_used_wins() {
        use crate::input::Key;
        let mut mapper = PadMapper::default();
        let mut controls = Controls::new(0.0);
        let stick = PadState {
            left_x: 1.0,
            ..PadState::default()
        };
        controls.key(Key::Forward, true);
        assert_eq!(controls.record().forward, 1.0);
        mapper.update(&stick, 0.016, &mut controls);
        // The stick moved last: it owns both axes, so forward is the stick's 0 and right is 1.
        let r = controls.record();
        assert!(close(r.right, 1.0) && close(r.forward, 0.0));
        // A key pressed now takes over again.
        controls.key(Key::Back, true);
        assert_eq!(controls.record().forward, 0.0);
        controls.key(Key::Back, false);
        controls.key(Key::Forward, false);
        controls.key(Key::Forward, true);
        let r = controls.record();
        assert!(close(r.forward, 1.0) && close(r.right, 0.0));
        // Letting the stick rest does not cancel a held key.
        mapper.update(&PadState::default(), 0.016, &mut controls);
        assert_eq!(controls.record().forward, 1.0);
    }
}
