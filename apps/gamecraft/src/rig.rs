//! The cameras that follow the player: from its eyes, or behind and above it on a boom that is
//! pulled in when the terrain is in the way.

use crate::pose::Pose;
use gamecraft_math::{DVec3, deg_to_rad, yaw_forward};
use gamecraft_render::Camera;
use gamecraft_world::{HeightField, ViewRules};

/// Vertical field of view of both views unless the world or the player sets one, degrees.
pub const FOV_DEG: f64 = gamecraft_world::DEFAULT_FOV_Y_DEG;
/// How far ahead of the eyes the first-person camera looks, metres.
const LOOK_AHEAD_M: f64 = 10.0;
/// The third-person boom's full length from the point looked at, metres.
pub const BOOM_M: f64 = 4.0;
/// The shortest the boom is pulled in to, metres.
pub const BOOM_MIN_M: f64 = 0.4;
/// The boom's angle above the horizontal when the mouse looks level, degrees.
const BOOM_ELEVATION_DEG: f64 = 15.0;
/// The lowest and highest the boom swings, degrees above the horizontal.
const BOOM_ELEVATION_MIN_DEG: f64 = -40.0;
const BOOM_ELEVATION_MAX_DEG: f64 = 80.0;
/// How far above the ground every point of the boom stays, metres.
const CLEARANCE_M: f64 = 0.3;
/// The steps the boom is tested in, metres.
const BOOM_STEP_M: f64 = 0.1;

/// Which camera follows the player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// From the character's eyes.
    FirstPerson,
    /// Behind and above the character.
    #[default]
    ThirdPerson,
}

impl ViewMode {
    /// The other view.
    pub fn toggled(self) -> ViewMode {
        match self {
            ViewMode::FirstPerson => ViewMode::ThirdPerson,
            ViewMode::ThirdPerson => ViewMode::FirstPerson,
        }
    }

    /// The camera of a world's view mode.
    pub fn of(mode: gamecraft_world::ViewMode) -> ViewMode {
        match mode {
            gamecraft_world::ViewMode::First => ViewMode::FirstPerson,
            gamecraft_world::ViewMode::Third => ViewMode::ThirdPerson,
        }
    }

    /// The first view the world's `rules` allow (the one a window starts in); third person
    /// for rules that allow none (an invalid world).
    pub fn first_allowed(rules: &ViewRules) -> ViewMode {
        rules
            .modes
            .first()
            .map_or(ViewMode::ThirdPerson, |m| ViewMode::of(*m))
    }

    /// This view if `rules` allow it, else the first they allow (the rules changed while
    /// playing).
    pub fn allowed(self, rules: &ViewRules) -> ViewMode {
        if rules.modes.iter().any(|m| ViewMode::of(*m) == self) {
            self
        } else {
            ViewMode::first_allowed(rules)
        }
    }

    /// The view after this one among those `rules` allow, in their order, round again after
    /// the last; with only one allowed, that one.
    pub fn next_allowed(self, rules: &ViewRules) -> ViewMode {
        let modes: Vec<ViewMode> = rules.modes.iter().map(|m| ViewMode::of(*m)).collect();
        match modes.iter().position(|m| *m == self) {
            Some(i) => modes
                .get(i + 1)
                .or_else(|| modes.first())
                .copied()
                .unwrap_or(self),
            None => ViewMode::first_allowed(rules),
        }
    }
}

/// The unit direction of a view turned `yaw_deg` and tilted `pitch_deg` up.
fn direction(yaw_deg: f64, pitch_deg: f64) -> DVec3 {
    let pitch = deg_to_rad(pitch_deg);
    yaw_forward(yaw_deg) * pitch.cos() + DVec3::Y * pitch.sin()
}

/// From the eyes of a character standing at `pose`, `eye_height_m` over its base, looking
/// along `yaw_deg` and `pitch_deg` (the mouse's, not the pose's: the view does not lag a tick).
pub fn first_person(pose: &Pose, eye_height_m: f64, yaw_deg: f64, pitch_deg: f64) -> Camera {
    let eye = DVec3::from_array(pose.position) + DVec3::Y * eye_height_m;
    Camera {
        position: eye.to_array(),
        target: (eye + direction(yaw_deg, pitch_deg) * LOOK_AHEAD_M).to_array(),
        fov_y_deg: FOV_DEG,
    }
}

/// Behind and above a character at `pose`, looking at its eyes: a boom of [`BOOM_M`] at 15
/// degrees over the horizontal, swung down as the mouse looks up and up as it looks down,
/// pulled in where the terrain (`heights`) would come between the eyes and the camera.
pub fn third_person(
    pose: &Pose,
    eye_height_m: f64,
    yaw_deg: f64,
    pitch_deg: f64,
    heights: Option<&HeightField>,
) -> Camera {
    let ground = |x: f64, z: f64| heights.and_then(|h| h.height_at(x, z));
    third_person_over(pose, eye_height_m, yaw_deg, pitch_deg, &ground)
}

/// [`third_person`] over any ground: `ground(x, z)` is the height there, `None` where there
/// is none.
pub fn third_person_over(
    pose: &Pose,
    eye_height_m: f64,
    yaw_deg: f64,
    pitch_deg: f64,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
) -> Camera {
    let focus = DVec3::from_array(pose.position) + DVec3::Y * eye_height_m;
    let elevation =
        (BOOM_ELEVATION_DEG - pitch_deg).clamp(BOOM_ELEVATION_MIN_DEG, BOOM_ELEVATION_MAX_DEG);
    // Back along the facing, raised by the elevation.
    let back = -direction(yaw_deg, -elevation);
    let length = boom_length(focus, back, ground);
    Camera {
        position: (focus + back * length).to_array(),
        target: focus.to_array(),
        fov_y_deg: FOV_DEG,
    }
}

/// How long the boom from `focus` along the unit `back` can be, [`BOOM_MIN_M`] to [`BOOM_M`]:
/// up to the last step whose point stays [`CLEARANCE_M`] over the ground.
fn boom_length(focus: DVec3, back: DVec3, ground: &dyn Fn(f64, f64) -> Option<f64>) -> f64 {
    let steps = (BOOM_M / BOOM_STEP_M).ceil() as u32;
    let mut clear = 0.0;
    for k in 1..=steps {
        let distance = (f64::from(k) * BOOM_STEP_M).min(BOOM_M);
        let p = focus + back * distance;
        if ground(p.x, p.z).is_some_and(|g| p.y < g + CLEARANCE_M) {
            break;
        }
        clear = distance;
    }
    clear.max(BOOM_MIN_M)
}
