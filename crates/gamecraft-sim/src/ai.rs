//! What a behaviour knows and feels besides its state: knowledge of every enemy (detection
//! that builds up), alertness, stress from near misses, how settled its aim is, and the cover
//! it chose (`docs/design/ballistics.md`, *Skill, detection and cover*).
//!
//! All numbers here are mechanisms with named constants; how good a soldier is lives in its
//! behaviour's `skill`, `precision`, `spot_time_s`, `aim_settle_s`, `aim_error_floor_mrad`,
//! `courage`, `cover_search_m` and `calm_s`, which a pack sets.
//!
//! - **Skill** scales times: `time_factor = 1 + (1 - skill)` multiplies the reaction time,
//!   the spot time, the pauses in cover and the ticks between two cover queries.
//! - **Precision** scales the aim error only: `(1 + 3 * (1 - precision)) * (1 + 2 * stress)`.
//! - **Knowledge** of an enemy, 0 to 1, rises while it is seen at
//!   `gain / (spot_time_s * time_factor / alert)` per second, with `alert` 1 (safe), 1.5
//!   (aware) or 2 (combat) and `gain` the product of `clamp(range / 2 / distance, 0.25, 4)`
//!   (with the effective view range, so darkness, mist and rain slow it), 1.5 while the
//!   enemy moves faster than [`MOVING_SPEED_MPS`] and 2 while it holds `fire`. A spot time of
//!   0 knows an enemy at first sight. Unseen, knowledge falls by 1 in [`FORGET_S`]; the last
//!   known position stays. A behaviour reacts to a seen enemy from [`REACT_KNOWLEDGE`] and
//!   fires from [`FIRE_KNOWLEDGE`].
//! - **Stress**, 0 to 1: a projectile of someone else passing within [`NEAR_MISS_M`] of its
//!   chest in a tick adds `STRESS_PER_NEAR_MISS * (1 - courage) * (1 - distance /
//!   NEAR_MISS_M)`; it recovers at [`STRESS_RECOVERY_PER_S`]. A near miss of an enemy also
//!   makes it know the shooter at least at [`REACT_KNOWLEDGE`], where it is.
//! - **Aim settling**: while the same target stays seen, the aim error falls linearly from
//!   `aim_error_mrad` to `min(aim_error_floor_mrad, aim_error_mrad)` over `aim_settle_s`.

use std::collections::BTreeMap;

use gamecraft_math::StateHasher;
use gamecraft_world::{Alertness, Behaviour, EntityId};
use serde::Serialize;

use crate::cover::CoverPoint;
use gamecraft_world::limits::AIM_ERROR_MAX_MRAD as MAX_AIM_ERROR_MRAD;

/// Knowledge at and above which a seen enemy is reacted to.
pub const REACT_KNOWLEDGE: f64 = 0.25;
/// Knowledge at and above which it is fired at.
pub const FIRE_KNOWLEDGE: f64 = 0.6;
/// Seconds in which unseen knowledge falls from 1 to 0.
pub const FORGET_S: f64 = 60.0;
/// How much longer every time is at skill 0 than at skill 1 (as a share).
pub const SKILL_TIME_GAIN: f64 = 1.0;
/// How much larger the aim error is at precision 0 than at precision 1 (as a share).
pub const PRECISION_ERROR_GAIN: f64 = 3.0;
/// How much larger the aim error is at stress 1 than at stress 0 (as a share).
pub const STRESS_AIM_GAIN: f64 = 2.0;
/// Above this horizontal speed a seen enemy counts as moving, metres per second.
pub const MOVING_SPEED_MPS: f64 = 0.5;
/// How much faster a moving enemy is known.
pub const MOVING_SPOT_GAIN: f64 = 1.5;
/// How much faster an enemy holding `fire` is known.
pub const FIRING_SPOT_GAIN: f64 = 2.0;
/// The bounds of the distance's share of the detection speed.
pub const DISTANCE_GAIN_RANGE: (f64, f64) = (0.25, 4.0);
/// How much faster an aware behaviour knows an enemy.
pub const AWARE_SPOT_GAIN: f64 = 1.5;
/// How much faster a behaviour in combat knows an enemy.
pub const COMBAT_SPOT_GAIN: f64 = 2.0;
/// A projectile passing within this distance of the chest is a near miss, metres.
pub const NEAR_MISS_M: f64 = 3.0;
/// Stress of a near miss through the chest, without courage.
pub const STRESS_PER_NEAR_MISS: f64 = 0.35;
/// Stress lost per second.
pub const STRESS_RECOVERY_PER_S: f64 = 0.1;
/// From this stress a behaviour with `cover_search_m` is in combat and takes cover.
pub const STRESS_COVER_AT: f64 = 0.3;
/// From this stress it stays down in cover and does not peek.
pub const SUPPRESSED_AT: f64 = 0.6;
/// Ticks between two cover queries of one behaviour at skill 1.
pub const COVER_EVAL_TICKS: u32 = 30;
/// Seconds it stays down in cover before it peeks, at skill 1.
pub const HIDE_S: f64 = 2.0;
/// Seconds it peeks and fires before it goes back down.
pub const PEEK_S: f64 = 2.5;
/// Within this horizontal distance of a cover or peek point it is there, metres.
pub const COVER_ARRIVE_M: f64 = 0.35;
/// Within this horizontal distance of a corner on the way to cover it has passed it, metres.
pub const CORNER_ARRIVE_M: f64 = 0.5;
/// Seconds it may take to reach a cover before it gives the cover up.
pub const COVER_MOVE_MAX_S: f64 = 15.0;
/// How much better another cover must score before it changes cover.
pub const COVER_SWITCH_MARGIN: f64 = 0.5;
/// Most enemies one behaviour knows about; the least known are forgotten first.
pub const MAX_KNOWN: usize = 32;

/// What a behaviour knows of one enemy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Knowledge {
    /// How well it knows it, 0 to 1.
    pub knowledge: f64,
    /// Where it last knew it to be (its chest), world metres.
    pub last_known: [f64; 3],
    /// Whether it sees it now.
    pub seen: bool,
}

/// What a behaviour in cover does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum CoverPhase {
    /// Runs to the cover, giving it up after `left_s` more seconds.
    Move {
        /// Seconds left.
        left_s: f64,
    },
    /// Stays down at the cover for `left_s` more seconds.
    Hide {
        /// Seconds left.
        left_s: f64,
    },
    /// Stands at the peek point and fires for `left_s` more seconds.
    Peek {
        /// Seconds left.
        left_s: f64,
    },
}

/// The cover a behaviour chose and what it does there.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct InCover {
    /// The chosen candidate, as scored when chosen.
    pub at: CoverPoint,
    /// How many corners of the way there it has passed.
    pub via_passed: u8,
    /// What it does.
    #[serde(flatten)]
    pub phase: CoverPhase,
}

/// The share every time of a behaviour of `skill` takes (1 at skill 1, 2 at skill 0).
pub fn time_factor(skill: f64) -> f64 {
    let skill = if skill.is_finite() {
        skill.clamp(0.0, 1.0)
    } else {
        1.0
    };
    1.0 + SKILL_TIME_GAIN * (1.0 - skill)
}

/// The aim error, milliradians, of `b` after `settled_s` seconds on its target under
/// `stress` (see the module docs); at most the largest aim error a behaviour may have.
pub fn aim_error_mrad(b: &Behaviour, settled_s: f64, stress: f64) -> f64 {
    let start = b.aim_error_mrad;
    let floor = b.aim_error_floor_mrad.min(start);
    let settled = if b.aim_settle_s > 0.0 {
        let share = (settled_s / b.aim_settle_s).clamp(0.0, 1.0);
        let share = if share.is_finite() { share } else { 0.0 };
        start - (start - floor) * share
    } else {
        start
    };
    let precision = if b.precision.is_finite() {
        b.precision.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let stress = if stress.is_finite() {
        stress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let e = settled
        * (1.0 + PRECISION_ERROR_GAIN * (1.0 - precision))
        * (1.0 + STRESS_AIM_GAIN * stress);
    if e.is_finite() {
        e.clamp(0.0, MAX_AIM_ERROR_MRAD)
    } else {
        start
    }
}

/// How much faster an alertness knows an enemy.
pub fn alert_gain(a: Alertness) -> f64 {
    match a {
        Alertness::Safe => 1.0,
        Alertness::Aware => AWARE_SPOT_GAIN,
        Alertness::Combat => COMBAT_SPOT_GAIN,
    }
}

/// The order of alertness: safe, aware, combat.
pub(crate) fn rank(a: Alertness) -> u8 {
    match a {
        Alertness::Safe => 0,
        Alertness::Aware => 1,
        Alertness::Combat => 2,
    }
}

/// One step calmer, but not below `floor`.
pub(crate) fn calmer(a: Alertness, floor: Alertness) -> Alertness {
    let down = match a {
        Alertness::Combat => Alertness::Aware,
        Alertness::Aware | Alertness::Safe => Alertness::Safe,
    };
    if rank(down) < rank(floor) {
        floor
    } else {
        down
    }
}

/// The larger of two alertnesses.
pub(crate) fn raised(a: Alertness, to: Alertness) -> Alertness {
    if rank(to) > rank(a) { to } else { a }
}

/// The distance from `p` to the segment from `a` to `b`, metres.
pub(crate) fn segment_distance(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if len2 > 0.0 && len2.is_finite() {
        ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let t = if t.is_finite() { t } else { 0.0 };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
    let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Keeps at most [`MAX_KNOWN`] entries, forgetting the least known (the highest id on a tie).
pub(crate) fn cap_known(known: &mut BTreeMap<EntityId, Knowledge>) {
    while known.len() > MAX_KNOWN {
        let weakest = known
            .iter()
            .min_by(|(ia, a), (ib, b)| a.knowledge.total_cmp(&b.knowledge).then(ib.cmp(ia)))
            .map(|(id, _)| *id);
        match weakest {
            Some(id) => {
                known.remove(&id);
            }
            None => break,
        }
    }
}

fn write_point(hasher: &mut StateHasher, p: Option<[f64; 3]>) {
    match p {
        Some(p) => {
            hasher.write_bool(true);
            for v in p {
                hasher.write_f64(v);
            }
        }
        None => hasher.write_bool(false),
    }
}

/// Writes what a brain knows and feels (everything of this module) into the state hash.
pub(crate) fn hash_brain(brain: &crate::Brain, hasher: &mut StateHasher) {
    hasher.write_str(brain.alertness.name());
    hasher.write_f64(brain.calm_left_s);
    hasher.write_f64(brain.stress);
    hasher.write_u64(brain.aim_target.map_or(0, |t| t.0));
    hasher.write_f64(brain.aim_settled_s);
    hasher.write_u64(u64::from(brain.cover_eval_in));
    hasher.write_u64(brain.knowledge.len() as u64);
    for (id, k) in &brain.knowledge {
        hasher.write_u64(id.0);
        hasher.write_f64(k.knowledge);
        write_point(hasher, Some(k.last_known));
        hasher.write_bool(k.seen);
    }
    match &brain.cover {
        Some(c) => {
            hasher.write_bool(true);
            write_point(hasher, Some(c.at.point));
            for via in c.at.via {
                write_point(hasher, via);
            }
            write_point(hasher, c.at.peek);
            hasher.write_f64(c.at.score);
            hasher.write_u64(u64::from(c.via_passed));
            let (phase, left) = match c.phase {
                CoverPhase::Move { left_s } => (0, left_s),
                CoverPhase::Hide { left_s } => (1, left_s),
                CoverPhase::Peek { left_s } => (2, left_s),
            };
            hasher.write_u64(phase);
            hasher.write_f64(left);
        }
        None => hasher.write_bool(false),
    }
}
