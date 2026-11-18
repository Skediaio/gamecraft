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

use gamecraft_math::{Portable, Rng, rad_to_deg, wrap_deg};
use gamecraft_world::{Behaviour, BehaviourMode, Entity, EntityId, HeightField, World};
use serde::Serialize;

use crate::combat::cone_offset;
use crate::perception::{CHEST_SHARE, sees};
use crate::{DT, GRAVITY_MPS2, Input};

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
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
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
        }
    }

    /// Keeps the brain valid for a changed behaviour: a waypoint beyond the route starts over,
    /// and a base state follows the mode.
    pub(crate) fn adapt(&mut self, behaviour: &Behaviour) {
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
}

impl Mind<'_> {
    /// The nearest enemy it sees.
    fn enemy(&self) -> Option<&Entity> {
        let mine = team_of(self.me)?;
        let mut best: Option<(f64, &Entity)> = None;
        for other in self.world.entities() {
            if other.id == self.me.id || other.character().is_none() || other.is_down() {
                continue;
            }
            if team_of(other).is_none_or(|t| t == mine) {
                continue;
            }
            if !sees(self.world, self.heights, self.me, other) {
                continue;
            }
            let d = horizontal(self.me.position, other.position);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, other));
            }
        }
        best.map(|(_, e)| e)
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

    /// Faces and aims at `target`; with `fire`, also holds the trigger with the aim error.
    fn aim(&self, target: &Entity, fire: bool) -> Input {
        let eye = self.eye();
        let chest = chest(target);
        let yaw = yaw_towards(eye, chest).unwrap_or(self.me.rotation_y_deg);
        let pitch = aim_pitch_deg(eye, chest, self.muzzle_velocity_mps.unwrap_or(0.0));
        let (yaw, pitch) = if fire {
            let mut rng = Rng::new(self.world.seed)
                .fork(LABEL_AIM)
                .fork(self.me.id.0)
                .fork(self.shots);
            let (dy, dp) = cone_offset(&mut rng, self.behaviour.aim_error_mrad);
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
        let enemy = self.enemy();
        // Transitions.
        if let Some(t) = enemy {
            brain.last_seen = Some(chest(t));
            brain.state = match brain.state {
                BrainState::Alert { target, wait_s } if target == t.id => {
                    let left = wait_s - DT;
                    if left <= 0.0 {
                        BrainState::Engage { target: t.id }
                    } else {
                        BrainState::Alert {
                            target: t.id,
                            wait_s: left,
                        }
                    }
                }
                BrainState::Engage { .. } => BrainState::Engage { target: t.id },
                _ if b.reaction_s <= 0.0 => BrainState::Engage { target: t.id },
                _ => BrainState::Alert {
                    target: t.id,
                    wait_s: b.reaction_s,
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
        // The input of the state.
        match brain.state {
            BrainState::Alert { .. } => match enemy {
                Some(t) => self.aim(t, false),
                None => self.stand(self.me.rotation_y_deg),
            },
            BrainState::Engage { .. } => match enemy {
                Some(t) if horizontal(self.me.position, t.position) <= b.engage_range_m => {
                    self.aim(t, true)
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
