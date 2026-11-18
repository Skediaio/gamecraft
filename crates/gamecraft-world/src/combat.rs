//! The components of fighting: `launcher` (something that fires projectiles), `health` (what
//! a hit takes away) and `behaviour` (a small state machine that patrols, reacts and fires).
//! They name no weapon or soldier: a pack fills them in for a rifle, a bow or a mortar
//! (`docs/design/ballistics.md`).

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    AIM_ERROR_MAX_MRAD, BALLISTIC_COEFFICIENT_MAX, BEHAVIOUR_RANGE_MAX_M, BEHAVIOUR_TIME_MAX_S,
    DAMAGE_PER_JOULE_MAX, DISPERSION_MAX_MRAD, HEALTH_MAX, LAUNCHER_TIME_MAX_S, MAGAZINE_MAX,
    MAX_COORDINATE_M, MAX_FLIGHT_MAX_S, MAX_NAME_CHARS, MAX_ROUTE_POINTS, MUZZLE_VELOCITY_MAX_MPS,
    PROJECTILE_MASS_MAX_KG, SHAPE_SIZE_MAX_M, SOUND_RANGE_MAX_M,
};
use serde::{Deserialize, Serialize};

/// `value` must be finite, above 0 and at most `max`.
fn above_zero(ctx: Ctx, field: &str, value: f64, max: f64, unit: &str) -> Result<(), WorldError> {
    if value.is_finite() && value > 0.0 && value <= max {
        Ok(())
    } else {
        Err(ctx.invalid(
            field,
            value,
            format!("it must be a finite number above 0 and at most {max}{unit}"),
        ))
    }
}

/// Something that fires projectiles: a rifle, a bow, a thrown stone, a mortar. On a character
/// it fires while the input holds `fire`; on anything else with `sim.fire`.
///
/// The projectile is a point mass flown by the simulation with gravity and drag; its drag is the
/// G7 reference drag function scaled by `ballistic_coefficient`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Launcher {
    /// Speed of the projectile leaving the muzzle, metres per second, above 0 and at most
    /// 3000. Default 800.
    pub muzzle_velocity_mps: f64,
    /// Mass of one projectile, kilograms, above 0 and at most 1000. Default 0.01.
    pub mass_kg: f64,
    /// G7 ballistic coefficient in kilograms per square metre: the mass over (the form factor
    /// times the reference area, pi d^2 / 4). From the usual G7 figure in lb/in^2 multiply by
    /// 703.07 * 4 / pi = 895.2. Above 0 and at most 100000. Default 200.
    pub ballistic_coefficient: f64,
    /// Spread of the shots, milliradians (the radius of a uniform cone), 0 to 500. Default 0.5.
    pub dispersion_mrad: f64,
    /// Seconds between two shots while firing is held, above 0 and at most 3600. Default 0.1.
    pub rate_s: f64,
    /// Shots before a reload, 1 to 100000. Default 10.
    pub magazine: u32,
    /// Seconds a reload takes once the magazine is empty, 0 to 3600. Default 2.5.
    pub reload_s: f64,
    /// Damage per joule of the projectile's kinetic energy at the hit, 0 to 1e6. Default 0.05.
    pub damage_per_joule: f64,
    /// Height of the muzzle above the entity's position, metres, 0 to 10000. A character fires
    /// from its eyes (`eye_height_m`) instead. Default 1.5.
    pub muzzle_height_m: f64,
    /// Longest flight of a projectile, seconds, above 0 and at most 600; it ends without a hit
    /// after that. Default 10.
    pub max_flight_s: f64,
    /// Distance from the muzzle within which characters hear a shot, metres, 0 to 100000.
    /// Default 500.
    pub sound_range_m: f64,
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher {
            muzzle_velocity_mps: 800.0,
            mass_kg: 0.01,
            ballistic_coefficient: 200.0,
            dispersion_mrad: 0.5,
            rate_s: 0.1,
            magazine: 10,
            reload_s: 2.5,
            damage_per_joule: 0.05,
            muzzle_height_m: 1.5,
            max_flight_s: 10.0,
            sound_range_m: 500.0,
        }
    }
}

impl Launcher {
    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        above_zero(
            ctx,
            "launcher.muzzle_velocity_mps",
            self.muzzle_velocity_mps,
            MUZZLE_VELOCITY_MAX_MPS,
            " metres per second",
        )?;
        above_zero(
            ctx,
            "launcher.mass_kg",
            self.mass_kg,
            PROJECTILE_MASS_MAX_KG,
            " kilograms",
        )?;
        above_zero(
            ctx,
            "launcher.ballistic_coefficient",
            self.ballistic_coefficient,
            BALLISTIC_COEFFICIENT_MAX,
            " kilograms per square metre",
        )?;
        ctx.range(
            "launcher.dispersion_mrad",
            self.dispersion_mrad,
            0.0,
            DISPERSION_MAX_MRAD,
            " milliradians",
        )?;
        above_zero(
            ctx,
            "launcher.rate_s",
            self.rate_s,
            LAUNCHER_TIME_MAX_S,
            " seconds",
        )?;
        if !(1..=MAGAZINE_MAX).contains(&self.magazine) {
            return Err(ctx.invalid(
                "launcher.magazine",
                self.magazine,
                format!("it must be from 1 to {MAGAZINE_MAX}"),
            ));
        }
        ctx.range(
            "launcher.reload_s",
            self.reload_s,
            0.0,
            LAUNCHER_TIME_MAX_S,
            " seconds",
        )?;
        ctx.range(
            "launcher.damage_per_joule",
            self.damage_per_joule,
            0.0,
            DAMAGE_PER_JOULE_MAX,
            "",
        )?;
        ctx.range(
            "launcher.muzzle_height_m",
            self.muzzle_height_m,
            0.0,
            SHAPE_SIZE_MAX_M,
            " metres",
        )?;
        above_zero(
            ctx,
            "launcher.max_flight_s",
            self.max_flight_s,
            MAX_FLIGHT_MAX_S,
            " seconds",
        )?;
        ctx.range(
            "launcher.sound_range_m",
            self.sound_range_m,
            0.0,
            SOUND_RANGE_MAX_M,
            " metres",
        )
    }
}

/// What hits take away. A hit takes `energy_j * damage_per_joule - armor` (at least 0) from
/// `current`; at 0 a character is down: it takes no input and stops.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Health {
    /// Full health, above 0 and at most 1e9. Default 100.
    pub max: f64,
    /// Health now, 0 to `max`. Default 100.
    pub current: f64,
    /// Damage every hit loses before it counts, 0 to 1e9. Default 0.
    pub armor: f64,
}

impl Default for Health {
    fn default() -> Self {
        Health {
            max: 100.0,
            current: 100.0,
            armor: 0.0,
        }
    }
}

impl Health {
    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        above_zero(ctx, "health.max", self.max, HEALTH_MAX, "")?;
        if !(self.current.is_finite() && self.current >= 0.0 && self.current <= self.max) {
            return Err(ctx.invalid(
                "health.current",
                self.current,
                format!(
                    "it must be a finite number from 0 to health.max {}",
                    self.max
                ),
            ));
        }
        ctx.range("health.armor", self.armor, 0.0, HEALTH_MAX, "")
    }

    /// Whether this health is used up (a character with it is down).
    pub fn is_down(&self) -> bool {
        self.current <= 0.0
    }
}

/// What a [`Behaviour`] does when nothing has caught its attention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BehaviourMode {
    /// Stands where it is.
    #[default]
    Idle,
    /// Walks its route, point after point, and starts again at the first.
    Patrol,
    /// Stands at its post (where it was when the simulation started) and returns to it.
    Guard,
}

impl BehaviourMode {
    /// Every mode name, as in JSON.
    pub const NAMES: [&'static str; 3] = ["idle", "patrol", "guard"];

    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            BehaviourMode::Idle => "idle",
            BehaviourMode::Patrol => "patrol",
            BehaviourMode::Guard => "guard",
        }
    }
}

/// A small state machine the simulation runs for a character: it patrols, guards or idles;
/// on seeing a character of another team it turns, waits its reaction time and fires at it
/// with its aim error; on hearing a shot it searches towards it; and it returns when it has
/// seen nothing for `search_s`. It decides through the same input records a player uses, so
/// `sim.input` takes it over (until `sim.release`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Behaviour {
    /// `idle`, `patrol` or `guard`. Default `idle`.
    pub mode: BehaviourMode,
    /// The patrol's points `[x, z]`, at most 256; a patrol needs at least one. Default none.
    pub route: Vec<[f64; 2]>,
    /// Its side. Characters of another, non-empty team are enemies; at most 200 characters.
    /// A character without a behaviour takes its team from a tag `team:<name>`. Default empty
    /// (no side: it fights nobody and nobody fights it).
    pub team: String,
    /// How far it sees in daylight and clear air, metres, above 0 and at most 10000. Default
    /// 300.
    pub view_range_m: f64,
    /// Its field of view, degrees, above 0 and at most 360. Default 120.
    pub fov_deg: f64,
    /// How far away it fires at an enemy it sees, metres, above 0 and at most 10000; farther
    /// it walks towards it. Default 200.
    pub engage_range_m: f64,
    /// Its aim error, milliradians (the radius of a uniform cone), 0 to 1000. Default 3.
    pub aim_error_mrad: f64,
    /// Seconds between first seeing an enemy and firing, 0 to 3600. Default 0.6.
    pub reaction_s: f64,
    /// Seconds it searches after losing sight of an enemy or hearing a shot, 0 to 3600.
    /// Default 15.
    pub search_s: f64,
}

impl Default for Behaviour {
    fn default() -> Self {
        Behaviour {
            mode: BehaviourMode::Idle,
            route: Vec::new(),
            team: String::new(),
            view_range_m: 300.0,
            fov_deg: 120.0,
            engage_range_m: 200.0,
            aim_error_mrad: 3.0,
            reaction_s: 0.6,
            search_s: 15.0,
        }
    }
}

impl Behaviour {
    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        if self.route.len() > MAX_ROUTE_POINTS {
            return Err(ctx.invalid(
                "behaviour.route",
                format!("{} points", self.route.len()),
                format!("a route has at most {MAX_ROUTE_POINTS} points"),
            ));
        }
        if self.mode == BehaviourMode::Patrol && self.route.is_empty() {
            return Err(ctx.invalid(
                "behaviour.route",
                "empty",
                "a patrol needs at least one point [x, z]; give a route or use mode \"guard\"",
            ));
        }
        for (i, point) in self.route.iter().enumerate() {
            for (axis, &v) in point.iter().enumerate() {
                ctx.range(
                    format!("behaviour.route[{i}][{axis}]"),
                    v,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )?;
            }
        }
        ctx.text("behaviour.team", &self.team, MAX_NAME_CHARS)?;
        above_zero(
            ctx,
            "behaviour.view_range_m",
            self.view_range_m,
            BEHAVIOUR_RANGE_MAX_M,
            " metres",
        )?;
        above_zero(ctx, "behaviour.fov_deg", self.fov_deg, 360.0, " degrees")?;
        above_zero(
            ctx,
            "behaviour.engage_range_m",
            self.engage_range_m,
            BEHAVIOUR_RANGE_MAX_M,
            " metres",
        )?;
        ctx.range(
            "behaviour.aim_error_mrad",
            self.aim_error_mrad,
            0.0,
            AIM_ERROR_MAX_MRAD,
            " milliradians",
        )?;
        ctx.range(
            "behaviour.reaction_s",
            self.reaction_s,
            0.0,
            BEHAVIOUR_TIME_MAX_S,
            " seconds",
        )?;
        ctx.range(
            "behaviour.search_s",
            self.search_s,
            0.0,
            BEHAVIOUR_TIME_MAX_S,
            " seconds",
        )
    }
}
