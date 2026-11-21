//! Cover: where a character can stand so that the threats it knows of cannot see its chest,
//! and from where it can step out to fire (`query.cover`; a behaviour with `cover_search_m`).
//!
//! The query is a position picker, deterministic and on a budget:
//!
//! 1. **Candidates**, at most [`MAX_COVER_CANDIDATES`], in this order: where it stands; eight
//!    points [`COVER_CLEARANCE_M`] beyond its radius around every solid shape and solid model
//!    near it (a box's four sides and four corners, eight points around a round solid), in the
//!    solids' id order; and two rings of [`RING_SAMPLES`] points at half and all of the search
//!    radius (they find the relief of the terrain). Only points within the search radius
//!    whose circle overlaps no solid count. A point stands on the terrain (or at the
//!    character's height without one).
//! 2. **Protection**: the share of the threats (at most [`MAX_COVER_THREATS`]) whose line of
//!    sight to the point's chest height ([`CHEST_SHARE`] of the character's height) is
//!    blocked, by the terrain or any shape or model ([`ray::line_of_sight`]).
//! 3. **A first score**: `2 * protection - 0.5 * distance / radius - crowding`, with crowding
//!    the sum over the friends of `1 - distance / FRIEND_SPACING_M` within
//!    [`FRIEND_SPACING_M`].
//! 4. **The way there**, for the [`MAX_PATH_CHECKS`] best: clear at knee height
//!    ([`KNEE_HEIGHT_M`], against solids only), straight or round one or two corners of the solid in
//!    the way; the distance is then the length of that way, and a point it cannot reach scores
//!    [`UNREACHABLE_PENALTY`] less.
//! 5. **A peek**, for the [`MAX_PEEK_CHECKS`] best it can reach: the point itself when its
//!    eyes see the first threat over the cover (it stands up to fire), else the first point
//!    stepped sideways by [`PEEK_STEPS_M`] (left, then right) from which they do; a point with
//!    a peek scores 1 more.
//!
//! The candidates come back best first (equal scores keep the order they were made in).

use std::collections::BTreeSet;

use gamecraft_math::Portable;
use gamecraft_world::{EntityId, HeightField, World};
use serde::Serialize;

use crate::perception::CHEST_SHARE;
use crate::ray;
use crate::solid::{self, Solid};

/// How far beyond the character's radius the points around a solid lie, metres.
pub const COVER_CLEARANCE_M: f64 = 0.3;
/// Points on each of the two sample rings.
pub const RING_SAMPLES: usize = 12;
/// Most candidates one query makes.
pub const MAX_COVER_CANDIDATES: usize = 64;
/// Most threats one query weighs.
pub const MAX_COVER_THREATS: usize = 4;
/// Most candidates whose way there one query checks.
pub const MAX_PATH_CHECKS: usize = 8;
/// Most candidates whose peek one query looks for.
pub const MAX_PEEK_CHECKS: usize = 8;
/// The sideways steps of a peek, metres, tried in this order.
pub const PEEK_STEPS_M: [f64; 3] = [0.75, 1.5, 2.5];
/// Within this distance of a friend a point is crowded, metres.
pub const FRIEND_SPACING_M: f64 = 3.0;
/// The height above the feet at which the way to a point is checked, metres (above a step).
pub const KNEE_HEIGHT_M: f64 = 0.5;
/// The search radius of `query.cover` when neither the call nor a behaviour gives one, metres.
pub const DEFAULT_COVER_SEARCH_M: f64 = 15.0;
/// What a point it cannot reach loses of its score.
pub const UNREACHABLE_PENALTY: f64 = 10.0;
/// Weight of protection in the score.
const W_PROTECTION: f64 = 2.0;
/// Weight of the way's length (over the search radius) in the score.
const W_DISTANCE: f64 = 0.5;
/// Weight of crowding in the score.
const W_CROWDING: f64 = 1.0;
/// What a peek adds to the score.
const W_PEEK: f64 = 1.0;

/// Where a candidate comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverSource {
    /// Where the character stands.
    Here,
    /// Around a solid shape or model.
    Solid,
    /// A ring sample.
    Ring,
}

/// A threat a cover is weighed against: a point it sees from (and is fired at), and the entity
/// it is, which does not block its own view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Threat {
    /// The point, world metres (a character's chest).
    pub point: [f64; 3],
    /// The entity, `None` for a bare point.
    pub entity: Option<EntityId>,
}

/// What a cover query asks.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverAsk<'a> {
    /// The character looking for cover, which blocks nothing; `None` for a bare position.
    pub me: Option<EntityId>,
    /// Where it stands (its feet), world metres.
    pub feet: [f64; 3],
    /// Its height, metres.
    pub height_m: f64,
    /// Its radius, metres.
    pub radius_m: f64,
    /// The height of its eyes above its feet, metres.
    pub eye_height_m: f64,
    /// How far around itself it looks, metres.
    pub search_m: f64,
    /// The threats, most important first.
    pub threats: &'a [Threat],
    /// Where its friends stand (or are going), `[x, z]`.
    pub friends: &'a [[f64; 2]],
}

/// One scored candidate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CoverPoint {
    /// Where to stand (the feet), world metres.
    pub point: [f64; 3],
    /// Where it comes from.
    pub source: CoverSource,
    /// The share of the threats that cannot see its chest there, 0 to 1.
    pub protection: f64,
    /// The way there, metres (straight until the way is checked).
    pub distance_m: f64,
    /// The corners the way goes round, in order (none when it is straight).
    pub via: [Option<[f64; 3]>; 2],
    /// Whether it can get there; `None` when that was not checked (outside the budget).
    pub reachable: Option<bool>,
    /// Where it steps to fire at the first threat; `None` when there is no such place or it
    /// was not looked for.
    pub peek: Option<[f64; 3]>,
    /// How crowded the point is with friends.
    pub crowding: f64,
    /// The score; higher is better.
    pub score: f64,
}

fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    (dx * dx + dz * dz).sqrt()
}

/// The whole query (see the module docs) with the solids of `world`.
pub fn evaluate_world(
    world: &World,
    heights: Option<&HeightField>,
    ask: &CoverAsk<'_>,
) -> Vec<CoverPoint> {
    evaluate(world, heights, &solid::solids_of(world), ask)
}

/// The whole query (see the module docs). Empty without threats or with a position or sizes
/// that are not finite.
pub(crate) fn evaluate(
    world: &World,
    heights: Option<&HeightField>,
    solids: &[Solid],
    ask: &CoverAsk<'_>,
) -> Vec<CoverPoint> {
    let threats: Vec<Threat> = ask
        .threats
        .iter()
        .filter(|t| t.point.iter().all(|v| v.is_finite()))
        .take(MAX_COVER_THREATS)
        .copied()
        .collect();
    let sane = ask.feet.iter().all(|v| v.is_finite())
        && [ask.height_m, ask.radius_m, ask.eye_height_m, ask.search_m]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
        && ask.search_m > 0.0;
    if threats.is_empty() || !sane {
        return Vec::new();
    }
    let picker = Picker {
        world,
        heights,
        solids,
        ask,
        threats: &threats,
        solid_ids: solids.iter().map(|s| s.entity).collect(),
    };
    picker.run()
}

struct Picker<'a> {
    world: &'a World,
    heights: Option<&'a HeightField>,
    solids: &'a [Solid],
    ask: &'a CoverAsk<'a>,
    threats: &'a [Threat],
    solid_ids: BTreeSet<EntityId>,
}

impl Picker<'_> {
    fn run(&self) -> Vec<CoverPoint> {
        let mut points: Vec<CoverPoint> = self
            .candidates()
            .into_iter()
            .filter_map(|(xz, source)| self.weigh(xz, source))
            .collect();
        sort_best_first(&mut points);
        let search = self.ask.search_m;
        for c in points.iter_mut().take(MAX_PATH_CHECKS) {
            let (reachable, via, length) = self.way(c.point);
            c.reachable = Some(reachable);
            c.via = via;
            c.distance_m = length;
            c.score = W_PROTECTION * c.protection
                - W_DISTANCE * length / search
                - W_CROWDING * c.crowding
                - if reachable { 0.0 } else { UNREACHABLE_PENALTY };
        }
        sort_best_first(&mut points);
        let mut peeks = 0;
        for c in &mut points {
            if peeks >= MAX_PEEK_CHECKS {
                break;
            }
            if c.reachable != Some(true) {
                continue;
            }
            peeks += 1;
            c.peek = self.peek(c.point);
            if c.peek.is_some() {
                c.score += W_PEEK;
            }
        }
        sort_best_first(&mut points);
        points
    }

    /// The candidate points `[x, z]`, in the order of the module docs.
    fn candidates(&self) -> Vec<([f64; 2], CoverSource)> {
        let [fx, _, fz] = self.ask.feet;
        let search = self.ask.search_m;
        let mut out = vec![([fx, fz], CoverSource::Here)];
        let within = |p: [f64; 2]| {
            let (dx, dz) = (p[0] - fx, p[1] - fz);
            (dx * dx + dz * dz).sqrt() <= search
        };
        let clearance = self.ask.radius_m + COVER_CLEARANCE_M;
        for s in self.solids {
            if Some(s.entity) == self.ask.me {
                continue;
            }
            let [cx, cz] = s.centre();
            let (dx, dz) = (cx - fx, cz - fz);
            if (dx * dx + dz * dz).sqrt() > search + s.reach() + clearance {
                continue;
            }
            for p in s.around(clearance) {
                if out.len() >= MAX_COVER_CANDIDATES {
                    return out;
                }
                if within(p) {
                    out.push((p, CoverSource::Solid));
                }
            }
        }
        for r in [search / 2.0, search] {
            for i in 0..RING_SAMPLES {
                if out.len() >= MAX_COVER_CANDIDATES {
                    return out;
                }
                let angle = std::f64::consts::TAU * i as f64 / RING_SAMPLES as f64;
                let (s, c) = angle.psin_cos();
                out.push(([fx + r * c, fz + r * s], CoverSource::Ring));
            }
        }
        out
    }

    /// The feet at `[x, z]`: on the terrain, or at the character's height without one.
    fn stand(&self, [x, z]: [f64; 2]) -> [f64; 3] {
        let y = self
            .heights
            .and_then(|h| h.height_at(x, z))
            .unwrap_or(self.ask.feet[1]);
        [x, y, z]
    }

    /// Whether a character standing at `feet` overlaps a solid.
    fn blocked(&self, feet: [f64; 3]) -> bool {
        self.solids.iter().any(|s| {
            s.push(
                [feet[0], feet[2]],
                self.ask.radius_m,
                feet[1],
                self.ask.height_m,
            )
            .is_some()
        })
    }

    /// A candidate with its protection, crowding and first score; `None` where it cannot
    /// stand.
    fn weigh(&self, xz: [f64; 2], source: CoverSource) -> Option<CoverPoint> {
        let feet = self.stand(xz);
        if !feet.iter().all(|v| v.is_finite()) || self.blocked(feet) {
            return None;
        }
        let chest = [feet[0], feet[1] + self.ask.height_m * CHEST_SHARE, feet[2]];
        let hidden = self.threats.iter().filter(|t| !self.sees(t, chest)).count();
        let protection = hidden as f64 / self.threats.len().max(1) as f64;
        let crowding: f64 = self
            .ask
            .friends
            .iter()
            .map(|f| {
                let d = horizontal(feet, [f[0], 0.0, f[1]]);
                (1.0 - d / FRIEND_SPACING_M).max(0.0)
            })
            .sum();
        let distance_m = horizontal(self.ask.feet, feet);
        let score = W_PROTECTION * protection
            - W_DISTANCE * distance_m / self.ask.search_m
            - W_CROWDING * crowding;
        Some(CoverPoint {
            point: feet,
            source,
            protection,
            distance_m,
            via: [None, None],
            reachable: None,
            peek: None,
            crowding,
            score,
        })
    }

    /// Whether the threat sees `to`: a clear line of sight, the threat and the character
    /// itself blocking nothing.
    fn sees(&self, t: &Threat, to: [f64; 3]) -> bool {
        let me = self.ask.me;
        let skip = |id: EntityId| Some(id) == me || Some(id) == t.entity;
        ray::line_of_sight(self.world, self.heights, t.point, to, skip)
            .is_some_and(|s| s.blocked_by.is_none())
    }

    /// Whether the way from feet `a` to feet `b` at knee height is clear of solids, and the
    /// solid entity in the way when it is not.
    fn clear(&self, a: [f64; 3], b: [f64; 3]) -> Result<(), Option<EntityId>> {
        let me = self.ask.me;
        let from = [a[0], a[1] + KNEE_HEIGHT_M, a[2]];
        let to = [b[0], b[1] + KNEE_HEIGHT_M, b[2]];
        let skip = |id: EntityId| Some(id) == me || !self.solid_ids.contains(&id);
        match ray::line_of_sight(self.world, None, from, to, skip) {
            Some(s) => match s.blocked_by {
                None => Ok(()),
                Some(found) => Err(found.entity),
            },
            None => Err(None),
        }
    }

    /// Whether it can walk to `to`, round which corners (one or two of the solid in the way),
    /// and how far that is. The shortest way wins; on a tie the first found.
    fn way(&self, to: [f64; 3]) -> (bool, [Option<[f64; 3]>; 2], f64) {
        let from = self.ask.feet;
        let straight = horizontal(from, to);
        let in_the_way = match self.clear(from, to) {
            Ok(()) => return (true, [None, None], straight),
            Err(None) => return (false, [None, None], straight),
            Err(Some(id)) => id,
        };
        let clearance = self.ask.radius_m + COVER_CLEARANCE_M;
        let corners: Vec<[f64; 3]> = self
            .solids
            .iter()
            .filter(|s| s.entity == in_the_way)
            .flat_map(|s| s.around(clearance).into_iter().skip(4))
            .map(|c| self.stand(c))
            .filter(|c| !self.blocked(*c))
            .collect();
        type Way = Option<([Option<[f64; 3]>; 2], f64)>;
        let mut best: Way = None;
        let consider = |best: &mut Way, via: [Option<[f64; 3]>; 2], length: f64| {
            if best.is_none_or(|(_, l)| length < l) {
                *best = Some((via, length));
            }
        };
        let from_ok: Vec<bool> = corners
            .iter()
            .map(|c| self.clear(from, *c).is_ok())
            .collect();
        let to_ok: Vec<bool> = corners.iter().map(|c| self.clear(*c, to).is_ok()).collect();
        for (i, c) in corners.iter().enumerate() {
            if from_ok.get(i) == Some(&true) && to_ok.get(i) == Some(&true) {
                consider(
                    &mut best,
                    [Some(*c), None],
                    horizontal(from, *c) + horizontal(*c, to),
                );
            }
        }
        if best.is_none() {
            for (i, a) in corners.iter().enumerate() {
                if from_ok.get(i) != Some(&true) {
                    continue;
                }
                for (j, b) in corners.iter().enumerate() {
                    if i == j || to_ok.get(j) != Some(&true) || self.clear(*a, *b).is_err() {
                        continue;
                    }
                    consider(
                        &mut best,
                        [Some(*a), Some(*b)],
                        horizontal(from, *a) + horizontal(*a, *b) + horizontal(*b, to),
                    );
                }
            }
        }
        match best {
            Some((via, length)) => (true, via, length),
            None => (false, [None, None], straight),
        }
    }

    /// Where it steps from cover at `p` to see the first threat.
    fn peek(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        let t = self.threats.first()?;
        let eye = |f: [f64; 3]| [f[0], f[1] + self.ask.eye_height_m, f[2]];
        // The threat's view of the eyes is the eyes' view of the threat.
        if self.sees(t, eye(p)) {
            return Some(p);
        }
        let (dx, dz) = (t.point[0] - p[0], t.point[2] - p[2]);
        let d = (dx * dx + dz * dz).sqrt();
        if !(d.is_finite() && d > 0.0) {
            return None;
        }
        let side = [-dz / d, dx / d];
        for step in PEEK_STEPS_M {
            for sign in [1.0, -1.0] {
                let q = self.stand([p[0] + sign * step * side[0], p[2] + sign * step * side[1]]);
                if !q.iter().all(|v| v.is_finite()) || self.blocked(q) || self.clear(p, q).is_err()
                {
                    continue;
                }
                if self.sees(t, eye(q)) {
                    return Some(q);
                }
            }
        }
        None
    }
}

impl crate::Sim {
    /// The cover query (see the module docs of [`crate::cover`]) in the running world.
    pub fn cover(&self, ask: &CoverAsk<'_>) -> Vec<CoverPoint> {
        evaluate(&self.world, self.heights.as_deref(), &self.solids, ask)
    }
}

/// Sorts best first; equal scores (and any score that is not a number, last) keep their
/// order.
fn sort_best_first(points: &mut [CoverPoint]) {
    points.sort_by(|a, b| {
        let key = |c: &CoverPoint| {
            if c.score.is_nan() {
                f64::NEG_INFINITY
            } else {
                c.score
            }
        };
        key(b).total_cmp(&key(a))
    });
}
