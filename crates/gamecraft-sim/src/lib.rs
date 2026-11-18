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
//! No GPU, no files, no clock, no threads.
//!
//! # The model (M0: characters on terrain among solid shapes)
//!
//! Every tick steps the characters one by one in id order:
//!
//! 1. **Facing.** With an input record the entity's `rotation_y_deg` becomes the input's
//!    (wrapped) `yaw_deg`. Without one the character keeps its rotation and wants no motion.
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

use std::collections::BTreeMap;
use std::fmt;

use gamecraft_math::{StateHasher, wrap_deg};
use gamecraft_world::{EntityId, HeightField, World};
use serde::{Deserialize, Serialize};

mod motion;
mod solid;

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
        }
    }
}

impl std::error::Error for SimError {}

/// A running world.
#[derive(Clone, Debug)]
pub struct Sim {
    world: World,
    heights: Option<HeightField>,
    tick: u64,
    bodies: BTreeMap<EntityId, Body>,
    inputs: BTreeMap<EntityId, Input>,
    /// The solid shapes of `world`, derived by [`Sim::replace_world`].
    solids: Vec<Solid>,
    /// The hash of `world` without its characters' positions and rotations.
    static_hash: u64,
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
        self.heights.as_ref()
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
    /// clamped to the unit disc and `yaw_deg` is wrapped; a non-finite field is an error.
    pub fn set_input(&mut self, id: EntityId, input: Input) -> Result<(), SimError> {
        let entity = self.world.entity(id).ok_or(SimError::NoEntity(id))?;
        if entity.character().is_none() {
            return Err(SimError::NotACharacter(id));
        }
        for (field, value) in [
            ("forward", input.forward),
            ("right", input.right),
            ("yaw_deg", input.yaw_deg),
        ] {
            if !value.is_finite() {
                return Err(SimError::BadInput(format!(
                    "`{field}` of the input for entity {id} is {value}; it must be a finite \
                     number ({})",
                    if field == "yaw_deg" {
                        "degrees, any value; it is wrapped into 0 to 360"
                    } else {
                        "-1 to 1; longer (forward, right) pairs are scaled onto the unit disc"
                    }
                )));
            }
        }
        let (forward, right) = clamp_to_disc(input.forward, input.right);
        self.inputs.insert(
            id,
            Input {
                forward,
                right,
                yaw_deg: wrap_deg(input.yaw_deg),
                ..input
            },
        );
        Ok(())
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
            let around = Surroundings {
                heights: self.heights.as_ref(),
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
                let next = around.step(state, character, self.inputs.get(&id));
                if let Some(entity) = self.world.entity_mut(id) {
                    entity.position = next.position;
                    entity.rotation_y_deg = next.rotation_y_deg;
                }
                self.bodies.insert(id, next.body);
            }
            self.tick = self.tick.saturating_add(1);
        }
    }

    /// Takes over a changed world between two ticks. The tick is kept. A character that exists
    /// in both keeps its body and its input, a new one gets a body at rest, a removed one loses
    /// both; positions come from the new world (so a character can be moved by hand). The
    /// height field is rebuilt only when the terrain changed. On an error nothing changes.
    pub fn replace_world(&mut self, world: World) -> Result<(), SimError> {
        world
            .validate()
            .map_err(|e| SimError::World(e.to_string()))?;
        let static_hash = static_hash(&world)?;
        // `None`: keep the current height field (the terrain did not change).
        let rebuilt = match &world.terrain {
            Some(terrain) if self.world.terrain.as_ref() == Some(terrain) => None,
            Some(terrain) => Some(Some(
                terrain
                    .build()
                    .map_err(|e| SimError::World(e.to_string()))?,
            )),
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
        self.solids = solid::solids_of(&world);
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
    /// and every character's body and input, in id order. Equal histories give equal hashes on
    /// every platform.
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
        hasher.finish()
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
