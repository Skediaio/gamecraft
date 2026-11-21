//! Launchers, projectiles in flight, hits, damage and `down`, and the sound of shots.
//!
//! - **Firing.** A launcher fires when its character's input holds `fire`, as its fire mode
//!   allows ([`crate::handling`]: once per pull in `semi`, while held in `auto`, never on
//!   `safe`), or by [`crate::Sim::fire`] (not on `safe`). A shot needs the launcher ready: the
//!   mode's interval after the last shot (`rate_s`, or the cyclic rate's in `auto` and
//!   `burst`), and after the last shot of a magazine `reload_empty_s` (else `reload_s`; or the
//!   interval when longer) before a full magazine; a reload asked for (the input's `reload`)
//!   takes `reload_s`; the magazines of a counted pouch keep their rounds (see [`Magazine`]) and
//!   a launcher with `auto_reload` off waits for that request. The muzzle is at a character's
//!   eyes, or `muzzle_height_m` above any other entity's position; a character whose
//!   `held_model` has a `muzzle_m` shoots from the barrel's mouth as the view model carries it,
//!   along a barrel zeroed at `zero_m` (`held_shot`). The direction is the yaw and pitch asked for (plus a character's
//!   recoil and sway), turned by the dispersion: a uniform draw in a cone of `dispersion_mrad`
//!   (times `ads_dispersion_factor` while aiming down the sights) from the world's seeded
//!   stream forked by [`LABEL_DISPERSION`], the launcher's entity id and its shot number. The
//!   shooter's own motion is not added.
//! - **Hearing.** Every character but the shooter whose eyes are within `sound_range_m` of the
//!   muzzle, and that is not down, hears the shot: a `heard` event.
//! - **Flight.** Per tick, projectiles in id order, each over [`crate::PROJECTILE_SUBSTEPS`]
//!   substeps of [`crate::ballistics::advance`]. The segment flown in a substep is tested
//!   against the terrain, every entity's shape (solid or not) and every character's capsule
//!   (feet to head, `radius_m`); the nearest hit ends the flight with a `hit` event. The
//!   shooter's own capsule and shape are left out for its first [`SELF_HIT_GRACE_S`]. A flight
//!   also ends, without a hit, after the launcher's `max_flight_s`, below the terrain, farther
//!   outside the terrain than its size, below [`crate::FALL_FLOOR_M`] without terrain, or
//!   beyond the coordinate limit of the world document.
//! - **Damage.** A hit takes `energy_j * damage_per_joule - armor` (at least 0) from the
//!   `health` of the entity hit, `energy_j` being `mass * speed^2 / 2` at the hit; the hit that
//!   brings it to 0 produces a `down` event. A downed character takes no input and stops.

use std::collections::BTreeMap;

use gamecraft_math::{Portable, Rng, deg_to_rad, rad_to_deg, yaw_forward};
use gamecraft_world::limits::MAX_COORDINATE_M;
use gamecraft_world::{
    AimAxes, Entity, EntityId, Environment, FireMode, HeightField, HeldModel, Launcher, World,
};
use serde::Serialize;

use crate::aim;
use crate::ballistics::{Flight, SUBSTEP_S, advance, air_at};
use crate::ray::{self, RayHit};
use crate::trunk::Trunks;
use crate::{EventKind, FALL_FLOOR_M, PROJECTILE_SUBSTEPS};

/// The label of the dispersion's random stream (forked from the world seed).
pub const LABEL_DISPERSION: u64 = 0xD15B_E25E;
/// Seconds of flight during which a projectile cannot hit its own shooter.
pub const SELF_HIT_GRACE_S: f64 = 0.25;
/// A launcher counts as ready when less than this is left of its wait, seconds (sums of tick
/// lengths are not exact).
const READY_EPSILON_S: f64 = 1e-9;
/// A projectile this far below the terrain has gone through it, metres.
const BELOW_GROUND_M: f64 = 0.01;
/// Most projectiles in flight at once; a shot beyond it is refused.
pub const MAX_PROJECTILES: usize = 10_000;

/// A projectile in flight.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Projectile {
    /// Its number, counted from 1 since the start; never reused.
    pub id: u64,
    /// The entity whose launcher fired it.
    pub shooter: EntityId,
    /// World position, metres.
    pub position: [f64; 3],
    /// Velocity, metres per second.
    pub velocity: [f64; 3],
    /// Seconds since it was fired.
    pub age_s: f64,
    /// Mass, kilograms (from the launcher).
    pub mass_kg: f64,
    /// Ballistic coefficient, kg/m2 (from the launcher).
    pub ballistic_coefficient: f64,
    /// Damage per joule (from the launcher).
    pub damage_per_joule: f64,
    /// Longest flight, seconds (from the launcher).
    pub max_flight_s: f64,
}

/// The state of one launcher that is not in the document.
///
/// With `spare_magazines` the shooter carries a pouch of magazines, each with its own round
/// count (`pouch`: round count to number of magazines; magazines of equal counts are alike, so
/// that is all there is to remember). A reload puts the magazine of the launcher back in the
/// pouch (an empty one is dropped) and fits the fullest one of the pouch, which takes
/// `reload_s`; the launcher holds no rounds meanwhile (`left` is 0, `loading` are the rounds
/// being fitted). Without `spare_magazines` the pouch is unlimited and full: a reload fills the
/// launcher after its time and the partly used magazine is lost.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Magazine {
    /// Shots left before a reload.
    pub left: u32,
    /// Shots fired since the start (numbers the dispersion's draws).
    pub shots: u64,
    /// Seconds until it can fire again.
    pub ready_in_s: f64,
    /// Whether the wait is a reload (the magazine is refilled when it ends).
    pub reloading: bool,
    /// Where the selector is: one of the launcher's `modes`.
    pub mode: FireMode,
    /// Magazines in the pouch; `None` is unlimited. Always the pouch's count.
    pub spare: Option<u32>,
    /// The pouch: round count to the number of magazines holding that many; `None` is
    /// unlimited full magazines. Never holds an empty magazine.
    pub pouch: Option<BTreeMap<u32, u32>>,
    /// Rounds of the magazine being fitted while a reload with a pouch runs, else 0.
    pub loading: u32,
    /// Seconds the running reload takes in all (0 while not reloading).
    pub reload_total_s: f64,
}

impl Magazine {
    /// A full magazine, ready, the selector at the launcher's first mode, a full pouch.
    pub(crate) fn full(launcher: &Launcher) -> Magazine {
        let pouch = launcher.spare_magazines.map(|n| {
            let mut p = BTreeMap::new();
            if n > 0 {
                p.insert(launcher.magazine, n);
            }
            p
        });
        let mut m = Magazine {
            left: launcher.magazine,
            shots: 0,
            ready_in_s: 0.0,
            reloading: false,
            mode: launcher.first_mode(),
            spare: None,
            pouch,
            loading: 0,
            reload_total_s: 0.0,
        };
        m.sync();
        m
    }

    /// Keeps `spare` the count of the pouch.
    fn sync(&mut self) {
        self.spare = self
            .pouch
            .as_ref()
            .map(|p| p.values().fold(0u32, |a, n| a.saturating_add(*n)));
    }

    /// Takes a changed launcher into account: no more rounds than its magazine holds, no more
    /// magazines in the pouch than `spare_magazines` (the emptiest go first), and a pouch that
    /// appears or goes.
    pub(crate) fn fit(&mut self, launcher: &Launcher) {
        let cap = launcher.magazine;
        self.left = self.left.min(cap);
        self.loading = self.loading.min(cap);
        match (launcher.spare_magazines, self.pouch.take()) {
            (None, _) => {}
            (Some(_), None) => self.pouch = Self::full(launcher).pouch,
            (Some(n), Some(old)) => {
                let mut merged: BTreeMap<u32, u32> = BTreeMap::new();
                for (rounds, count) in old {
                    let slot = merged.entry(rounds.min(cap)).or_insert(0);
                    *slot = slot.saturating_add(count);
                }
                let mut over = merged
                    .values()
                    .fold(0u32, |a, c| a.saturating_add(*c))
                    .saturating_sub(n);
                while over > 0 {
                    let Some((&rounds, &count)) = merged.first_key_value() else {
                        break;
                    };
                    let drop = count.min(over);
                    over -= drop;
                    if drop == count {
                        merged.remove(&rounds);
                    } else {
                        merged.insert(rounds, count - drop);
                    }
                }
                self.pouch = Some(merged);
            }
        }
        self.sync();
    }

    /// The rounds of the fullest magazine of the pouch (`None` for an unlimited or an empty
    /// one).
    fn fullest(&self) -> Option<u32> {
        self.pouch
            .as_ref()?
            .last_key_value()
            .map(|(rounds, _)| *rounds)
            .filter(|rounds| *rounds > 0)
    }

    /// The magazines of the pouch, fullest first (`None` for an unlimited pouch).
    pub fn pouch_rounds(&self) -> Option<Vec<u32>> {
        let pouch = self.pouch.as_ref()?;
        let mut out = Vec::new();
        for (rounds, count) in pouch.iter().rev() {
            for _ in 0..*count {
                out.push(*rounds);
            }
        }
        Some(out)
    }

    /// Whether a reload may start: not reloading already, the launcher not full, and with a
    /// counted pouch a magazine in it that holds more rounds than the launcher does.
    fn can_reload(&self, launcher: &Launcher) -> bool {
        if self.reloading || self.left >= launcher.magazine {
            return false;
        }
        match &self.pouch {
            None => true,
            Some(_) => self.fullest().is_some_and(|rounds| rounds > self.left),
        }
    }

    /// Starts a reload of `seconds` (after any wait still running); the caller has checked
    /// [`Magazine::can_reload`]. With a pouch the launcher's magazine goes back in it (an empty
    /// one is dropped) and the fullest one is taken out to be fitted.
    fn begin_reload(&mut self, seconds: f64) {
        let fitted = self.fullest();
        if let (Some(pouch), Some(rounds)) = (&mut self.pouch, fitted) {
            if let Some(count) = pouch.get_mut(&rounds) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    pouch.remove(&rounds);
                }
            }
            if self.left > 0 {
                let slot = pouch.entry(self.left).or_insert(0);
                *slot = slot.saturating_add(1);
            }
            self.loading = rounds;
            self.left = 0;
            self.sync();
        }
        self.reloading = true;
        self.ready_in_s = self.ready_in_s.max(seconds);
        self.reload_total_s = self.ready_in_s;
    }

    /// Starts a reload of `reload_s` unless the launcher is full or reloading already, or its
    /// pouch has no better magazine. Whether it started.
    pub(crate) fn reload(&mut self, launcher: &Launcher) -> bool {
        if !self.can_reload(launcher) {
            return false;
        }
        self.begin_reload(launcher.reload_s);
        true
    }

    /// How far the running reload is, 0 to 1 (0 while not reloading).
    pub fn reload_progress(&self) -> f64 {
        if !self.reloading || self.reload_total_s.is_nan() || self.reload_total_s <= 0.0 {
            return 0.0;
        }
        (1.0 - self.ready_in_s / self.reload_total_s).clamp(0.0, 1.0)
    }

    /// Whether it can fire now.
    pub fn ready(&self) -> bool {
        self.ready_in_s <= READY_EPSILON_S && self.left > 0
    }

    /// One tick of waiting.
    pub(crate) fn wait(&mut self, dt: f64, launcher: &Launcher) {
        self.ready_in_s = (self.ready_in_s - dt).max(0.0);
        if self.reloading && self.ready_in_s <= READY_EPSILON_S {
            self.left = if self.pouch.is_some() {
                self.loading.min(launcher.magazine)
            } else {
                launcher.magazine
            };
            self.loading = 0;
            self.reloading = false;
            self.reload_total_s = 0.0;
            self.ready_in_s = 0.0;
        }
    }

    /// Counts a shot and starts the wait for the next one; an emptied magazine starts a reload
    /// by itself when the launcher's `auto_reload` is on and a magazine is to be had.
    fn shoot(&mut self, launcher: &Launcher) {
        self.left = self.left.saturating_sub(1);
        self.shots = self.shots.saturating_add(1);
        let interval = launcher.interval_s(self.mode);
        self.ready_in_s = interval;
        if self.left == 0 && launcher.auto_reload && self.can_reload(launcher) {
            self.begin_reload(launcher.empty_reload_s().max(interval));
        }
    }
}

/// The unit direction of a yaw (counter-clockwise from north) and a pitch (up from level).
pub fn aim_direction(yaw_deg: f64, pitch_deg: f64) -> [f64; 3] {
    let f = yaw_forward(yaw_deg);
    let (sin_p, cos_p) = deg_to_rad(pitch_deg).psin_cos();
    [f.x * cos_p, sin_p, f.z * cos_p]
}

/// Where an entity's launcher fires from: a character's eyes, or `muzzle_height_m` above
/// any other entity's position.
pub fn muzzle(entity: &Entity, launcher: &Launcher) -> [f64; 3] {
    let [x, y, z] = entity.position;
    let up = entity
        .character()
        .map_or(launcher.muzzle_height_m, |c| c.eye_height_m);
    [x, y + up, z]
}

/// A uniform offset (yaw, pitch) in radians within a cone of `radius_mrad`, drawn from `rng`.
pub(crate) fn cone_offset(rng: &mut Rng, radius_mrad: f64) -> (f64, f64) {
    if !(radius_mrad.is_finite() && radius_mrad > 0.0) {
        return (0.0, 0.0);
    }
    let r = radius_mrad * 1e-3 * rng.unit().sqrt();
    let angle = 2.0 * core::f64::consts::PI * rng.unit();
    let (sin, cos) = angle.psin_cos();
    (r * cos, r * sin)
}

/// One shot about to leave a muzzle.
pub(crate) struct Shot {
    /// The entity whose launcher fires.
    pub(crate) shooter: EntityId,
    /// The muzzle.
    pub(crate) from: [f64; 3],
    /// Where it is aimed, degrees, before dispersion.
    pub(crate) yaw_deg: f64,
    /// Up from level, degrees, before dispersion.
    pub(crate) pitch_deg: f64,
    /// The launcher's shot number (numbers the dispersion's draw).
    pub(crate) number: u64,
    /// The projectile's id.
    pub(crate) id: u64,
    /// The spread of this shot, milliradians: the launcher's, less while aiming down the
    /// sights.
    pub(crate) dispersion_mrad: f64,
}

/// The projectile of `shot` from `launcher`.
pub(crate) fn launch(world: &World, launcher: &Launcher, shot: &Shot) -> Projectile {
    let mut rng = Rng::new(world.seed)
        .fork(LABEL_DISPERSION)
        .fork(shot.shooter.0)
        .fork(shot.number);
    let (dy, dp) = cone_offset(&mut rng, shot.dispersion_mrad);
    let yaw = shot.yaw_deg + dy.to_degrees();
    let pitch = shot.pitch_deg + dp.to_degrees();
    let [dx, up, dz] = aim_direction(yaw, pitch);
    let v = launcher.muzzle_velocity_mps;
    Projectile {
        id: shot.id,
        shooter: shot.shooter,
        position: shot.from,
        velocity: [dx * v, up * v, dz * v],
        age_s: 0.0,
        mass_kg: launcher.mass_kg,
        ballistic_coefficient: launcher.ballistic_coefficient,
        damage_per_joule: launcher.damage_per_joule,
        max_flight_s: launcher.max_flight_s,
    }
}

/// Where a shot is aimed and how widely it spreads.
pub(crate) struct Aim {
    /// Yaw, degrees, before dispersion.
    pub(crate) yaw_deg: f64,
    /// Pitch, degrees, before dispersion.
    pub(crate) pitch_deg: f64,
    /// Spread, milliradians.
    pub(crate) dispersion_mrad: f64,
    /// How far up to the sights the held model is, 0 (hip) to 1.
    pub(crate) ads_amount: f64,
    /// The shooter's own velocity, metres per second; a shot from a held model's muzzle
    /// carries it (the muzzle velocity is the launcher's, relative to the shooter).
    pub(crate) shooter_velocity_mps: [f64; 3],
}

/// The eye height of the zeroing's air, metres.
const ZERO_EYE_Y_M: f64 = 1.6;
/// How close the zeroed barrel crosses the line of sight at the zeroing distance, metres.
const ZERO_TOLERANCE_M: f64 = 1e-3;

/// The direction of the barrel of `held` relative to its aim, as (right, up, back) in the aim's
/// axes, when the weapon is zeroed: the sights (the model at `ads_offset_m`, else `offset_m`)
/// are on the line of sight, and the barrel, which sits below and beside it, is tilted so that
/// a shot crosses the line of sight at `zero_m` in still standard air (the firing solution of
/// [`crate::aim`], to a millimetre). Once per weapon: it does not move with the sights' blend.
/// `None` without `muzzle_m`.
fn zeroed_bore(held: &HeldModel, launcher: &Launcher) -> Option<[f64; 3]> {
    held.muzzle_m?;
    let [right, up, back] = held.muzzle_local(1.0)?;
    let distance = held.zero_distance_m();
    let mut still = World::new("zero", 0);
    still.environment.wind_speed_mps = 0.0;
    let eye = [0.0, ZERO_EYE_Y_M, 0.0];
    let from = [eye[0] + right, eye[1] + up, eye[2] + back];
    let target = [eye[0], eye[1], eye[2] - distance];
    let solution = aim::solve_within(&still, None, from, launcher, target, ZERO_TOLERANCE_M);
    let d = aim_direction(solution.yaw_deg, solution.pitch_deg);
    d.iter().all(|v| v.is_finite()).then_some(d)
}

/// Where a character's shot leaves and along what (yaw and pitch, degrees) when it holds a
/// model with a `muzzle_m`: from the barrel's mouth as the view model carries it at `aim`'s
/// sights blend, along the zeroed barrel. `None` for everyone else (they shoot from the eye).
fn held_shot(shooter: &Entity, launcher: &Launcher, aim: &Aim) -> Option<([f64; 3], f64, f64)> {
    let held = shooter.held_model()?;
    let character = shooter.character()?;
    let [x, y, z] = shooter.position;
    let eye = [x, y + character.eye_height_m, z];
    let from = held.muzzle_point(eye, aim.yaw_deg, aim.pitch_deg, aim.ads_amount)?;
    let bore = zeroed_bore(held, launcher)?;
    let [dx, dy, dz] = AimAxes::of(aim.yaw_deg, aim.pitch_deg).vector(bore);
    let yaw = rad_to_deg((-dx).patan2(-dz));
    let pitch = rad_to_deg(dy.patan2((dx * dx + dz * dz).sqrt()));
    (yaw.is_finite() && pitch.is_finite()).then_some((from, yaw, pitch))
}

/// Fires `launcher` of `shooter` along `aim` if it is ready and not on `safe`: the projectile
/// and the `heard` events of its sound; the magazine counts the shot. `None` when it does not
/// fire.
pub(crate) fn try_fire(
    world: &World,
    shooter: &Entity,
    launcher: &Launcher,
    magazine: &mut Magazine,
    aim: &Aim,
    id: u64,
) -> Option<(Projectile, Vec<EventKind>)> {
    if !magazine.ready() || magazine.mode == FireMode::Safe {
        return None;
    }
    let held = held_shot(shooter, launcher, aim);
    let (from, yaw_deg, pitch_deg) =
        held.unwrap_or_else(|| (muzzle(shooter, launcher), aim.yaw_deg, aim.pitch_deg));
    let shot = Shot {
        shooter: shooter.id,
        from,
        yaw_deg,
        pitch_deg,
        number: magazine.shots,
        id,
        dispersion_mrad: aim.dispersion_mrad,
    };
    let mut projectile = launch(world, launcher, &shot);
    if held.is_some() {
        for (v, own) in projectile.velocity.iter_mut().zip(aim.shooter_velocity_mps) {
            if own.is_finite() {
                *v += own;
            }
        }
    }
    magazine.shoot(launcher);
    Some((
        projectile,
        heard(world, shooter.id, from, launcher.sound_range_m),
    ))
}

/// The `heard` events of a shot from `point` by `shooter`: every character but the shooter,
/// not down, whose eyes are within `range_m`, in id order.
fn heard(world: &World, shooter: EntityId, point: [f64; 3], range_m: f64) -> Vec<EventKind> {
    let mut out = Vec::new();
    for e in world.entities() {
        let Some(c) = e.character() else {
            continue;
        };
        if e.id == shooter || e.is_down() {
            continue;
        }
        let [x, y, z] = e.position;
        let (dx, dy, dz) = (x - point[0], y + c.eye_height_m - point[1], z - point[2]);
        let d = (dx * dx + dy * dy + dz * dz).sqrt();
        if d.is_finite() && d <= range_m {
            out.push(EventKind::Heard {
                who: e.id,
                target: shooter,
                point,
            });
        }
    }
    out
}

/// What a substep's segment hit first.
struct Impact {
    hit: RayHit,
    target: Option<EntityId>,
    /// What was hit is a tree trunk (target is its plant): it stops the projectile, and takes
    /// no damage.
    trunk: bool,
}

/// The nearest hit along the segment from `from` with unit `dir` and length `len`: terrain,
/// shapes and character capsules, leaving out `skip`.
fn impact(
    world: &World,
    heights: Option<&HeightField>,
    trunks: &Trunks,
    from: [f64; 3],
    dir: [f64; 3],
    len: f64,
    skip: Option<EntityId>,
) -> Option<Impact> {
    let mut best = ray::cast(world, heights, trunks, from, dir, len, |id| {
        Some(id) == skip
    })
    .map(|f| Impact {
        hit: f.hit,
        target: f.entity,
        trunk: f.trunk,
    });
    for e in world.entities() {
        let Some(c) = e.character() else {
            continue;
        };
        if Some(e.id) == skip {
            continue;
        }
        let [x, y, z] = e.position;
        let r = c.radius_m;
        let a = [x, y + r, z];
        let b = [x, y + (c.height_m - r).max(r), z];
        let Some(hit) = ray::ray_capsule(from, dir, a, b, r) else {
            continue;
        };
        if hit.distance_m <= len
            && best
                .as_ref()
                .is_none_or(|b| hit.distance_m < b.hit.distance_m)
        {
            best = Some(Impact {
                hit,
                target: Some(e.id),
                trunk: false,
            });
        }
    }
    best
}

/// Whether a projectile at `p` has left the world: below the terrain, farther outside it than
/// its size, below the fall floor without terrain, or beyond the coordinate limit.
fn gone(heights: Option<&HeightField>, p: [f64; 3]) -> bool {
    let [x, y, z] = p;
    if !p
        .iter()
        .all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE_M)
    {
        return true;
    }
    match heights {
        Some(h) => {
            if let Some(g) = h.height_at(x, z) {
                return y < g - BELOW_GROUND_M;
            }
            let [x0, z0] = h.min();
            let [x1, z1] = h.max();
            let size = (x1 - x0).max(z1 - z0);
            x < x0 - size || x > x1 + size || z < z0 - size || z > z1 + size || y < FALL_FLOOR_M
        }
        None => y < FALL_FLOOR_M,
    }
}

/// One substep of flight through the air of `env` at the flight's position (with the terrain
/// height under it from `heights`): the flight model of every projectile and of
/// [`crate::aim::solve`].
pub fn substep(
    env: &Environment,
    heights: Option<&HeightField>,
    flight: Flight,
    ballistic_coefficient: f64,
) -> Flight {
    let [x, _, z] = flight.position;
    let ground = heights.and_then(|h| h.height_at(x, z));
    let air = air_at(env, ground, flight.position);
    advance(flight, ballistic_coefficient, &air)
}

/// Flies one projectile one tick. `Some(events)` (a `hit`, maybe a `down`) when it hit
/// something, `Some(empty)` when its flight ended without a hit, `None` while it flies on.
/// Damage is written into `world`.
pub(crate) fn fly(
    world: &mut World,
    heights: Option<&HeightField>,
    trunks: &Trunks,
    p: &mut Projectile,
) -> Option<Vec<EventKind>> {
    for _ in 0..PROJECTILE_SUBSTEPS {
        let next = substep(
            &world.environment,
            heights,
            Flight {
                position: p.position,
                velocity: p.velocity,
            },
            p.ballistic_coefficient,
        );
        if !(next.position.iter().all(|c| c.is_finite())
            && next.velocity.iter().all(|c| c.is_finite()))
        {
            return Some(Vec::new());
        }
        let delta = [
            next.position[0] - p.position[0],
            next.position[1] - p.position[1],
            next.position[2] - p.position[2],
        ];
        let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        let skip = (p.age_s < SELF_HIT_GRACE_S).then_some(p.shooter);
        if let Some(dir) = ray::normalize(delta)
            && let Some(found) = impact(world, heights, trunks, p.position, dir, len, skip)
        {
            let k = if len > 0.0 {
                (found.hit.distance_m / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let speed_at = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let speed = speed_at(p.velocity) + k * (speed_at(next.velocity) - speed_at(p.velocity));
            let energy_j = 0.5 * p.mass_kg * speed * speed;
            let point = ray::along(p.position, dir, found.hit.distance_m);
            let mut events = vec![EventKind::Hit {
                projectile: p.id,
                shooter: p.shooter,
                target: found.target,
                point,
                normal: found.hit.normal,
                speed_mps: speed,
                energy_j,
            }];
            if let Some(target) = found.target
                && !found.trunk
                && let Some(health) = world.entity_mut(target).and_then(Entity::health_mut)
                && !health.is_down()
            {
                let damage = (energy_j * p.damage_per_joule - health.armor).max(0.0);
                if damage.is_finite() {
                    health.current = (health.current - damage).max(0.0);
                }
                if health.is_down() {
                    events.push(EventKind::Down {
                        who: p.shooter,
                        target,
                    });
                }
            }
            return Some(events);
        }
        p.position = next.position;
        p.velocity = next.velocity;
        p.age_s += SUBSTEP_S;
        if p.age_s >= p.max_flight_s || gone(heights, p.position) {
            return Some(Vec::new());
        }
    }
    None
}
