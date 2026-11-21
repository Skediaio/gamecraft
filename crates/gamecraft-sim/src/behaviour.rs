//! The `behaviour` state machine: what an enemy decides each tick, written as an [`Input`]
//! record, exactly the record a player's keyboard or an agent writes.
//!
//! Before the characters move, every character with a behaviour that is not down and not taken
//! over by an agent (`sim.input`, until `sim.release`) decides, in id order:
//!
//! - **Enemies.** The characters it [sees](crate::perception) whose team is set and differs
//!   from its own (set) team, and that are not down; the nearest counts (the lowest id on a
//!   tie). A character's team is its behaviour's `team`, or the name in a tag `team:<name>`.
//! - **Alert.** On first seeing an enemy it stands, turns to it and waits `reaction_s`; then it
//!   **engages**: within `engage_range_m` it stands, aims at the enemy's chest (allowing for
//!   the drop of its launcher's projectile over the distance, as in a vacuum) with an aim error
//!   drawn per shot from a uniform cone of `aim_error_mrad` (the world's seeded stream forked
//!   by [`LABEL_AIM`], its id and its launcher's shot number) and holds `fire`; farther away it
//!   runs towards it.
//! - **Search.** When it loses sight of its enemy, or hears a shot while not fighting, it runs
//!   to the last place it saw the enemy (or the shot's muzzle), looks around there turning at
//!   [`SEARCH_TURN_DEG_PER_S`], and after `search_s` **returns**: it walks back to its post
//!   (`guard`, `idle`) or to its route (`patrol`).
//! - **Patrol** walks the route point after point (within [`ARRIVE_M`] counts as there) and
//!   starts again at the first; **guard** stands at its post facing the way it faced at the
//!   start; **idle** stands.
//!
//! With the numbers of skill, precision, detection and cover ([`crate::ai`]) it also knows
//! every enemy gradually (it reacts to a seen enemy only when it knows it well enough, and
//! fires only when it knows it better), keeps an **alertness** (`safe`, `aware`, `combat`:
//! contact, or stress of [`crate::ai::STRESS_COVER_AT`], makes combat; a shot heard or an
//! enemy glimpsed makes aware; `calm_s` without contact outside a fight calms it one step,
//! never below its behaviour's `alertness`), settles its aim and feels near misses. In combat,
//! with `cover_search_m` above 0 and a threat known, it fights from **cover**
//! ([`crate::cover`], queried every [`crate::ai::COVER_EVAL_TICKS`] ticks): it runs there
//! (round the corner of the solid in the way), stays down [`crate::ai::HIDE_S`], steps to its
//! peek point and fires for [`crate::ai::PEEK_S`], and goes back down; from a stress of
//! [`crate::ai::SUPPRESSED_AT`] it does not peek. With every one of those numbers at its
//! default it decides exactly as without them.

use std::collections::BTreeMap;

use gamecraft_math::{Portable, Rng, rad_to_deg, wrap_deg};
use gamecraft_world::limits::BEHAVIOUR_TIME_MAX_S;
use gamecraft_world::{Alertness, Behaviour, BehaviourMode, Entity, EntityId, HeightField, World};
use serde::Serialize;

use crate::ai::{self, CoverPhase, InCover, Knowledge};
use crate::combat::cone_offset;
use crate::cover::{self, CoverAsk, Threat};
use crate::perception::{CHEST_SHARE, effective_range_m, sees};
use crate::solid::Solid;
use crate::{Body, DT, GRAVITY_MPS2, Input, Projectile};

/// The label of the aim error's random stream (forked from the world seed).
pub const LABEL_AIM: u64 = 0xA1_3E44;
/// Within this horizontal distance of a point, metres, a character is there.
pub const ARRIVE_M: f64 = 1.0;
/// How fast a searching character turns to look around, degrees per second.
pub const SEARCH_TURN_DEG_PER_S: f64 = 60.0;
/// The tag prefix that gives a character without a behaviour its team.
pub const TEAM_TAG_PREFIX: &str = "team:";

/// What a behaviour is doing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BrainState {
    /// Stands.
    Idle,
    /// Walks its route.
    Patrol,
    /// Stands at its post.
    Guard,
    /// Has seen `target` and waits `wait_s` more before engaging.
    Alert {
        /// The enemy.
        target: EntityId,
        /// Seconds left of its reaction time.
        wait_s: f64,
    },
    /// Fires at `target`, or runs towards it when it is out of range.
    Engage {
        /// The enemy.
        target: EntityId,
    },
    /// Searches around `point` ([x, z]) for `left_s` more seconds.
    Search {
        /// Where it searches.
        point: [f64; 2],
        /// Seconds left.
        left_s: f64,
    },
    /// Walks back to its post or route.
    Return,
}

impl BrainState {
    /// The state's name, as in JSON.
    pub fn name(&self) -> &'static str {
        match self {
            BrainState::Idle => "idle",
            BrainState::Patrol => "patrol",
            BrainState::Guard => "guard",
            BrainState::Alert { .. } => "alert",
            BrainState::Engage { .. } => "engage",
            BrainState::Search { .. } => "search",
            BrainState::Return => "return",
        }
    }

    fn base(mode: BehaviourMode) -> BrainState {
        match mode {
            BehaviourMode::Idle => BrainState::Idle,
            BehaviourMode::Patrol => BrainState::Patrol,
            BehaviourMode::Guard => BrainState::Guard,
        }
    }
}

/// The running state of one behaviour (not in the document).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Brain {
    /// What it is doing.
    pub state: BrainState,
    /// The route point it walks to next.
    pub waypoint: usize,
    /// Where it stood when its behaviour started: its post.
    pub post: [f64; 3],
    /// Which way it faced then, degrees.
    pub post_yaw_deg: f64,
    /// Where it last saw an enemy's chest.
    pub last_seen: Option<[f64; 3]>,
    /// How alert it is.
    pub alertness: Alertness,
    /// Seconds without contact left before it calms down one step.
    pub calm_left_s: f64,
    /// What it knows of every enemy it has noticed, by id.
    pub knowledge: BTreeMap<EntityId, Knowledge>,
    /// Stress from near misses, 0 to 1.
    pub stress: f64,
    /// The target its aim is settling on.
    pub aim_target: Option<EntityId>,
    /// Seconds it has kept that target in view.
    pub aim_settled_s: f64,
    /// The cover it chose, while it fights from cover.
    pub cover: Option<InCover>,
    /// Ticks until it queries for cover again.
    pub cover_eval_in: u32,
}

impl Brain {
    /// A behaviour starting at the entity's place, in its mode's base state.
    pub(crate) fn new(entity: &Entity, behaviour: &Behaviour) -> Brain {
        Brain {
            state: BrainState::base(behaviour.mode),
            waypoint: 0,
            post: entity.position,
            post_yaw_deg: entity.rotation_y_deg,
            last_seen: None,
            alertness: behaviour.alertness,
            calm_left_s: behaviour.calm_s,
            knowledge: BTreeMap::new(),
            stress: 0.0,
            aim_target: None,
            aim_settled_s: 0.0,
            cover: None,
            cover_eval_in: 0,
        }
    }

    /// How well it knows `id`, 0 to 1.
    pub fn knows(&self, id: EntityId) -> f64 {
        self.knowledge.get(&id).map_or(0.0, |k| k.knowledge)
    }

    /// Keeps the brain valid for a changed behaviour: a waypoint beyond the route starts over,
    /// a base state follows the mode, a changed `alertness` puts it in that alertness, and it
    /// is never calmer than its behaviour's `alertness`.
    pub(crate) fn adapt(&mut self, behaviour: &Behaviour, old: Option<&Behaviour>) {
        if old.is_some_and(|o| o.alertness != behaviour.alertness) {
            self.alertness = behaviour.alertness;
            self.calm_left_s = behaviour.calm_s;
        }
        self.alertness = ai::raised(self.alertness, behaviour.alertness);
        if self.waypoint >= behaviour.route.len() {
            self.waypoint = 0;
        }
        if matches!(
            self.state,
            BrainState::Idle | BrainState::Patrol | BrainState::Guard
        ) {
            self.state = BrainState::base(behaviour.mode);
        }
    }
}

/// A character's team: its behaviour's team, else the name of its first `team:` tag; `None`
/// when empty or absent.
pub fn team_of(e: &Entity) -> Option<&str> {
    let from_behaviour = e
        .behaviour()
        .map(|b| b.team.as_str())
        .filter(|t| !t.is_empty());
    from_behaviour.or_else(|| {
        e.tags()
            .iter()
            .find_map(|t| t.strip_prefix(TEAM_TAG_PREFIX))
            .filter(|t| !t.is_empty())
    })
}

/// The yaw (degrees, counter-clockwise from north) that faces from `from` to `to`; `None`
/// when they coincide horizontally.
pub(crate) fn yaw_towards(from: [f64; 3], to: [f64; 3]) -> Option<f64> {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    let d = (dx * dx + dz * dz).sqrt();
    // yaw_forward(yaw) = (-sin yaw, 0, -cos yaw), so yaw = atan2(-dx, -dz).
    (d.is_finite() && d > 1e-9).then(|| wrap_deg(rad_to_deg((-dx).patan2(-dz))))
}

/// The pitch (degrees) that sends a projectile at `speed_mps` from `from` to `to`, allowing for
/// its drop in a vacuum over the flight time `distance / speed`.
pub fn aim_pitch_deg(from: [f64; 3], to: [f64; 3], speed_mps: f64) -> f64 {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let d = (dx * dx + dz * dz).sqrt();
    let t = if speed_mps > 0.0 { d / speed_mps } else { 0.0 };
    let drop = 0.5 * GRAVITY_MPS2 * t * t;
    let pitch = rad_to_deg((dy + drop).patan2(d));
    if pitch.is_finite() {
        pitch.clamp(-crate::MAX_PITCH_DEG, crate::MAX_PITCH_DEG)
    } else {
        0.0
    }
}

/// Everything a decision reads besides the brain.
pub(crate) struct Mind<'a> {
    pub(crate) world: &'a World,
    pub(crate) heights: Option<&'a HeightField>,
    /// The deciding character.
    pub(crate) me: &'a Entity,
    pub(crate) behaviour: &'a Behaviour,
    /// The muzzle velocity of its launcher, `None` without one.
    pub(crate) muzzle_velocity_mps: Option<f64>,
    /// Its launcher's shot number.
    pub(crate) shots: u64,
    /// The muzzle of the last shot it heard since its last decision.
    pub(crate) heard: Option<[f64; 3]>,
    /// The solids of the running world.
    pub(crate) solids: &'a [Solid],
    /// The projectiles in flight (each flew the last tick).
    pub(crate) projectiles: &'a [Projectile],
    /// Every character's input.
    pub(crate) inputs: &'a BTreeMap<EntityId, Input>,
    /// Every character's body.
    pub(crate) bodies: &'a BTreeMap<EntityId, Body>,
    /// Every behaviour's brain before this tick's decisions.
    pub(crate) brains: &'a BTreeMap<EntityId, Brain>,
}

impl<'a> Mind<'a> {
    /// The enemies it sees, in id order.
    fn perceived(&self) -> Vec<&'a Entity> {
        let Some(mine) = team_of(self.me) else {
            return Vec::new();
        };
        let (world, heights, me) = (self.world, self.heights, self.me);
        world
            .entities()
            .filter(|other| {
                other.id != me.id
                    && other.character().is_some()
                    && !other.is_down()
                    && team_of(other).is_some_and(|t| t != mine)
                    && sees(world, heights, me, other)
            })
            .collect()
    }

    /// The nearest of `seen` it knows well enough to react to (the lowest id on a tie).
    fn enemy(&self, brain: &Brain, seen: &[&'a Entity]) -> Option<&'a Entity> {
        let mut best: Option<(f64, &Entity)> = None;
        for &other in seen {
            if brain.knows(other.id) < ai::REACT_KNOWLEDGE {
                continue;
            }
            let d = horizontal(self.me.position, other.position);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, other));
            }
        }
        best.map(|(_, e)| e)
    }

    /// Stress recovers, and the projectiles of others that passed close in the last tick add
    /// to it; an enemy's near miss makes it know the shooter.
    fn feel(&self, brain: &mut Brain) {
        let courage = self.behaviour.courage.clamp(0.0, 1.0);
        brain.stress = (brain.stress - ai::STRESS_RECOVERY_PER_S * DT).max(0.0);
        let me = chest(self.me);
        let mine = team_of(self.me);
        for p in self.projectiles {
            if p.shooter == self.me.id {
                continue;
            }
            let start = [
                p.position[0] - p.velocity[0] * DT,
                p.position[1] - p.velocity[1] * DT,
                p.position[2] - p.velocity[2] * DT,
            ];
            let d = ai::segment_distance(start, p.position, me);
            if !(d.is_finite() && d <= ai::NEAR_MISS_M) {
                continue;
            }
            brain.stress +=
                ai::STRESS_PER_NEAR_MISS * (1.0 - courage) * (1.0 - d / ai::NEAR_MISS_M);
            brain.stress = if brain.stress.is_finite() {
                brain.stress.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let Some(shooter) = self.world.entity(p.shooter) else {
                continue;
            };
            let enemy = mine.is_some() && team_of(shooter).is_some_and(|t| Some(t) != mine);
            if enemy && shooter.character().is_some() && !shooter.is_down() {
                let k = brain.knowledge.entry(shooter.id).or_insert(Knowledge {
                    knowledge: 0.0,
                    last_known: chest(shooter),
                    seen: false,
                });
                k.knowledge = k.knowledge.max(ai::REACT_KNOWLEDGE);
                k.last_known = chest(shooter);
            }
        }
    }

    /// Knowledge rises for the enemies it sees and falls for the others.
    fn learn(&self, brain: &mut Brain, seen: &[&Entity], time_factor: f64) {
        let b = self.behaviour;
        let range = effective_range_m(&self.world.environment, b.view_range_m);
        let spot_s = b.spot_time_s * time_factor / ai::alert_gain(brain.alertness);
        for k in brain.knowledge.values_mut() {
            k.seen = false;
        }
        for &t in seen {
            let gain = self.spot_gain(t, range);
            let k = brain.knowledge.entry(t.id).or_insert(Knowledge {
                knowledge: 0.0,
                last_known: chest(t),
                seen: false,
            });
            k.seen = true;
            k.last_known = chest(t);
            let next = if spot_s > 0.0 {
                k.knowledge + gain / spot_s * DT
            } else {
                1.0
            };
            k.knowledge = if next.is_finite() {
                next.clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
        let world = self.world;
        brain.knowledge.retain(|id, k| {
            if !k.seen {
                k.knowledge -= DT / ai::FORGET_S;
            }
            k.knowledge > 0.0 && world.entity(*id).is_some_and(|e| !e.is_down())
        });
        ai::cap_known(&mut brain.knowledge);
    }

    /// How much faster than at half its range, standing still and silent, it knows `t`.
    fn spot_gain(&self, t: &Entity, range: f64) -> f64 {
        let (lo, hi) = ai::DISTANCE_GAIN_RANGE;
        let d = horizontal(self.me.position, t.position);
        let distance = if d > 0.0 { range / 2.0 / d } else { hi };
        let distance = if distance.is_finite() {
            distance.clamp(lo, hi)
        } else {
            hi
        };
        let moving = self.bodies.get(&t.id).is_some_and(|body| {
            let [vx, _, vz] = body.velocity;
            (vx * vx + vz * vz).sqrt() > ai::MOVING_SPEED_MPS
        });
        let firing = self.inputs.get(&t.id).is_some_and(|i| i.fire);
        let moving = if moving { ai::MOVING_SPOT_GAIN } else { 1.0 };
        let firing = if firing { ai::FIRING_SPOT_GAIN } else { 1.0 };
        distance * moving * firing
    }

    /// Alertness follows contact and calms down without it.
    fn alert(&self, brain: &mut Brain, contact: bool, noticed: bool) {
        let b = self.behaviour;
        if contact {
            brain.alertness = Alertness::Combat;
            brain.calm_left_s = b.calm_s;
        } else if noticed && brain.alertness != Alertness::Combat {
            brain.alertness = Alertness::Aware;
            brain.calm_left_s = b.calm_s;
        } else if !fighting(brain.state) && brain.alertness != Alertness::Safe {
            brain.calm_left_s -= DT;
            if brain.calm_left_s <= 0.0 {
                brain.alertness = ai::calmer(brain.alertness, b.alertness);
                brain.calm_left_s = b.calm_s;
            }
        }
        brain.alertness = ai::raised(brain.alertness, b.alertness);
    }

    /// The threats it knows well enough to react to, best known first (the lowest id on a
    /// tie), at most [`cover::MAX_COVER_THREATS`].
    fn threats(brain: &Brain) -> Vec<Threat> {
        let mut known: Vec<(EntityId, &Knowledge)> = brain
            .knowledge
            .iter()
            .filter(|(_, k)| k.knowledge >= ai::REACT_KNOWLEDGE)
            .map(|(id, k)| (*id, k))
            .collect();
        known.sort_by(|(ia, a), (ib, b)| b.knowledge.total_cmp(&a.knowledge).then(ia.cmp(ib)));
        known
            .into_iter()
            .take(cover::MAX_COVER_THREATS)
            .map(|(id, k)| Threat {
                point: k.last_known,
                entity: Some(id),
            })
            .collect()
    }

    /// Where its friends stand, and the covers they chose.
    fn friends(&self) -> Vec<[f64; 2]> {
        let Some(mine) = team_of(self.me) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in self.world.entities() {
            if e.id == self.me.id || e.character().is_none() || e.is_down() {
                continue;
            }
            if team_of(e) != Some(mine) {
                continue;
            }
            out.push([e.position[0], e.position[2]]);
            if let Some(c) = self.brains.get(&e.id).and_then(|b| b.cover) {
                out.push([c.at.point[0], c.at.point[2]]);
            }
        }
        out
    }

    /// Chooses, keeps or drops a cover, and gives the input of fighting from it; `None` when
    /// it does not fight from cover now.
    fn take_cover(
        &self,
        brain: &mut Brain,
        enemy: Option<&Entity>,
        error_mrad: f64,
        time_factor: f64,
    ) -> Option<Input> {
        let b = self.behaviour;
        let threats = Self::threats(brain);
        let Some(first) = threats.first() else {
            brain.cover = None;
            brain.cover_eval_in = 0;
            return None;
        };
        if b.cover_search_m <= 0.0 || brain.alertness != Alertness::Combat || !fighting(brain.state)
        {
            brain.cover = None;
            brain.cover_eval_in = 0;
            return None;
        }
        let peeking = matches!(brain.cover.map(|c| c.phase), Some(CoverPhase::Peek { .. }));
        if brain.cover_eval_in == 0 && !peeking {
            let ticks = f64::from(ai::COVER_EVAL_TICKS) * time_factor;
            brain.cover_eval_in = if ticks.is_finite() && ticks >= 1.0 {
                ticks.round().min(f64::from(u32::MAX)) as u32
            } else {
                ai::COVER_EVAL_TICKS
            };
            self.choose_cover(brain, &threats);
        } else {
            brain.cover_eval_in = brain.cover_eval_in.saturating_sub(1);
        }
        let mut c = brain.cover?;
        let face = yaw_towards(self.me.position, first.point).unwrap_or(self.me.rotation_y_deg);
        let at = |p: [f64; 3], within: f64| horizontal(self.me.position, p) <= within;
        let engaging = matches!(brain.state, BrainState::Engage { .. });
        let fight = |t: &Entity| {
            let fire = engaging && horizontal(self.me.position, t.position) <= b.engage_range_m;
            self.aim(t, fire, error_mrad)
        };
        let hide_s = ai::HIDE_S * time_factor;
        let (input, phase) = match c.phase {
            CoverPhase::Move { left_s } => {
                let left = left_s - DT;
                if left <= 0.0 {
                    brain.cover = None;
                    brain.cover_eval_in = 0;
                    return None;
                }
                let corners: Vec<[f64; 3]> = c.at.via.into_iter().flatten().collect();
                while let Some(&corner) = corners.get(usize::from(c.via_passed)) {
                    if !at(corner, ai::CORNER_ARRIVE_M) {
                        break;
                    }
                    c.via_passed = c.via_passed.saturating_add(1);
                }
                match corners.get(usize::from(c.via_passed)) {
                    Some(&corner) => (self.go(corner, true), CoverPhase::Move { left_s: left }),
                    None if at(c.at.point, ai::COVER_ARRIVE_M) => {
                        (self.stand(face), CoverPhase::Hide { left_s: hide_s })
                    }
                    None => (self.go(c.at.point, true), CoverPhase::Move { left_s: left }),
                }
            }
            CoverPhase::Hide { left_s } => {
                let input = if !at(c.at.point, 2.0 * ai::COVER_ARRIVE_M) {
                    self.go(c.at.point, true)
                } else {
                    // Seen from cover after all: fight from where it is.
                    enemy.map_or_else(|| self.stand(face), fight)
                };
                let left = left_s - DT;
                let phase = if left > 0.0 {
                    CoverPhase::Hide { left_s: left }
                } else if brain.stress < ai::SUPPRESSED_AT && c.at.peek.is_some() {
                    CoverPhase::Peek { left_s: ai::PEEK_S }
                } else {
                    CoverPhase::Hide { left_s: hide_s }
                };
                (input, phase)
            }
            CoverPhase::Peek { left_s } => {
                let peek = c.at.peek.unwrap_or(c.at.point);
                let input = if !at(peek, ai::COVER_ARRIVE_M) {
                    self.go(peek, true)
                } else {
                    enemy.map_or_else(|| self.stand(face), fight)
                };
                let left = left_s - DT;
                let phase = if left > 0.0 && brain.stress < ai::SUPPRESSED_AT {
                    CoverPhase::Peek { left_s: left }
                } else {
                    CoverPhase::Hide { left_s: hide_s }
                };
                (input, phase)
            }
        };
        c.phase = phase;
        brain.cover = Some(c);
        Some(input)
    }

    /// Queries for cover against `threats` and keeps, changes or drops the brain's cover.
    fn choose_cover(&self, brain: &mut Brain, threats: &[Threat]) {
        let Some(character) = self.me.character() else {
            brain.cover = None;
            return;
        };
        let friends = self.friends();
        let ask = CoverAsk {
            me: Some(self.me.id),
            feet: self.me.position,
            height_m: character.height_m,
            radius_m: character.radius_m,
            eye_height_m: character.eye_height_m,
            search_m: self.behaviour.cover_search_m,
            threats,
            friends: &friends,
        };
        let found = cover::evaluate(self.world, self.heights, self.solids, &ask);
        let best = found
            .iter()
            .find(|c| c.reachable == Some(true) && c.protection > 0.0)
            .copied();
        let Some(best) = best else {
            brain.cover = None;
            return;
        };
        let moving_to = |at| InCover {
            at,
            via_passed: 0,
            phase: CoverPhase::Move {
                left_s: ai::COVER_MOVE_MAX_S,
            },
        };
        let Some(current) = brain.cover else {
            brain.cover = Some(moving_to(best));
            return;
        };
        let near = |a: [f64; 3], b: [f64; 3]| horizontal(a, b) <= ai::COVER_ARRIVE_M;
        if near(best.point, current.at.point) {
            return;
        }
        let fresh = found.iter().find(|c| near(c.point, current.at.point));
        let (protection, score) = fresh.map_or((current.at.protection, current.at.score), |f| {
            (f.protection, f.score)
        });
        if protection <= 0.0 || best.score > score + ai::COVER_SWITCH_MARGIN {
            brain.cover = Some(moving_to(best));
        }
    }

    fn eye(&self) -> [f64; 3] {
        let [x, y, z] = self.me.position;
        let up = self.me.character().map_or(0.0, |c| c.eye_height_m);
        [x, y + up, z]
    }

    /// Where it goes back to: its route's next point, or its post.
    fn home(&self, brain: &Brain) -> [f64; 3] {
        match (
            self.behaviour.mode,
            self.behaviour.route.get(brain.waypoint),
        ) {
            (BehaviourMode::Patrol, Some(&[x, z])) => [x, self.me.position[1], z],
            _ => brain.post,
        }
    }

    /// An input that walks (or runs) towards `to`.
    fn go(&self, to: [f64; 3], run: bool) -> Input {
        match yaw_towards(self.me.position, to) {
            Some(yaw_deg) => Input {
                forward: 1.0,
                yaw_deg,
                run,
                ..Input::default()
            },
            None => self.stand(self.me.rotation_y_deg),
        }
    }

    fn stand(&self, yaw_deg: f64) -> Input {
        Input {
            yaw_deg,
            ..Input::default()
        }
    }

    /// Faces and aims at `target`; with `fire`, also holds the trigger with an aim error of
    /// `error_mrad`.
    fn aim(&self, target: &Entity, fire: bool, error_mrad: f64) -> Input {
        let eye = self.eye();
        let chest = chest(target);
        let yaw = yaw_towards(eye, chest).unwrap_or(self.me.rotation_y_deg);
        let pitch = aim_pitch_deg(eye, chest, self.muzzle_velocity_mps.unwrap_or(0.0));
        let (yaw, pitch) = if fire {
            let mut rng = Rng::new(self.world.seed)
                .fork(LABEL_AIM)
                .fork(self.me.id.0)
                .fork(self.shots);
            let (dy, dp) = cone_offset(&mut rng, error_mrad);
            (yaw + rad_to_deg(dy), pitch + rad_to_deg(dp))
        } else {
            (yaw, pitch)
        };
        Input {
            yaw_deg: wrap_deg(yaw),
            pitch_deg: pitch.clamp(-crate::MAX_PITCH_DEG, crate::MAX_PITCH_DEG),
            fire: fire && self.muzzle_velocity_mps.is_some(),
            ..Input::default()
        }
    }

    /// One decision: updates `brain` and gives the input it writes.
    pub(crate) fn decide(&self, brain: &mut Brain) -> Input {
        let b = self.behaviour;
        let time_factor = ai::time_factor(b.skill);
        self.feel(brain);
        let seen = self.perceived();
        self.learn(brain, &seen, time_factor);
        let enemy = self.enemy(brain, &seen);
        let reaction_s = b.reaction_s * time_factor;
        // Transitions.
        if let Some(t) = enemy {
            let can_fire = brain.knows(t.id) >= ai::FIRE_KNOWLEDGE;
            brain.last_seen = Some(chest(t));
            brain.state = match brain.state {
                BrainState::Alert { target, wait_s } if target == t.id => {
                    let left = wait_s - DT;
                    if left > 0.0 {
                        BrainState::Alert {
                            target: t.id,
                            wait_s: left,
                        }
                    } else if can_fire {
                        BrainState::Engage { target: t.id }
                    } else {
                        BrainState::Alert {
                            target: t.id,
                            wait_s: 0.0,
                        }
                    }
                }
                BrainState::Engage { .. } if can_fire => BrainState::Engage { target: t.id },
                BrainState::Engage { .. } => BrainState::Alert {
                    target: t.id,
                    wait_s: 0.0,
                },
                _ if reaction_s <= 0.0 && can_fire => BrainState::Engage { target: t.id },
                _ => BrainState::Alert {
                    target: t.id,
                    wait_s: reaction_s,
                },
            };
        } else {
            let lost = matches!(
                brain.state,
                BrainState::Alert { .. } | BrainState::Engage { .. }
            );
            let point = if lost {
                Some(brain.last_seen.unwrap_or(self.me.position))
            } else {
                self.heard
            };
            if let Some([x, _, z]) = point {
                brain.state = BrainState::Search {
                    point: [x, z],
                    left_s: b.search_s,
                };
            } else if let BrainState::Search { point, left_s } = brain.state {
                let left = left_s - DT;
                brain.state = if left <= 0.0 {
                    BrainState::Return
                } else {
                    BrainState::Search {
                        point,
                        left_s: left,
                    }
                };
            }
        }
        let contact = enemy.is_some() || brain.stress >= ai::STRESS_COVER_AT;
        let noticed = !seen.is_empty() || self.heard.is_some();
        self.alert(brain, contact, noticed);
        match enemy {
            Some(t) if brain.aim_target == Some(t.id) => {
                brain.aim_settled_s = (brain.aim_settled_s + DT).min(BEHAVIOUR_TIME_MAX_S);
            }
            other => {
                brain.aim_target = other.map(|t| t.id);
                brain.aim_settled_s = 0.0;
            }
        }
        let error_mrad = ai::aim_error_mrad(b, brain.aim_settled_s, brain.stress);
        if let Some(input) = self.take_cover(brain, enemy, error_mrad, time_factor) {
            return input;
        }
        // The input of the state.
        match brain.state {
            BrainState::Alert { .. } => match enemy {
                Some(t) => self.aim(t, false, error_mrad),
                None => self.stand(self.me.rotation_y_deg),
            },
            BrainState::Engage { .. } => match enemy {
                Some(t) if horizontal(self.me.position, t.position) <= b.engage_range_m => {
                    self.aim(t, true, error_mrad)
                }
                Some(t) => self.go(t.position, true),
                None => self.stand(self.me.rotation_y_deg),
            },
            BrainState::Search { point: [x, z], .. } => {
                let to = [x, self.me.position[1], z];
                if horizontal(self.me.position, to) > ARRIVE_M {
                    self.go(to, true)
                } else {
                    self.stand(wrap_deg(
                        self.me.rotation_y_deg + SEARCH_TURN_DEG_PER_S * DT,
                    ))
                }
            }
            BrainState::Return => {
                let home = self.home(brain);
                if horizontal(self.me.position, home) > ARRIVE_M {
                    self.go(home, false)
                } else {
                    brain.state = BrainState::base(b.mode);
                    self.stand(self.me.rotation_y_deg)
                }
            }
            BrainState::Patrol => {
                let mut target = self.home(brain);
                if horizontal(self.me.position, target) <= ARRIVE_M && !b.route.is_empty() {
                    brain.waypoint = (brain.waypoint + 1) % b.route.len();
                    target = self.home(brain);
                }
                self.go(target, false)
            }
            BrainState::Guard => {
                if horizontal(self.me.position, brain.post) > ARRIVE_M {
                    self.go(brain.post, false)
                } else {
                    self.stand(brain.post_yaw_deg)
                }
            }
            BrainState::Idle => self.stand(self.me.rotation_y_deg),
        }
    }
}

/// A character's chest (its position without a character component).
fn chest(e: &Entity) -> [f64; 3] {
    let [x, y, z] = e.position;
    [
        x,
        y + e.character().map_or(0.0, |c| c.height_m * CHEST_SHARE),
        z,
    ]
}

fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    (dx * dx + dz * dz).sqrt()
}

/// Whether a state is part of a fight: alert, engaging or searching.
fn fighting(state: BrainState) -> bool {
    matches!(
        state,
        BrainState::Alert { .. } | BrainState::Engage { .. } | BrainState::Search { .. }
    )
}
