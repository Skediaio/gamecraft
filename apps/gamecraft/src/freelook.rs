//! Free look: while it is held (Left Alt), the mouse and the gamepad's right stick turn only
//! the head, the camera; the character's facing and its aim (the input record's `yaw_deg` and
//! `pitch_deg`, which drive walking and firing) stay where they were. The head turns at most
//! [`HEAD_YAW_LIMIT_DEG`] to either side of the aim and tilts within the usual pitch limits;
//! when it is let go the camera is back on the aim at once.

use crate::input::PITCH_LIMIT_DEG;
use gamecraft_math::wrap_deg;

/// How far the head turns from the aim at most, to either side, degrees.
pub const HEAD_YAW_LIMIT_DEG: f64 = 100.0;

/// The head's turn away from the aim while free look is held.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Freelook {
    held: bool,
    /// The head's yaw from the aim's, degrees, counter-clockwise positive, within the limit.
    yaw_deg: f64,
    /// The head's pitch from the aim's, degrees, up positive.
    pitch_deg: f64,
}

impl Freelook {
    /// Free look is held (`true`) or let go. Either way the head starts on (or returns to)
    /// the aim.
    pub fn hold(&mut self, held: bool) {
        *self = Freelook {
            held,
            ..Freelook::default()
        };
    }

    /// Whether free look is held.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Turns the head by `dyaw_deg` (counter-clockwise positive) and `dpitch_deg` (up
    /// positive) while held; `aim_pitch_deg` is the aim's tilt, so the head's own tilt stays
    /// within the pitch limit. Its yaw stays within [`HEAD_YAW_LIMIT_DEG`] of the aim. A
    /// number that is not finite is ignored; nothing happens when not held.
    pub fn turn(&mut self, dyaw_deg: f64, dpitch_deg: f64, aim_pitch_deg: f64) {
        if !self.held {
            return;
        }
        if dyaw_deg.is_finite() {
            self.yaw_deg = (self.yaw_deg + dyaw_deg).clamp(-HEAD_YAW_LIMIT_DEG, HEAD_YAW_LIMIT_DEG);
        }
        if dpitch_deg.is_finite() && aim_pitch_deg.is_finite() {
            let aim = aim_pitch_deg.clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG);
            self.pitch_deg =
                (self.pitch_deg + dpitch_deg).clamp(-PITCH_LIMIT_DEG - aim, PITCH_LIMIT_DEG - aim);
        }
    }

    /// Where the head looks for an aim of `aim_yaw_deg` and `aim_pitch_deg`: the aim turned by
    /// the head's turn while held, the aim itself otherwise. The yaw is 0 to 360.
    pub fn view(&self, aim_yaw_deg: f64, aim_pitch_deg: f64) -> (f64, f64) {
        if !self.held {
            return (aim_yaw_deg, aim_pitch_deg);
        }
        (
            wrap_deg(aim_yaw_deg + self.yaw_deg),
            (aim_pitch_deg + self.pitch_deg).clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_head_turns_only_while_held_and_returns_on_release() {
        let mut f = Freelook::default();
        f.turn(30.0, 10.0, 0.0);
        assert_eq!(
            f.view(40.0, 5.0),
            (40.0, 5.0),
            "not held: the view is the aim"
        );
        f.hold(true);
        f.turn(30.0, 10.0, 5.0);
        let (yaw, pitch) = f.view(40.0, 5.0);
        assert!(close(yaw, 70.0) && close(pitch, 15.0), "{yaw} {pitch}");
        f.hold(false);
        assert_eq!(f.view(40.0, 5.0), (40.0, 5.0), "back on the aim at once");
        f.hold(true);
        assert_eq!(
            f.view(40.0, 5.0),
            (40.0, 5.0),
            "a new hold starts on the aim"
        );
    }

    #[test]
    fn the_head_turns_at_most_100_degrees_and_tilts_within_the_pitch_limit() {
        let mut f = Freelook::default();
        f.hold(true);
        for _ in 0..50 {
            f.turn(10.0, 10.0, 20.0);
        }
        let (yaw, pitch) = f.view(350.0, 20.0);
        assert!(close(yaw, 90.0), "350 + 100 wraps to 90: {yaw}");
        assert!(close(pitch, PITCH_LIMIT_DEG), "{pitch}");
        for _ in 0..50 {
            f.turn(-10.0, -10.0, 20.0);
        }
        let (yaw, pitch) = f.view(350.0, 20.0);
        assert!(close(yaw, 250.0), "350 - 100: {yaw}");
        assert!(close(pitch, -PITCH_LIMIT_DEG), "{pitch}");
        f.turn(f64::NAN, f64::INFINITY, 0.0);
        assert_eq!(
            f.view(350.0, 20.0),
            (yaw, pitch),
            "non-finite turns are ignored"
        );
    }
}
