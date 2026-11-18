//! Scenario events: characters entering and leaving zones, interacting with interactables, and
//! simulated time reaching a `time` trigger's `at_s`. Pure functions of the world after a
//! tick, so every platform detects the same events in the same order.

use std::collections::BTreeSet;

use gamecraft_math::{Portable, deg_to_rad, yaw_forward};
use gamecraft_world::{Entity, EntityId, When, World, ZoneShape};
use serde::Serialize;

/// Cosine of the widest angle, 45 degrees, between a character's facing and the direction to
/// something it interacts with.
const FACING_COS: f64 = core::f64::consts::FRAC_1_SQRT_2;

/// Below this horizontal distance, metres, a target counts as faced whatever the facing.
const MIN_FACING_DISTANCE_M: f64 = 1e-9;

/// What happened during a tick.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    /// The character `who` came inside the zone of entity `zone`.
    Enter {
        /// The character.
        who: EntityId,
        /// The entity with the `zone` component.
        zone: EntityId,
    },
    /// The character `who` is no longer inside the zone of entity `zone`.
    Leave {
        /// The character.
        who: EntityId,
        /// The entity with the `zone` component.
        zone: EntityId,
    },
    /// The character `who` interacted with entity `target`.
    Interact {
        /// The character.
        who: EntityId,
        /// The entity with the `interactable` component.
        target: EntityId,
    },
    /// Simulated time reached `at_s`, the moment of an enabled `time` trigger.
    Time {
        /// Seconds since the start.
        at_s: f64,
    },
}

/// An event and the tick it happened in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    /// The tick (1 for the first step).
    pub tick: u64,
    /// What happened.
    #[serde(flatten)]
    pub kind: EventKind,
}

impl EventKind {
    /// The event's name, as a trigger's `when.event` names it.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::Enter { .. } => "enter",
            EventKind::Leave { .. } => "leave",
            EventKind::Interact { .. } => "interact",
            EventKind::Time { .. } => "time",
        }
    }

    /// The entity that caused the event (`$who`), `None` for time.
    pub fn who(&self) -> Option<EntityId> {
        match self {
            EventKind::Enter { who, .. }
            | EventKind::Leave { who, .. }
            | EventKind::Interact { who, .. } => Some(*who),
            EventKind::Time { .. } => None,
        }
    }

    /// The entity the event is about (`$target`): the zone or the interacted entity.
    pub fn target(&self) -> Option<EntityId> {
        match self {
            EventKind::Enter { zone, .. } | EventKind::Leave { zone, .. } => Some(*zone),
            EventKind::Interact { target, .. } => Some(*target),
            EventKind::Time { .. } => None,
        }
    }
}

/// The scenario parts of a world the detection needs, derived when the world changes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Watch {
    /// Entities with a `zone`, in id order.
    pub(crate) zones: Vec<EntityId>,
    /// Entities with an `interactable`, in id order.
    pub(crate) interactables: Vec<EntityId>,
    /// The `at_s` of every enabled `time` trigger, ascending, each once.
    pub(crate) times: Vec<f64>,
}

impl Watch {
    pub(crate) fn of(world: &World) -> Watch {
        let zones = world
            .entities()
            .filter(|e| e.zone().is_some())
            .map(|e| e.id)
            .collect();
        let interactables = world
            .entities()
            .filter(|e| e.interactable().is_some())
            .map(|e| e.id)
            .collect();
        let mut times: Vec<f64> = world
            .triggers
            .iter()
            .filter(|t| t.enabled)
            .filter_map(|t| match t.when {
                When::Time { at_s } if at_s.is_finite() => Some(at_s),
                _ => None,
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        Watch {
            zones,
            interactables,
            times,
        }
    }

    /// The time events of tick `tick` (counted from 1): every `at_s` in
    /// `((tick - 1) * DT, tick * DT]`, and at or before 0 on the first tick.
    pub(crate) fn times_of(&self, tick: u64, dt: f64) -> impl Iterator<Item = f64> + '_ {
        let now = tick as f64 * dt;
        let before = tick.saturating_sub(1) as f64 * dt;
        let first = tick <= 1;
        self.times
            .iter()
            .copied()
            .filter(move |&t| t <= now && (first || t > before))
    }
}

/// Whether the character `who` (feet at its position, `height_m` tall) is inside the zone of
/// `zone`.
pub(crate) fn inside(who: &Entity, height_m: f64, zone: &Entity) -> bool {
    let Some(z) = zone.zone() else {
        return false;
    };
    let s = zone.scale;
    let [px, py, pz] = zone.position;
    let [x, y, cz] = who.position;
    let top = py + z.height_m * s;
    if !(y <= top && y + height_m >= py) {
        return false;
    }
    let (dx, dz) = (x - px, cz - pz);
    match z.shape {
        ZoneShape::Circle => {
            let r = z.radius_m * s;
            dx * dx + dz * dz <= r * r
        }
        ZoneShape::Box => {
            // World to local: the inverse of the entity's turn, counter-clockwise from above.
            let (sin, cos) = deg_to_rad(zone.rotation_y_deg).psin_cos();
            let lx = dx * cos - dz * sin;
            let lz = dx * sin + dz * cos;
            let [sx, sz] = z.size;
            lx.abs() <= sx * s / 2.0 && lz.abs() <= sz * s / 2.0
        }
    }
}

/// The interactable `who` (a character with eyes `eye_height_m` above its feet) reaches and
/// faces, the nearest by distance from the eye (the lowest id on a tie); `None` when there is
/// none.
///
/// The distance is from the eye to the nearest point of the target's upright extent: its
/// position up to the top of its shape (`size[1] * scale`), or its position alone without a
/// shape. The target is faced when the horizontal direction to it is within 45 degrees of the
/// character's facing, or when the character stands right over it.
pub(crate) fn reached(
    world: &World,
    who: &Entity,
    eye_height_m: f64,
    candidates: &[EntityId],
) -> Option<EntityId> {
    let [x, y, z] = who.position;
    let eye = y + eye_height_m;
    let forward = yaw_forward(who.rotation_y_deg);
    let mut best: Option<(f64, EntityId)> = None;
    for &id in candidates {
        if id == who.id {
            continue;
        }
        let Some(target) = world.entity(id) else {
            continue;
        };
        let Some(interactable) = target.interactable() else {
            continue;
        };
        let [tx, ty, tz] = target.position;
        let top = ty + target.shape().map_or(0.0, |s| s.size[1] * target.scale);
        let nearest_y = eye.clamp(ty.min(top), ty.max(top));
        let (dx, dy, dz) = (tx - x, nearest_y - eye, tz - z);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let distance = (horizontal * horizontal + dy * dy).sqrt();
        if !(distance.is_finite() && distance <= interactable.reach_m) {
            continue;
        }
        if horizontal > MIN_FACING_DISTANCE_M {
            let cos = (forward.x * dx + forward.z * dz) / horizontal;
            if cos < FACING_COS {
                continue;
            }
        }
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Every (character, zone) pair whose character is inside the zone, in id order.
pub(crate) fn memberships(
    world: &World,
    characters: &[EntityId],
    zones: &[EntityId],
) -> BTreeSet<(EntityId, EntityId)> {
    let mut out = BTreeSet::new();
    for &c in characters {
        let Some(who) = world.entity(c) else {
            continue;
        };
        let Some(character) = who.character() else {
            continue;
        };
        for &z in zones {
            if z == c {
                continue;
            }
            if let Some(zone) = world.entity(z)
                && inside(who, character.height_m, zone)
            {
                out.insert((c, z));
            }
        }
    }
    out
}
