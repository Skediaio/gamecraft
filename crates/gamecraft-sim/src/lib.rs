//! The GameCraft simulation (layer L2): a world that runs.
//!
//! A [`Sim`] owns a copy of a world document and advances it in fixed steps of [`DT`] seconds.
//! Characters move because of [`Input`] records, whoever wrote them (a keyboard, a script, an
//! agent, the network). The same world and the same inputs give bit-identical state on every
//! platform; [`Sim::state_hash`] is the oracle for that (AGENTS.md, rule 3).
//!
//! A running world can be changed between two ticks with [`Sim::replace_world`]: that is the
//! game master's hand (AGENTS.md, rule 6).
//!
//! # Scenario events
//!
//! After the characters have moved, every tick detects [`Event`]s and keeps them until
//! [`Sim::take_events`] hands them out:
//!
//! - **time**: the end of the tick (`tick * DT`) reached the `at_s` of an enabled `time`
//!   trigger of the world, which the tick before had not (an `at_s` of 0 fires on the first
//!   tick);
//! - **enter** and **leave**: a character came inside, or is no longer inside, the zone of an
//!   entity with a `zone` component (see [`gamecraft_world::Zone`]). Membership is kept from
//!   tick to tick, starting empty, so a character inside a zone at the start enters on the
//!   first tick. A removed zone or character drops its membership without an event;
//! - **interact**: a character's input `interact` went from false to true (held, it fires
//!   once) and an `interactable` is within reach of its eye and faced, within 45 degrees; the
//!   event names the nearest one.
//!
//! The events of a tick are ordered by time first (by `at_s`), then by the acting character's
//! id, then by the zone's or target's id (leave, enter, then interact for the same pair). The
//! state hash covers zone membership and the held interaction, not the events waiting to be
//! taken. Lights and tags do not affect the simulation.
//!
//! # Projectiles, health and behaviour
//!
//! Every tick runs, in this order: the behaviours decide ([`behaviour`] docs: they write the
//! input of their characters), the characters move (a character that is down takes no input),
//! the launchers fire (inputs holding `fire`; [`combat`] docs), and the projectiles fly
//! ([`ballistics`] for the flight, [`combat`] for hits, damage and `down`). Then the events
//! above are detected. The events of a tick are the scenario events above, then `heard`,
//! `hit` and `down` in the order they happened (shots in id order of the shooter, projectiles
//! in id order). A shot of [`Sim::fire`] (between two ticks) is heard at once.
//!
//! Perception (what a character sees) is described in [`perception`]; the ray maths it shares
//! with the engine's queries is [`ray`].
//!
//! No GPU, no files, no clock, no threads.
//!
//! # The model (M0: characters on terrain among solid shapes)
//!
//! Every tick steps the characters one by one in id order:
//!
//! 1. **Facing.** With an input record the entity's `rotation_y_deg` becomes the input's
//!    (wrapped) `yaw_deg`. Without one the character keeps its rotation and wants no motion.
//!    An input that holds a [`Face`] turns first: at the start of every tick, after the
//!    behaviours decided and before anybody moves, characters in id order get the `yaw_deg`
//!    that faces the point, or the entity's [`aim::aim_point`] (a character's chest), from
//!    where they stand; with [`Face::Entity`] `pitch` and a launcher, also the `pitch_deg`
//!    that allows for the drop in a vacuum from the muzzle
//!    ([`behaviour::aim_pitch_deg`], as behaviours aim; [`aim::solve`] gives the exact
//!    solution against drag and wind). A point under the character's feet, or an entity that
//!    no longer exists, leaves the facing as it was. A character that is down does not turn.
//! 2. **Wanted horizontal velocity.** `yaw_forward(yaw) * forward + right * right`, the
//!    `(forward, right)` pair clamped to the unit disc, times `run_speed_mps` when running
//!    and `walk_speed_mps` otherwise; "right" is the facing turned 90 degrees clockwise seen
//!    from above. It applies at once, on the ground and in the air: there is **no inertia or
//!    friction** in M0. [`Body::velocity`] holds this wanted horizontal velocity, even on a
//!    tick a wall or a slope held the character back.
//! 3. **Jump and gravity.** A grounded character with `jump` gets `jump_speed_mps` upwards and
//!    leaves the ground. In the air `vy -= GRAVITY_MPS2 * DT`, then `y += vy * DT`
//!    (semi-implicit Euler: velocity first).
//! 4. **Slope limit.** A character standing on the terrain does not take a horizontal step
//!    over which the terrain rises more than `tan(max_slope_deg)` per metre; it stays where it
//!    is that tick. Downhill is always allowed, and there is no sliding: a character without
//!    input stands still on any slope.
//! 5. **Solids block.** A solid box, sphere or cylinder (rotated by its entity's
//!    `rotation_y_deg`, sized by `size * scale`, standing on its entity's position) whose
//!    vertical span overlaps the character's (feet to feet + `height_m`) and whose top is more
//!    than [`STEP_HEIGHT_M`] above the feet pushes the character's circle (`radius_m`) out of
//!    its footprint: circle against rotated rectangle for a box, circle against circle for a
//!    cylinder or a sphere (the sphere's widest circle within the overlap). Up to
//!    [`MAX_PUSH_ITERATIONS`] rounds; two coinciding centres push along +X.
//! 6. **Ground.** The ground under the character's centre is the highest of the terrain and
//!    the tops of the solid boxes and cylinders whose footprint contains the centre and whose
//!    top is at most [`STEP_HEIGHT_M`] above the feet (so a low box is a step to stand on). A
//!    falling character whose feet reach the ground lands (`vy = 0`, grounded). A grounded
//!    character follows the ground up, and down by at most [`STEP_HEIGHT_M`] per tick; a
//!    larger drop makes it fall from the next tick on.
//! 7. **The edge of the world.** With a terrain, the centre stays inside the terrain square.
//!    Without any ground beneath, a character falls to [`FALL_FLOOR_M`] and rests there.
//!    Positions also stay within the world document's coordinate limit, and no non-finite
//!    number is ever written.
//!
//! What M0 leaves out: characters do not collide with each other (a shape on a character's
//! own entity is ignored as an obstacle); a character's entity `scale` does not change its
//! size or speed; spheres block but are not ground to stand on; plants are no obstacles;
//! there are no rigid bodies, inertia or friction.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use gamecraft_math::{StateHasher, wrap_deg};
use gamecraft_world::{Entity, EntityId, HeightField, World};
use serde::{Deserialize, Serialize};

pub mod aim;
pub mod ballistics;
pub mod behaviour;
pub mod combat;
mod events;
mod motion;
pub mod perception;
pub mod ray;
mod solid;

pub use behaviour::{Brain, BrainState};
pub use combat::{Magazine, Projectile};
pub use events::{Event, EventKind};

use motion::{State, Surroundings, clamp_to_disc};
use solid::Solid;

/// Simulation ticks per second.
pub const TICK_HZ: u32 = 60;
/// Seconds per tick.
pub const DT: f64 = 1.0 / 60.0;
/// Downward acceleration in m/s2.
pub const GRAVITY_MPS2: f64 = 9.81;
/// Most ticks one call to [`Sim::step`] advances (ten minutes); more is clamped.
pub const MAX_STEP_TICKS: u32 = 36_000;
/// Highest rise, metres, a character steps up (or follows down) without jumping or falling.
pub const STEP_HEIGHT_M: f64 = 0.35;
/// Most rounds of pushing a character out of solid shapes per tick.
pub const MAX_PUSH_ITERATIONS: u32 = 4;
/// Without any ground beneath, a character falls until here and is put to rest, metres.
pub const FALL_FLOOR_M: f64 = -1000.0;
/// Most events kept until [`Sim::take_events`]; later ones are dropped and counted.
pub const MAX_PENDING_EVENTS: usize = 100_000;

/// Substeps of a projectile's flight per tick: a shot at 900 m/s moves at most about 2 m per
/// substep.
pub const PROJECTILE_SUBSTEPS: u32 = 8;
/// Steepest aim of an input's `pitch_deg`, up or down, degrees.
pub const MAX_PITCH_DEG: f64 = 89.0;
/// Steepest aim of [`Sim::fire`], up or down, degrees (straight up is allowed).
pub const MAX_FIRE_PITCH_DEG: f64 = 90.0;

/// Marks the projectiles, launchers, health, behaviours and aim in the state hash.
const HASH_MARK_COMBAT: u64 = 0xC0BA_7C0B_A7C0_BA7C;
/// Marks the zone memberships in the state hash.
const HASH_MARK_INSIDE: u64 = 0x5A0E_5A0E_5A0E_5A0E;
/// Marks the held interactions in the state hash.
const HASH_MARK_HELD: u64 = 0x1A7E_1A7E_1A7E_1A7E;
/// Marks the held facings in the state hash.
const HASH_MARK_FACE: u64 = 0xFACE_FACE_FACE_FACE;

/// What a character keeps facing, turned towards at the start of every tick (see the crate
/// docs, *Facing*).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Face {
    /// A point of the ground plane, `[x, z]`.
    Point([f64; 2]),
    /// An entity's aim point ([`aim::aim_point`]: a character's chest).
    Entity {
        /// The entity.
        id: EntityId,
        /// Also aim the pitch at it, when the character has a launcher.
        pitch: bool,
    },
}

/// What a character is asked to do, until the record is replaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    /// -1 (back) to 1 (forward), along the facing.
    pub forward: f64,
    /// -1 (left) to 1 (right), across the facing.
    pub right: f64,
    /// The facing: becomes the entity's `rotation_y_deg` (0 faces north, -Z; counter-clockwise
    /// seen from above).
    pub yaw_deg: f64,
    /// Run instead of walk.
    pub run: bool,
    /// Held: a jump starts on every tick the character stands on the ground.
    pub jump: bool,
    /// Held: an interaction happens once, on the tick it goes from false to true.
    pub interact: bool,
    /// Where the character aims, degrees up from level, -89 to 89 (clamped). It does not turn
    /// the body.
    pub pitch_deg: f64,
    /// Held: the character's launcher fires every `rate_s` (while it has shots).
    pub fire: bool,
    /// Held: what the character keeps turning towards, every tick; `None` keeps `yaw_deg`.
    pub face: Option<Face>,
}

/// The moving state of one character that is not in the document. A new body is at rest and
/// not yet grounded: it settles on the ground in its first tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Body {
    /// Metres per second, world axes.
    pub velocity: [f64; 3],
    /// Standing on the ground or on a solid.
    pub grounded: bool,
}

/// Why the simulation refused something. `Display` gives a sentence an agent can act on.
#[derive(Clone, Debug, PartialEq)]
pub enum SimError {
    /// The world is not valid, or its terrain cannot be built.
    World(String),
    /// No entity with this id.
    NoEntity(EntityId),
    /// The entity has no `character` component, so it takes no input.
    NotACharacter(EntityId),
    /// An input field is not a finite number.
    BadInput(String),
    /// The entity has no `launcher` component.
    NoLauncher(EntityId),
    /// The launcher cannot fire yet: why, as a sentence.
    NotReady(String),
    /// The entity is down and does nothing.
    Down(EntityId),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(why) => f.write_str(why),
            Self::NoEntity(id) => write!(f, "no entity {id} in the running world"),
            Self::NotACharacter(id) => write!(
                f,
                "entity {id} has no `character` component, so it takes no input"
            ),
            Self::BadInput(why) => write!(f, "bad input: {why}"),
            Self::NoLauncher(id) => write!(
                f,
                "entity {id} has no `launcher` component, so it cannot fire; add one with \
                 entity.set_component"
            ),
            Self::NotReady(why) => f.write_str(why),
            Self::Down(id) => write!(f, "entity {id} is down (its health is 0) and does nothing"),
        }
    }
}

impl std::error::Error for SimError {}

/// A running world.
#[derive(Clone, Debug)]
pub struct Sim {
    world: World,
    heights: Option<Arc<HeightField>>,
    tick: u64,
    bodies: BTreeMap<EntityId, Body>,
    inputs: BTreeMap<EntityId, Input>,
    /// The solid shapes of `world`, derived by [`Sim::replace_world`].
    solids: Vec<Solid>,
    /// The hash of `world` without its characters' positions and rotations.
    static_hash: u64,
    /// The zones, interactables and trigger times of `world`, derived by
    /// [`Sim::replace_world`].
    watch: events::Watch,
    /// Which characters are inside which zones: (character, zone).
    inside: BTreeSet<(EntityId, EntityId)>,
    /// Characters whose input held `interact` at the end of the last tick.
    held: BTreeSet<EntityId>,
    /// Events waiting for [`Sim::take_events`].
    events: Vec<Event>,
    /// Events dropped because more than [`MAX_PENDING_EVENTS`] were waiting.
    dropped_events: u64,
    /// Projectiles in flight, in id order.
    projectiles: Vec<Projectile>,
    /// The id of the next projectile.
    next_projectile: u64,
    /// The state of every launcher of the running world.
    magazines: BTreeMap<EntityId, Magazine>,
    /// The state of every behaviour of a character of the running world.
    brains: BTreeMap<EntityId, Brain>,
    /// Characters with a behaviour whose input an agent took over.
    taken_over: BTreeSet<EntityId>,
    /// Shots heard since the listeners last decided: (listener, muzzle), in order.
    sounds: Vec<(EntityId, [f64; 3])>,
}

impl Sim {
    /// Starts a simulation of `world` at tick 0: validates it, builds its height field, and
    /// gives every character a body at rest.
    pub fn new(world: World) -> Result<Sim, SimError> {
        let mut sim = Sim {
            world: World::new("", 0),
            heights: None,
            tick: 0,
            bodies: BTreeMap::new(),
            inputs: BTreeMap::new(),
            solids: Vec::new(),
            static_hash: 0,
            watch: events::Watch::default(),
            inside: BTreeSet::new(),
            held: BTreeSet::new(),
            events: Vec::new(),
            dropped_events: 0,
            projectiles: Vec::new(),
            next_projectile: 1,
            magazines: BTreeMap::new(),
            brains: BTreeMap::new(),
            taken_over: BTreeSet::new(),
            sounds: Vec::new(),
        };
        sim.replace_world(world)?;
        Ok(sim)
    }

    /// The running world: every character's `position` and `rotation_y_deg` are current.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Ends the simulation and hands back the running world.
    pub fn into_world(self) -> World {
        self.world
    }

    /// The height field of the running world's terrain, `None` without terrain.
    pub fn heights(&self) -> Option<&HeightField> {
        self.heights.as_deref()
    }

    /// Hands out the events detected since the last call, in order, and forgets them.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Events dropped since the start because more than [`MAX_PENDING_EVENTS`] were waiting.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// Which characters are inside which zones, as (character, zone), in id order.
    pub fn inside(&self) -> impl Iterator<Item = (EntityId, EntityId)> + '_ {
        self.inside.iter().copied()
    }

    /// Ticks stepped since the start.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Simulated seconds since the start: `tick * DT`.
    pub fn time_s(&self) -> f64 {
        self.tick as f64 * DT
    }

    /// Stores the input of a character; it holds until replaced. `forward` and `right` are
    /// clamped to the unit disc, `yaw_deg` is wrapped and `pitch_deg` clamped to
    /// [`MAX_PITCH_DEG`]; a non-finite field (or `face` point) or a `face` entity that does not
    /// exist is an error. A held `face` turns the stored input at once, and again every tick. A
    /// character with a behaviour is taken over: its behaviour stops deciding until
    /// [`Sim::release`].
    pub fn set_input(&mut self, id: EntityId, input: Input) -> Result<(), SimError> {
        let entity = self.world.entity(id).ok_or(SimError::NoEntity(id))?;
        if entity.character().is_none() {
            return Err(SimError::NotACharacter(id));
        }
        let behaves = entity.behaviour().is_some();
        for (field, value) in [
            ("forward", input.forward),
            ("right", input.right),
            ("yaw_deg", input.yaw_deg),
            ("pitch_deg", input.pitch_deg),
        ] {
            if !value.is_finite() {
                return Err(SimError::BadInput(format!(
                    "`{field}` of the input for entity {id} is {value}; it must be a finite \
                     number ({})",
                    if field == "yaw_deg" {
                        "degrees, any value; it is wrapped into 0 to 360"
                    } else if field == "pitch_deg" {
                        "degrees up from level, -89 to 89"
                    } else {
                        "-1 to 1; longer (forward, right) pairs are scaled onto the unit disc"
                    }
                )));
            }
        }
        match input.face {
            Some(Face::Point([x, z])) if !(x.is_finite() && z.is_finite()) => {
                return Err(SimError::BadInput(format!(
                    "`face` of the input for entity {id} is [{x}, {z}]; it must be a point \
                     [x, z] of finite metres"
                )));
            }
            Some(Face::Entity { id: target, .. }) if self.world.entity(target).is_none() => {
                return Err(SimError::NoEntity(target));
            }
            _ => {}
        }
        let (forward, right) = clamp_to_disc(input.forward, input.right);
        self.inputs.insert(
            id,
            Input {
                forward,
                right,
                yaw_deg: wrap_deg(input.yaw_deg),
                pitch_deg: input.pitch_deg.clamp(-MAX_PITCH_DEG, MAX_PITCH_DEG),
                ..input
            },
        );
        if behaves {
            self.taken_over.insert(id);
        }
        self.turn(id);
        Ok(())
    }

    /// Turns the input of `id` towards what it holds in `face`, if anything (see the crate
    /// docs, *Facing*).
    fn turn(&mut self, id: EntityId) {
        let Some(input) = self.inputs.get(&id).copied() else {
            return;
        };
        let Some(face) = input.face else {
            return;
        };
        let Some(me) = self.world.entity(id) else {
            return;
        };
        if me.is_down() {
            return;
        }
        let (to, pitch) = match face {
            Face::Point([x, z]) => ([x, me.position[1], z], false),
            Face::Entity { id: target, pitch } => match self.world.entity(target) {
                Some(t) => (aim::aim_point(t), pitch),
                None => return,
            },
        };
        let mut turned = input;
        if let Some(yaw) = behaviour::yaw_towards(me.position, to) {
            turned.yaw_deg = yaw;
        }
        if pitch && let Some(launcher) = me.launcher() {
            let from = combat::muzzle(me, launcher);
            turned.pitch_deg = behaviour::aim_pitch_deg(from, to, launcher.muzzle_velocity_mps);
        }
        if turned != input {
            self.inputs.insert(id, turned);
        }
    }

    /// Every input holding a `face` turns, in id order.
    fn turn_all(&mut self) {
        let facing: Vec<EntityId> = self
            .inputs
            .iter()
            .filter(|(_, i)| i.face.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in facing {
            self.turn(id);
        }
    }

    /// Hands a character taken over by [`Sim::set_input`] back to its behaviour, which decides
    /// again from the next tick. Releasing a character that is not taken over changes nothing.
    pub fn release(&mut self, id: EntityId) -> Result<(), SimError> {
        let entity = self.world.entity(id).ok_or(SimError::NoEntity(id))?;
        if entity.character().is_none() {
            return Err(SimError::NotACharacter(id));
        }
        self.taken_over.remove(&id);
        Ok(())
    }

    /// Whether an agent's input took over this character's behaviour.
    pub fn is_taken_over(&self, id: EntityId) -> bool {
        self.taken_over.contains(&id)
    }

    /// The behaviour state of a character, `None` without a behaviour.
    pub fn brain(&self, id: EntityId) -> Option<&Brain> {
        self.brains.get(&id)
    }

    /// Every behaviour's state, in id order.
    pub fn brains(&self) -> impl Iterator<Item = (EntityId, &Brain)> {
        self.brains.iter().map(|(id, b)| (*id, b))
    }

    /// The state of an entity's launcher, `None` without one.
    pub fn magazine(&self, id: EntityId) -> Option<&Magazine> {
        self.magazines.get(&id)
    }

    /// The projectiles in flight, in id order.
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }

    /// Whether character `a` sees character `b` now ([`perception`]). False when either is not
    /// a character of the running world.
    pub fn sees(&self, a: EntityId, b: EntityId) -> bool {
        match (self.world.entity(a), self.world.entity(b)) {
            (Some(a), Some(b)) => perception::sees(&self.world, self.heights.as_deref(), a, b),
            _ => false,
        }
    }

    /// Fires the launcher of `id` now, between two ticks: along `yaw_deg` (default: the
    /// entity's rotation) and `pitch_deg` (default: a character's input pitch, else 0; -90 to
    /// 90), with its dispersion. The projectile flies from the next tick; the shot is heard at
    /// once. Returns the projectile's id. An error when the entity has no launcher, is down, or
    /// the launcher is not ready (it names how long to wait).
    pub fn fire(
        &mut self,
        id: EntityId,
        yaw_deg: Option<f64>,
        pitch_deg: Option<f64>,
    ) -> Result<u64, SimError> {
        let entity = self.world.entity(id).ok_or(SimError::NoEntity(id))?;
        let launcher = entity.launcher().ok_or(SimError::NoLauncher(id))?;
        if entity.is_down() {
            return Err(SimError::Down(id));
        }
        let yaw = yaw_deg.unwrap_or(entity.rotation_y_deg);
        let pitch = pitch_deg
            .or_else(|| self.inputs.get(&id).map(|i| i.pitch_deg))
            .unwrap_or(0.0);
        for (field, value) in [("yaw_deg", yaw), ("pitch_deg", pitch)] {
            if !value.is_finite() {
                return Err(SimError::BadInput(format!(
                    "`{field}` of the shot of entity {id} is {value}; it must be a finite number \
                     of degrees"
                )));
            }
        }
        if pitch.abs() > MAX_FIRE_PITCH_DEG {
            return Err(SimError::BadInput(format!(
                "`pitch_deg` of the shot of entity {id} is {pitch}; it must be from \
                 -{MAX_FIRE_PITCH_DEG} to {MAX_FIRE_PITCH_DEG} degrees"
            )));
        }
        if self.projectiles.len() >= combat::MAX_PROJECTILES {
            return Err(SimError::NotReady(format!(
                "{} projectiles are in flight, the most there can be; step until some land",
                combat::MAX_PROJECTILES
            )));
        }
        let mut magazine = self
            .magazines
            .get(&id)
            .copied()
            .unwrap_or_else(|| Magazine::full(launcher));
        if !magazine.ready() {
            let what = if magazine.reloading {
                "it is reloading"
            } else {
                "it fired a moment ago"
            };
            return Err(SimError::NotReady(format!(
                "the launcher of entity {id} is not ready: {what}; ready in {:.3} s",
                magazine.ready_in_s
            )));
        }
        let projectile_id = self.next_projectile;
        let fired = combat::try_fire(
            &self.world,
            entity,
            launcher,
            &mut magazine,
            wrap_deg(yaw),
            pitch,
            projectile_id,
        );
        let Some((projectile, heard)) = fired else {
            return Err(SimError::NotReady(format!(
                "the launcher of entity {id} is not ready"
            )));
        };
        self.magazines.insert(id, magazine);
        self.next_projectile = self.next_projectile.saturating_add(1);
        self.projectiles.push(projectile);
        let tick = self.tick;
        for kind in heard {
            self.note_sound(&kind);
            self.push_event(tick, kind);
        }
        Ok(projectile_id)
    }

    /// Keeps a `heard` event for its listener's next decision.
    fn note_sound(&mut self, kind: &EventKind) {
        if let EventKind::Heard { who, point, .. } = kind
            && self.brains.contains_key(who)
            && self.sounds.len() < MAX_PENDING_EVENTS
        {
            self.sounds.push((*who, *point));
        }
    }

    /// Keeps an event for [`Sim::take_events`], or counts it as dropped.
    fn push_event(&mut self, tick: u64, kind: EventKind) {
        if self.events.len() < MAX_PENDING_EVENTS {
            self.events.push(Event { tick, kind });
        } else {
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
    }

    /// The behaviours decide: each writes its character's input.
    fn think(&mut self) {
        let sounds = std::mem::take(&mut self.sounds);
        let mut decided = Vec::new();
        for (&id, brain) in &self.brains {
            if self.taken_over.contains(&id) {
                continue;
            }
            let Some(me) = self.world.entity(id) else {
                continue;
            };
            let Some(behaviour) = me.behaviour() else {
                continue;
            };
            if me.character().is_none() || me.is_down() {
                continue;
            }
            let heard = sounds
                .iter()
                .rev()
                .find(|(who, _)| *who == id)
                .map(|(_, p)| *p);
            let mind = behaviour::Mind {
                world: &self.world,
                heights: self.heights.as_deref(),
                me,
                behaviour,
                muzzle_velocity_mps: me.launcher().map(|l| l.muzzle_velocity_mps),
                shots: self.magazines.get(&id).map_or(0, |m| m.shots),
                heard,
            };
            let mut next = *brain;
            let input = mind.decide(&mut next);
            decided.push((id, next, input));
        }
        for (id, brain, input) in decided {
            self.brains.insert(id, brain);
            let finite = [input.forward, input.right, input.yaw_deg, input.pitch_deg]
                .iter()
                .all(|v| v.is_finite());
            if finite {
                self.inputs.insert(id, input);
            }
        }
    }

    /// The launchers wait a tick, and those whose character holds `fire` shoot.
    fn shoot(&mut self) -> Vec<EventKind> {
        let mut events = Vec::new();
        let ids: Vec<EntityId> = self.magazines.keys().copied().collect();
        for id in ids {
            let Some(entity) = self.world.entity(id) else {
                continue;
            };
            let Some(launcher) = entity.launcher() else {
                continue;
            };
            let Some(mut magazine) = self.magazines.get(&id).copied() else {
                continue;
            };
            magazine.wait(DT, launcher);
            let input = self.inputs.get(&id).copied();
            let wants = entity.character().is_some()
                && !entity.is_down()
                && input.is_some_and(|i| i.fire)
                && self.projectiles.len() < combat::MAX_PROJECTILES;
            if wants
                && let Some(i) = input
                && let Some((projectile, heard)) = combat::try_fire(
                    &self.world,
                    entity,
                    launcher,
                    &mut magazine,
                    i.yaw_deg,
                    i.pitch_deg,
                    self.next_projectile,
                )
            {
                self.next_projectile = self.next_projectile.saturating_add(1);
                self.projectiles.push(projectile);
                events.extend(heard);
            }
            self.magazines.insert(id, magazine);
        }
        events
    }

    /// The projectiles fly one tick.
    fn fly(&mut self) -> Vec<EventKind> {
        let mut events = Vec::new();
        let heights = self.heights.clone();
        let mut flying = std::mem::take(&mut self.projectiles);
        flying.retain_mut(
            |p| match combat::fly(&mut self.world, heights.as_deref(), p) {
                Some(hit) => {
                    events.extend(hit);
                    false
                }
                None => true,
            },
        );
        self.projectiles = flying;
        events
    }

    /// The stored input of a character, `None` when it never got one.
    pub fn input(&self, id: EntityId) -> Option<&Input> {
        self.inputs.get(&id)
    }

    /// The body of a character, `None` when the entity is not one.
    pub fn body(&self, id: EntityId) -> Option<&Body> {
        self.bodies.get(&id)
    }

    /// Every character's body, in id order.
    pub fn bodies(&self) -> impl Iterator<Item = (EntityId, &Body)> {
        self.bodies.iter().map(|(id, body)| (*id, body))
    }

    /// Advances `ticks` fixed steps (at most [`MAX_STEP_TICKS`]). Never fails and never writes
    /// a non-finite number into the world.
    pub fn step(&mut self, ticks: u32) {
        let ids: Vec<EntityId> = self.bodies.keys().copied().collect();
        for _ in 0..ticks.min(MAX_STEP_TICKS) {
            self.think();
            self.turn_all();
            let around = Surroundings {
                heights: self.heights.as_deref(),
                solids: &self.solids,
            };
            for &id in &ids {
                let Some(entity) = self.world.entity(id) else {
                    continue;
                };
                let Some(character) = entity.character() else {
                    continue;
                };
                let state = State {
                    position: entity.position,
                    rotation_y_deg: entity.rotation_y_deg,
                    body: self.bodies.get(&id).copied().unwrap_or_default(),
                };
                // A character that is down takes no input.
                let input = if entity.is_down() {
                    None
                } else {
                    self.inputs.get(&id)
                };
                let next = around.step(state, character, input);
                if let Some(entity) = self.world.entity_mut(id) {
                    entity.position = next.position;
                    entity.rotation_y_deg = next.rotation_y_deg;
                }
                self.bodies.insert(id, next.body);
            }
            let mut combat = self.shoot();
            combat.extend(self.fly());
            self.tick = self.tick.saturating_add(1);
            self.detect(&ids);
            let tick = self.tick;
            for kind in combat {
                self.note_sound(&kind);
                self.push_event(tick, kind);
            }
        }
    }

    /// Detects the events of the tick that just ended (see the crate docs) and updates zone
    /// membership and the held interactions.
    fn detect(&mut self, characters: &[EntityId]) {
        let mut found: Vec<EventKind> = self
            .watch
            .times_of(self.tick, DT)
            .map(|at_s| EventKind::Time { at_s })
            .collect();
        let now = events::memberships(&self.world, characters, &self.watch.zones);
        let mut held = BTreeSet::new();
        for &id in characters {
            let mut mine: Vec<(EntityId, u8, EventKind)> = Vec::new();
            for &(who, zone) in self
                .inside
                .range((id, EntityId(0))..=(id, EntityId(u64::MAX)))
            {
                if !now.contains(&(who, zone)) {
                    mine.push((zone, 0, EventKind::Leave { who, zone }));
                }
            }
            for &(who, zone) in now.range((id, EntityId(0))..=(id, EntityId(u64::MAX))) {
                if !self.inside.contains(&(who, zone)) {
                    mine.push((zone, 1, EventKind::Enter { who, zone }));
                }
            }
            let pressing = self.inputs.get(&id).is_some_and(|i| i.interact);
            if pressing {
                held.insert(id);
                if !self.held.contains(&id)
                    && let Some(who) = self.world.entity(id)
                    && let Some(character) = who.character()
                    && let Some(target) = events::reached(
                        &self.world,
                        who,
                        character.eye_height_m,
                        &self.watch.interactables,
                    )
                {
                    mine.push((target, 2, EventKind::Interact { who: id, target }));
                }
            }
            mine.sort_by_key(|(subject, order, _)| (*subject, *order));
            found.extend(mine.into_iter().map(|(_, _, kind)| kind));
        }
        self.inside = now;
        self.held = held;
        let tick = self.tick;
        for kind in found {
            self.push_event(tick, kind);
        }
    }

    /// Takes over a changed world between two ticks. The tick is kept. A character that exists
    /// in both keeps its body and its input, a new one gets a body at rest, a removed one loses
    /// both; positions come from the new world (so a character can be moved by hand). The
    /// height field is rebuilt only when the terrain changed. Projectiles fly on; a launcher
    /// and a behaviour that exist in both keep their state, new ones start fresh (a full
    /// magazine; the base state, with the entity's place as its post); a behaviour whose route
    /// changed walks to the new route's first point next. On an error nothing changes.
    pub fn replace_world(&mut self, world: World) -> Result<(), SimError> {
        world
            .validate()
            .map_err(|e| SimError::World(e.to_string()))?;
        let static_hash = static_hash(&world)?;
        // `None`: keep the current height field (the terrain did not change).
        let rebuilt = match &world.terrain {
            Some(terrain) if self.world.terrain.as_ref() == Some(terrain) => None,
            Some(terrain) => Some(Some(Arc::new(
                terrain
                    .build()
                    .map_err(|e| SimError::World(e.to_string()))?,
            ))),
            None => Some(None),
        };
        let mut bodies = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        for entity in world.entities() {
            if entity.character().is_some() {
                let body = self.bodies.get(&entity.id).copied().unwrap_or_default();
                bodies.insert(entity.id, body);
                if let Some(input) = self.inputs.get(&entity.id) {
                    inputs.insert(entity.id, *input);
                }
            }
        }
        let mut magazines = BTreeMap::new();
        let mut brains = BTreeMap::new();
        for entity in world.entities() {
            if let Some(launcher) = entity.launcher() {
                let mut magazine = self
                    .magazines
                    .get(&entity.id)
                    .copied()
                    .unwrap_or_else(|| Magazine::full(launcher));
                magazine.left = magazine.left.min(launcher.magazine);
                magazines.insert(entity.id, magazine);
            }
            if let (Some(behaviour), Some(_)) = (entity.behaviour(), entity.character()) {
                let mut brain = self
                    .brains
                    .get(&entity.id)
                    .copied()
                    .unwrap_or_else(|| Brain::new(entity, behaviour));
                let old_route = self
                    .world
                    .entity(entity.id)
                    .and_then(Entity::behaviour)
                    .map(|b| b.route.as_slice());
                if old_route.is_some_and(|r| r != behaviour.route.as_slice()) {
                    // A new route starts at its first point.
                    brain.waypoint = 0;
                }
                brain.adapt(behaviour);
                brains.insert(entity.id, brain);
            }
        }
        self.taken_over.retain(|id| brains.contains_key(id));
        self.sounds.retain(|(id, _)| brains.contains_key(id));
        self.magazines = magazines;
        self.brains = brains;
        self.solids = solid::solids_of(&world);
        self.watch = events::Watch::of(&world);
        let zone_ids: BTreeSet<EntityId> = self.watch.zones.iter().copied().collect();
        self.inside
            .retain(|(who, zone)| bodies.contains_key(who) && zone_ids.contains(zone));
        self.held.retain(|who| bodies.contains_key(who));
        self.world = world;
        if let Some(heights) = rebuilt {
            self.heights = heights;
        }
        self.bodies = bodies;
        self.inputs = inputs;
        self.static_hash = static_hash;
        Ok(())
    }

    /// A hash of everything that moves or decides what moves next: the tick, the running world,
    /// every character's body and input, in id order, and (when there are any) which
    /// characters are inside which zones and which hold `interact`, the inputs' held facings;
    /// and, when there are any,
    /// the projectiles in flight, the launchers' magazines, every health, the behaviours'
    /// states, the inputs' aim and fire, the agents' take-overs and the shots heard but not yet
    /// acted on. Events waiting to be taken are not part of it. Equal histories give equal
    /// hashes on every platform.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        hasher.write_u64(self.tick);
        hasher.write_u64(self.static_hash);
        for (id, body) in &self.bodies {
            hasher.write_u64(id.0);
            if let Some(entity) = self.world.entity(*id) {
                for v in entity.position {
                    hasher.write_f64(v);
                }
                hasher.write_f64(entity.rotation_y_deg);
            }
            for v in body.velocity {
                hasher.write_f64(v);
            }
            hasher.write_bool(body.grounded);
            match self.inputs.get(id) {
                Some(input) => {
                    hasher.write_bool(true);
                    hasher.write_f64(input.forward);
                    hasher.write_f64(input.right);
                    hasher.write_f64(input.yaw_deg);
                    hasher.write_bool(input.run);
                    hasher.write_bool(input.jump);
                }
                None => hasher.write_bool(false),
            }
        }
        // Written only when present, so a world without scenario state hashes as before.
        if !self.inside.is_empty() {
            hasher.write_u64(HASH_MARK_INSIDE);
            for (who, zone) in &self.inside {
                hasher.write_u64(who.0);
                hasher.write_u64(zone.0);
            }
        }
        let pressing: Vec<EntityId> = self
            .inputs
            .iter()
            .filter(|(_, i)| i.interact)
            .map(|(id, _)| *id)
            .collect();
        if !pressing.is_empty() || !self.held.is_empty() {
            hasher.write_u64(HASH_MARK_HELD);
            for id in &pressing {
                hasher.write_u64(id.0);
            }
            hasher.write_u64(u64::MAX);
            for id in &self.held {
                hasher.write_u64(id.0);
            }
        }
        let facing: Vec<(EntityId, Face)> = self
            .inputs
            .iter()
            .filter_map(|(id, i)| i.face.map(|f| (*id, f)))
            .collect();
        if !facing.is_empty() {
            hasher.write_u64(HASH_MARK_FACE);
            for (id, face) in facing {
                hasher.write_u64(id.0);
                match face {
                    Face::Point([x, z]) => {
                        hasher.write_u64(0);
                        hasher.write_f64(x);
                        hasher.write_f64(z);
                    }
                    Face::Entity { id: target, pitch } => {
                        hasher.write_u64(1);
                        hasher.write_u64(target.0);
                        hasher.write_bool(pitch);
                    }
                }
            }
        }
        self.hash_combat(&mut hasher);
        hasher.finish()
    }

    /// The combat part of [`Sim::state_hash`], written only when there is any, so a world
    /// without it hashes as before.
    fn hash_combat(&self, hasher: &mut StateHasher) {
        let healths: Vec<(EntityId, f64)> = self
            .world
            .entities()
            .filter_map(|e| e.health().map(|h| (e.id, h.current)))
            .collect();
        let aims: Vec<(EntityId, &Input)> = self
            .inputs
            .iter()
            .filter(|(_, i)| i.fire || i.pitch_deg != 0.0)
            .map(|(id, i)| (*id, i))
            .collect();
        let any = !self.projectiles.is_empty()
            || !self.magazines.is_empty()
            || !healths.is_empty()
            || !self.brains.is_empty()
            || !aims.is_empty()
            || !self.taken_over.is_empty()
            || !self.sounds.is_empty()
            || self.next_projectile != 1;
        if !any {
            return;
        }
        hasher.write_u64(HASH_MARK_COMBAT);
        hasher.write_u64(self.next_projectile);
        hasher.write_u64(self.projectiles.len() as u64);
        for p in &self.projectiles {
            hasher.write_u64(p.id);
            hasher.write_u64(p.shooter.0);
            for v in p.position.iter().chain(&p.velocity) {
                hasher.write_f64(*v);
            }
            for v in [
                p.age_s,
                p.mass_kg,
                p.ballistic_coefficient,
                p.damage_per_joule,
                p.max_flight_s,
            ] {
                hasher.write_f64(v);
            }
        }
        hasher.write_u64(self.magazines.len() as u64);
        for (id, m) in &self.magazines {
            hasher.write_u64(id.0);
            hasher.write_u64(u64::from(m.left));
            hasher.write_u64(m.shots);
            hasher.write_f64(m.ready_in_s);
            hasher.write_bool(m.reloading);
        }
        hasher.write_u64(healths.len() as u64);
        for (id, current) in healths {
            hasher.write_u64(id.0);
            hasher.write_f64(current);
        }
        hasher.write_u64(self.brains.len() as u64);
        for (id, b) in &self.brains {
            hasher.write_u64(id.0);
            hasher.write_str(b.state.name());
            match b.state {
                BrainState::Alert { target, wait_s } => {
                    hasher.write_u64(target.0);
                    hasher.write_f64(wait_s);
                }
                BrainState::Engage { target } => hasher.write_u64(target.0),
                BrainState::Search { point, left_s } => {
                    for v in point {
                        hasher.write_f64(v);
                    }
                    hasher.write_f64(left_s);
                }
                BrainState::Idle | BrainState::Patrol | BrainState::Guard | BrainState::Return => {}
            }
            hasher.write_u64(b.waypoint as u64);
            for v in b.post {
                hasher.write_f64(v);
            }
            hasher.write_f64(b.post_yaw_deg);
            match b.last_seen {
                Some(p) => {
                    hasher.write_bool(true);
                    for v in p {
                        hasher.write_f64(v);
                    }
                }
                None => hasher.write_bool(false),
            }
        }
        hasher.write_u64(aims.len() as u64);
        for (id, i) in aims {
            hasher.write_u64(id.0);
            hasher.write_f64(i.pitch_deg);
            hasher.write_bool(i.fire);
        }
        hasher.write_u64(self.taken_over.len() as u64);
        for id in &self.taken_over {
            hasher.write_u64(id.0);
        }
        hasher.write_u64(self.sounds.len() as u64);
        for (id, p) in &self.sounds {
            hasher.write_u64(id.0);
            for v in p {
                hasher.write_f64(*v);
            }
        }
    }
}

/// The hash of a world's static content: its JSON text with every character's position and
/// rotation set to zero, since those move and are hashed per tick by [`Sim::state_hash`].
fn static_hash(world: &World) -> Result<u64, SimError> {
    let mut still = world.clone();
    let characters: Vec<EntityId> = world
        .entities()
        .filter(|e| e.character().is_some())
        .map(|e| e.id)
        .collect();
    for id in characters {
        if let Some(entity) = still.entity_mut(id) {
            entity.position = [0.0; 3];
            entity.rotation_y_deg = 0.0;
        }
    }
    let text = still
        .to_json()
        .map_err(|e| SimError::World(e.to_string()))?;
    let mut hasher = StateHasher::new();
    hasher.write_str(&text);
    Ok(hasher.finish())
}
