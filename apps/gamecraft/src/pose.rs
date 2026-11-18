//! Where the characters stand at the last two ticks, and the picture in between: the display
//! is interpolated, the simulation is not touched.

use gamecraft_math::{wrap_deg, wrap_deg_signed};
use gamecraft_world::{EntityId, World};
use std::collections::BTreeMap;

/// Above this distance between two ticks, metres, a character jumped (a game master moved
/// it) and is shown where it is now instead of sliding there.
pub const SNAP_DISTANCE_M: f64 = 5.0;

/// A character's position (its base) and facing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The base, metres.
    pub position: [f64; 3],
    /// The facing, degrees counter-clockwise from north.
    pub yaw_deg: f64,
}

/// The pose of every character of `world`, by id.
pub fn characters(world: &World) -> BTreeMap<EntityId, Pose> {
    world
        .entities()
        .filter(|e| e.character().is_some())
        .map(|e| {
            (
                e.id,
                Pose {
                    position: e.position,
                    yaw_deg: e.rotation_y_deg,
                },
            )
        })
        .collect()
}

/// The pose `alpha` (0 to 1, clamped; NaN counts as 1) of the way from `from` to `to`: the
/// position along the line, the facing the short way round. A step longer than
/// [`SNAP_DISTANCE_M`] or a non-finite pose gives `to`.
pub fn interpolate(from: &Pose, to: &Pose, alpha: f64) -> Pose {
    let alpha = if alpha.is_nan() {
        1.0
    } else {
        alpha.clamp(0.0, 1.0)
    };
    let [a, b] = [from.position, to.position];
    let distance = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
    let finite = a.iter().chain(&b).all(|v| v.is_finite())
        && from.yaw_deg.is_finite()
        && to.yaw_deg.is_finite();
    if !finite || distance > SNAP_DISTANCE_M {
        return *to;
    }
    let lerp = |p: f64, q: f64| p + (q - p) * alpha;
    let turn = wrap_deg_signed(to.yaw_deg - from.yaw_deg);
    Pose {
        position: [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])],
        yaw_deg: wrap_deg(from.yaw_deg + turn * alpha),
    }
}

/// The characters' poses at the tick before the last and at the last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poses {
    before: BTreeMap<EntityId, Pose>,
    last: BTreeMap<EntityId, Pose>,
}

impl Poses {
    /// Both ticks as `world` stands now.
    pub fn new(world: &World) -> Poses {
        let now = characters(world);
        Poses {
            before: now.clone(),
            last: now,
        }
    }

    /// A tick ran: the last becomes the one before, `world` the last.
    pub fn ticked(&mut self, world: &World) {
        self.before = std::mem::replace(&mut self.last, characters(world));
    }

    /// No tick ran, but `world` may have changed by a command: a character that is not where
    /// the last tick left it is shown where it is now, without sliding.
    pub fn observe(&mut self, world: &World) {
        let now = characters(world);
        if now != self.last {
            *self = Poses {
                before: now.clone(),
                last: now,
            };
        }
    }

    /// Character `id`'s pose `alpha` of the way from the tick before to the last.
    pub fn at(&self, id: EntityId, alpha: f64) -> Option<Pose> {
        let last = self.last.get(&id)?;
        Some(match self.before.get(&id) {
            Some(before) => interpolate(before, last, alpha),
            None => *last,
        })
    }

    /// Puts every character of `world` at its interpolated pose (for display only).
    pub fn apply(&self, world: &mut World, alpha: f64) {
        for &id in self.last.keys() {
            let Some(pose) = self.at(id, alpha) else {
                continue;
            };
            if let Some(e) = world.entity_mut(id) {
                e.position = pose.position;
                e.rotation_y_deg = pose.yaw_deg;
            }
        }
    }
}
