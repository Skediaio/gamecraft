//! Components: the typed pieces of data an entity carries.

use crate::WorldError;
use crate::combat::{Behaviour, Health, Launcher};
use crate::error::Ctx;
use crate::limits::{
    INTERACT_REACH_MAX_M, LIGHT_INTENSITY_MAX_CD, LIGHT_RANGE_MAX_M, MAX_NAME_CHARS,
    MAX_SLOPE_MAX_DEG, MAX_SLOPE_MIN_DEG, MAX_TAGS, MAX_TEXT_CHARS, SHAPE_SIZE_MAX_M,
    SHAPE_SIZE_MIN_M,
};
use serde::{Deserialize, Serialize};

/// One component of an entity. An entity has at most one component of each type.
///
/// In JSON a component is an object whose `type` field names the variant
/// (`{"type": "shape", ...}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Component {
    /// A simple solid: box, sphere or cylinder.
    Shape(Shape),
    /// A plant grown by the plant library.
    Plant(Plant),
    /// A character that can walk, run and jump.
    Character(Character),
    /// A volume that tells when characters enter and leave it.
    Zone(Zone),
    /// Something a character can interact with.
    Interactable(Interactable),
    /// A point light.
    Light(Light),
    /// Free labels that triggers and queries filter by.
    Tags(Tags),
    /// Something that fires projectiles.
    Launcher(Launcher),
    /// What hits take away.
    Health(Health),
    /// A small state machine that patrols, reacts and fires.
    Behaviour(Behaviour),
}

impl Component {
    /// Every component type name, in the order the engine lists them.
    pub const KINDS: [&'static str; 10] = [
        "shape",
        "plant",
        "character",
        "zone",
        "interactable",
        "light",
        "tags",
        "launcher",
        "health",
        "behaviour",
    ];

    /// The type name of this component (`shape`, `plant`, `character`, `zone`,
    /// `interactable`, `light`, `tags`, `launcher`, `health` or `behaviour`), as in JSON.
    pub fn kind(&self) -> &'static str {
        match self {
            Component::Shape(_) => "shape",
            Component::Plant(_) => "plant",
            Component::Character(_) => "character",
            Component::Zone(_) => "zone",
            Component::Interactable(_) => "interactable",
            Component::Light(_) => "light",
            Component::Tags(_) => "tags",
            Component::Launcher(_) => "launcher",
            Component::Health(_) => "health",
            Component::Behaviour(_) => "behaviour",
        }
    }

    /// Checks the component's own rules; `ctx` names the entity in errors.
    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        match self {
            Component::Shape(s) => s.validate(ctx),
            Component::Plant(p) => p.validate(ctx),
            Component::Character(c) => c.validate(ctx),
            Component::Zone(z) => z.validate(ctx),
            Component::Interactable(i) => i.validate(ctx),
            Component::Light(l) => l.validate(ctx),
            Component::Tags(t) => t.validate(ctx),
            Component::Launcher(l) => l.validate(ctx),
            Component::Health(h) => h.validate(ctx),
            Component::Behaviour(b) => b.validate(ctx),
        }
    }
}

/// The geometric kind of a [`Shape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    /// An axis-aligned box (before the entity's rotation) of extent `size`.
    Box,
    /// A sphere; `size` holds its diameter three times.
    Sphere,
    /// An upright cylinder; `size` is `[diameter, height, diameter]`.
    Cylinder,
}

impl ShapeKind {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Box => "box",
            ShapeKind::Sphere => "sphere",
            ShapeKind::Cylinder => "cylinder",
        }
    }
}

fn default_shape_size() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}

fn default_shape_color() -> [f32; 3] {
    [0.8, 0.8, 0.8]
}

fn default_true() -> bool {
    true
}

/// A simple solid standing on the entity's position (its base is the bottom centre).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    /// Box, sphere or cylinder.
    pub kind: ShapeKind,
    /// Bounding extent `[x, y, z]` in metres, before the entity's scale. A sphere needs all
    /// three equal (its diameter); a cylinder needs x == z (its diameter) and y is its height.
    /// Default `[1, 1, 1]`.
    #[serde(default = "default_shape_size")]
    pub size: [f64; 3],
    /// Linear RGB, each channel 0 to 1. Default light grey `[0.8, 0.8, 0.8]`.
    #[serde(default = "default_shape_color")]
    pub color: [f32; 3],
    /// Whether it collides. Default true.
    #[serde(default = "default_true")]
    pub solid: bool,
}

impl Shape {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        for (axis, &v) in self.size.iter().enumerate() {
            ctx.range(
                format!("shape.size[{axis}]"),
                v,
                SHAPE_SIZE_MIN_M,
                SHAPE_SIZE_MAX_M,
                " metres",
            )?;
        }
        let [x, y, z] = self.size;
        match self.kind {
            ShapeKind::Box => {}
            ShapeKind::Sphere => {
                if x != y || y != z {
                    return Err(ctx.invalid(
                        "shape.size",
                        format!("[{x}, {y}, {z}]"),
                        "a sphere needs all three sizes equal (its diameter), \
                         for example [d, d, d]",
                    ));
                }
            }
            ShapeKind::Cylinder => {
                if x != z {
                    return Err(ctx.invalid(
                        "shape.size",
                        format!("[{x}, {y}, {z}]"),
                        "a cylinder needs size[0] == size[2] (its diameter) and size[1] is \
                         its height, for example [d, h, d]",
                    ));
                }
            }
        }
        for (channel, &c) in self.color.iter().enumerate() {
            ctx.range(
                format!("shape.color[{channel}]"),
                f64::from(c),
                0.0,
                1.0,
                " (linear RGB)",
            )?;
        }
        Ok(())
    }
}

fn default_growth() -> f64 {
    1.0
}

/// A plant, grown deterministically by the plant library from its species and seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plant {
    /// An id of the plant library (`quercus-robur`); not checked against the library here.
    pub species: String,
    /// Seed of this individual's shape. Default 0.
    #[serde(default)]
    pub seed: u64,
    /// Growth stage in (0, 1]; 1 is mature. Default 1.
    #[serde(default = "default_growth")]
    pub growth: f64,
}

impl Plant {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if self.species.trim().is_empty() {
            return Err(ctx.invalid(
                "plant.species",
                "empty",
                "it must name a species of the plant library, such as \"quercus-robur\"",
            ));
        }
        ctx.name("plant.species", &self.species)?;
        if !(self.growth.is_finite() && self.growth > 0.0 && self.growth <= 1.0) {
            return Err(ctx.invalid(
                "plant.growth",
                self.growth,
                "it must be above 0 and at most 1 (1 is mature)",
            ));
        }
        Ok(())
    }
}

/// A character: a capsule that walks, runs and jumps, driven by input records.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Character {
    /// Standing height, metres. Default 1.8.
    pub height_m: f64,
    /// Capsule radius, metres; at most half the height. Default 0.3.
    pub radius_m: f64,
    /// Eye height above the feet, metres; at most the height. Default 1.65.
    pub eye_height_m: f64,
    /// Walking speed, metres per second. Default 1.6.
    pub walk_speed_mps: f64,
    /// Running speed, metres per second. Default 5.0.
    pub run_speed_mps: f64,
    /// Upward speed at the start of a jump, metres per second. Default 4.5.
    pub jump_speed_mps: f64,
    /// Steepest walkable slope, degrees, 1 to 89. Default 45.
    pub max_slope_deg: f64,
}

impl Default for Character {
    fn default() -> Self {
        Character {
            height_m: 1.8,
            radius_m: 0.3,
            eye_height_m: 1.65,
            walk_speed_mps: 1.6,
            run_speed_mps: 5.0,
            jump_speed_mps: 4.5,
            max_slope_deg: 45.0,
        }
    }
}

impl Character {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        ctx.positive("character.height_m", self.height_m, " metres")?;
        ctx.positive("character.radius_m", self.radius_m, " metres")?;
        ctx.positive("character.eye_height_m", self.eye_height_m, " metres")?;
        ctx.positive(
            "character.walk_speed_mps",
            self.walk_speed_mps,
            " metres per second",
        )?;
        ctx.positive(
            "character.run_speed_mps",
            self.run_speed_mps,
            " metres per second",
        )?;
        ctx.positive(
            "character.jump_speed_mps",
            self.jump_speed_mps,
            " metres per second",
        )?;
        ctx.range(
            "character.max_slope_deg",
            self.max_slope_deg,
            MAX_SLOPE_MIN_DEG,
            MAX_SLOPE_MAX_DEG,
            " degrees",
        )?;
        if self.radius_m * 2.0 > self.height_m {
            return Err(ctx.invalid(
                "character.radius_m",
                self.radius_m,
                format!(
                    "it must be at most half of character.height_m {} (at most {})",
                    self.height_m,
                    self.height_m / 2.0
                ),
            ));
        }
        if self.eye_height_m > self.height_m {
            return Err(ctx.invalid(
                "character.eye_height_m",
                self.eye_height_m,
                format!("it must be at most character.height_m {}", self.height_m),
            ));
        }
        Ok(())
    }
}

/// The footprint of a [`Zone`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneShape {
    /// An upright cylinder of radius `radius_m`.
    #[default]
    Circle,
    /// A box of `size` `[x, z]`, turned by the entity's yaw.
    Box,
}

impl ZoneShape {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ZoneShape::Circle => "circle",
            ZoneShape::Box => "box",
        }
    }
}

/// A volume standing on the entity's position: an upright cylinder, or a box turned by the
/// entity's `rotation_y_deg`, `height_m` tall. Its sizes are multiplied by the entity's scale.
/// A character whose body (feet to feet + height) overlaps it vertically and whose position
/// lies in its footprint is inside; entering and leaving produce `enter` and `leave` events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Zone {
    /// `circle` or `box`. Default `circle`.
    pub shape: ZoneShape,
    /// Radius of a circle, metres (ignored by a box). Default 2.
    pub radius_m: f64,
    /// Extent `[x, z]` of a box, metres (ignored by a circle). Default `[4, 4]`.
    pub size: [f64; 2],
    /// Height above the entity's position, metres. Default 3.
    pub height_m: f64,
}

impl Default for Zone {
    fn default() -> Self {
        Zone {
            shape: ZoneShape::Circle,
            radius_m: 2.0,
            size: [4.0, 4.0],
            height_m: 3.0,
        }
    }
}

impl Zone {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        ctx.range(
            "zone.radius_m",
            self.radius_m,
            SHAPE_SIZE_MIN_M,
            SHAPE_SIZE_MAX_M,
            " metres",
        )?;
        for (axis, &v) in self.size.iter().enumerate() {
            ctx.range(
                format!("zone.size[{axis}]"),
                v,
                SHAPE_SIZE_MIN_M,
                SHAPE_SIZE_MAX_M,
                " metres",
            )?;
        }
        ctx.range(
            "zone.height_m",
            self.height_m,
            SHAPE_SIZE_MIN_M,
            SHAPE_SIZE_MAX_M,
            " metres",
        )
    }
}

/// Something a character can interact with: a character whose input asks to interact, whose
/// eye is within `reach_m` of it and who faces it (within 45 degrees) produces an `interact`
/// event for the nearest such entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Interactable {
    /// How far from the character's eye it can be reached, metres, above 0 and at most 100.
    /// Default 2.
    pub reach_m: f64,
    /// What a player is told they can do (`Light the beacon`). Default empty.
    pub prompt: String,
}

impl Default for Interactable {
    fn default() -> Self {
        Interactable {
            reach_m: 2.0,
            prompt: String::new(),
        }
    }
}

impl Interactable {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if !(self.reach_m.is_finite() && self.reach_m > 0.0 && self.reach_m <= INTERACT_REACH_MAX_M)
        {
            return Err(ctx.invalid(
                "interactable.reach_m",
                self.reach_m,
                format!("it must be above 0 and at most {INTERACT_REACH_MAX_M} metres"),
            ));
        }
        ctx.text("interactable.prompt", &self.prompt, MAX_TEXT_CHARS)
    }
}

fn default_light_color() -> [f32; 3] {
    [1.0, 0.85, 0.6]
}

/// A point light at the entity's position; on a character it moves with it. The renderer draws
/// it; it does not change the simulation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Light {
    /// Linear RGB, each channel 0 to 1. Default warm white `[1, 0.85, 0.6]`.
    pub color: [f32; 3],
    /// Luminous intensity, candela, 0 to 1e7. Default 100.
    pub intensity_cd: f64,
    /// Distance beyond which it lights nothing, metres, above 0 and at most 10000. Default 10.
    pub range_m: f64,
    /// Whether it shines. Default true.
    pub on: bool,
}

impl Default for Light {
    fn default() -> Self {
        Light {
            color: default_light_color(),
            intensity_cd: 100.0,
            range_m: 10.0,
            on: true,
        }
    }
}

impl Light {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        for (channel, &c) in self.color.iter().enumerate() {
            ctx.range(
                format!("light.color[{channel}]"),
                f64::from(c),
                0.0,
                1.0,
                " (linear RGB)",
            )?;
        }
        ctx.range(
            "light.intensity_cd",
            self.intensity_cd,
            0.0,
            LIGHT_INTENSITY_MAX_CD,
            " candela",
        )?;
        if !(self.range_m.is_finite() && self.range_m > 0.0 && self.range_m <= LIGHT_RANGE_MAX_M) {
            return Err(ctx.invalid(
                "light.range_m",
                self.range_m,
                format!("it must be above 0 and at most {LIGHT_RANGE_MAX_M} metres"),
            ));
        }
        Ok(())
    }
}

/// Free labels (`beacon`, `team:red`) that triggers (`"who": "tag:beacon"`) and queries
/// (`entity.find`) filter by. Matched exactly, case included.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tags {
    /// The labels, each used once, at most 64, each at most 200 characters. Default none.
    pub tags: Vec<String>,
}

impl Tags {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if self.tags.len() > MAX_TAGS {
            return Err(ctx.invalid(
                "tags.tags",
                format!("{} tags", self.tags.len()),
                format!("an entity has at most {MAX_TAGS} tags"),
            ));
        }
        for (i, tag) in self.tags.iter().enumerate() {
            let field = format!("tags.tags[{i}]");
            if tag.trim().is_empty() {
                return Err(ctx.invalid(
                    field,
                    "empty",
                    "a tag must hold at least one character other than a space",
                ));
            }
            ctx.text(field.clone(), tag, MAX_NAME_CHARS)?;
            if self.tags.iter().take(i).any(|t| t == tag) {
                return Err(ctx.invalid(
                    field,
                    format!("{tag:?}"),
                    "a tag is listed more than once; list each tag once",
                ));
            }
        }
        Ok(())
    }
}
