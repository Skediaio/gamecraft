//! Firing solutions: the yaw and pitch that make a launcher's projectile pass through a target
//! point in the current air and wind.
//!
//! # How a solution is found
//!
//! There is no second flight model: [`solve`] flies trial shots with
//! [`crate::combat::substep`], the very substep every projectile in flight takes (gravity, G7
//! drag, air density and the wind profile of [`crate::ballistics`]), and corrects the aim
//! from where each one passes the target:
//!
//! 1. The first trial faces the target and takes the pitch that allows for the drop in a
//!    vacuum ([`crate::behaviour::aim_pitch_deg`]).
//! 2. A trial flies until it crosses the vertical plane through the target square to the line
//!    from the muzzle to the target. Where it crosses gives the miss: `dy` above the target
//!    and `lateral` to its right. The pitch is lowered by `atan(dy / range)` and the yaw turned
//!    left by `atan(lateral / range)`. A trial that comes down below the target before it gets
//!    there, or outlives the launcher's `max_flight_s`, is short.
//! 3. The pitches known to fly high and low (short) bracket the answer; a correction that
//!    leaves the bracket is replaced by its middle.
//!
//! It stops when a trial passes within [`AIM_TOLERANCE_M`] of the target, after
//! [`MAX_AIM_ITERATIONS`] trials, or after [`MAX_AIM_SUBSTEPS`] substeps in all. Only the low,
//! direct solution is searched (pitch at most [`MAX_AIM_PITCH_DEG`]); a target that is short
//! even at that pitch is out of reach. Obstacles are not considered (the line of fire may
//! cross a hill or a wall), and neither is the dispersion of the launcher nor the motion of
//! the target.

use gamecraft_math::{Portable, rad_to_deg, wrap_deg};
use gamecraft_world::limits::MAX_COORDINATE_M;
use gamecraft_world::{Entity, HeightField, Launcher, World};
use serde::Serialize;

use crate::ballistics::{Flight, SUBSTEP_S};
use crate::behaviour::{aim_pitch_deg, yaw_towards};
use crate::combat::{aim_direction, substep};
use crate::perception::CHEST_SHARE;

/// A solution passes this close to the target, metres.
pub const AIM_TOLERANCE_M: f64 = 0.02;
/// Most trial shots of one [`solve`].
pub const MAX_AIM_ITERATIONS: u32 = 40;
/// Most substeps of flight of one [`solve`], over all its trials.
pub const MAX_AIM_SUBSTEPS: u64 = 2_000_000;
/// Steepest pitch searched, degrees: the low, direct solution only.
pub const MAX_AIM_PITCH_DEG: f64 = 45.0;
/// Lowest pitch searched, degrees.
const MIN_AIM_PITCH_DEG: f64 = -crate::MAX_PITCH_DEG;
/// Below this horizontal distance, metres, a target is straight above or below the muzzle.
const MIN_RANGE_M: f64 = 1e-6;

/// A firing solution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AimSolution {
    /// The yaw to fire along, degrees counter-clockwise from north, 0 to 360.
    pub yaw_deg: f64,
    /// The pitch to fire at, degrees up from level.
    pub pitch_deg: f64,
    /// Seconds from the muzzle to the target.
    pub time_of_flight_s: f64,
    /// How far the projectile falls below the line of departure (the straight line along
    /// `yaw_deg` and `pitch_deg`) at the target, metres.
    pub drop_m: f64,
    /// How far it drifts to the right of the line of departure at the target (looking along
    /// the line of fire), metres; negative to the left.
    pub drift_m: f64,
    /// How far the best trial passed from the target, metres; `None` when every trial fell
    /// short.
    pub miss_m: Option<f64>,
    /// Whether the best trial passed within [`AIM_TOLERANCE_M`].
    pub reachable: bool,
}

/// The point a shot at `entity` aims at: a character's chest, the middle of a shape's height,
/// or else the entity's position.
pub fn aim_point(entity: &Entity) -> [f64; 3] {
    let [x, y, z] = entity.position;
    let up = match (entity.character(), entity.shape()) {
        (Some(c), _) => c.height_m * CHEST_SHARE,
        (None, Some(shape)) => shape.size[1] * entity.scale.y * 0.5,
        (None, None) => 0.0,
    };
    [x, y + up, z]
}

/// Where a trial ended.
enum Trial {
    /// It crossed the target's plane here, after this many seconds.
    Crossed { point: [f64; 3], time_s: f64 },
    /// It came down before the target, or its flight ended.
    Short,
    /// The substep budget ran out.
    Spent,
}

/// The line from the muzzle to the target, on the ground plane.
struct Line {
    from: [f64; 3],
    target: [f64; 3],
    /// Unit direction towards the target (x, z).
    along: [f64; 2],
    /// Unit direction to its right (x, z).
    right: [f64; 2],
    range: f64,
}

impl Line {
    /// The horizontal distance of `p` along the line, from the muzzle.
    fn reach(&self, p: [f64; 3]) -> f64 {
        (p[0] - self.from[0]) * self.along[0] + (p[2] - self.from[2]) * self.along[1]
    }

    /// The horizontal offset of `p` from `q` to the right of the line.
    fn lateral(&self, p: [f64; 3], q: [f64; 3]) -> f64 {
        (p[0] - q[0]) * self.right[0] + (p[2] - q[2]) * self.right[1]
    }
}

/// What [`solve`] needs of the world and the launcher.
struct Gun<'a> {
    world: &'a World,
    heights: Option<&'a HeightField>,
    launcher: &'a Launcher,
}

impl Gun<'_> {
    /// Flies one trial shot along `yaw_deg` and `pitch_deg` until it crosses the target's
    /// plane or falls short.
    fn trial(&self, line: &Line, yaw_deg: f64, pitch_deg: f64, budget: &mut u64) -> Trial {
        let [dx, dy, dz] = aim_direction(yaw_deg, pitch_deg);
        let v = self.launcher.muzzle_velocity_mps;
        let mut flight = Flight {
            position: line.from,
            velocity: [dx * v, dy * v, dz * v],
        };
        let mut reach = 0.0;
        let mut age = 0.0;
        loop {
            if *budget == 0 {
                return Trial::Spent;
            }
            *budget -= 1;
            let next = substep(
                &self.world.environment,
                self.heights,
                flight,
                self.launcher.ballistic_coefficient,
            );
            let sane = next
                .position
                .iter()
                .chain(&next.velocity)
                .all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE_M);
            if !sane {
                return Trial::Short;
            }
            let next_reach = line.reach(next.position);
            if next_reach >= line.range && next_reach > reach {
                let k = ((line.range - reach) / (next_reach - reach)).clamp(0.0, 1.0);
                let [a, b] = [flight.position, next.position];
                return Trial::Crossed {
                    point: [
                        a[0] + k * (b[0] - a[0]),
                        a[1] + k * (b[1] - a[1]),
                        a[2] + k * (b[2] - a[2]),
                    ],
                    time_s: age + k * SUBSTEP_S,
                };
            }
            flight = next;
            reach = next_reach;
            age += SUBSTEP_S;
            let [vx, vy, vz] = flight.velocity;
            let onwards = vx * line.along[0] + vz * line.along[1];
            // Falling below the target it never rises again: drag never outweighs gravity.
            let below = vy < 0.0 && flight.position[1] < line.target[1] - AIM_TOLERANCE_M;
            if age >= self.launcher.max_flight_s || below || onwards <= 0.0 {
                return Trial::Short;
            }
        }
    }
}

/// The best crossing so far.
#[derive(Clone, Copy)]
struct Best {
    yaw_deg: f64,
    pitch_deg: f64,
    point: [f64; 3],
    time_s: f64,
    miss: f64,
}

/// The firing solution for `launcher` fired from the muzzle `from` at the point `target`, in
/// the air and wind of `world` over the terrain `heights` (see the module docs). Never fails:
/// without a solution `reachable` is false and the best trial is described.
pub fn solve(
    world: &World,
    heights: Option<&HeightField>,
    from: [f64; 3],
    launcher: &Launcher,
    target: [f64; 3],
) -> AimSolution {
    let (dx, dz) = (target[0] - from[0], target[2] - from[2]);
    let range = (dx * dx + dz * dz).sqrt();
    let finite = from.iter().chain(&target).all(|c| c.is_finite());
    let unreachable = AimSolution {
        yaw_deg: yaw_towards(from, target).unwrap_or(0.0),
        pitch_deg: 0.0,
        time_of_flight_s: 0.0,
        drop_m: 0.0,
        drift_m: 0.0,
        miss_m: None,
        reachable: false,
    };
    if !(finite && range.is_finite() && range > MIN_RANGE_M) {
        return unreachable;
    }
    let along = [dx / range, dz / range];
    let line = Line {
        from,
        target,
        along,
        // The facing turned 90 degrees clockwise seen from above: north (0, -1) gives east.
        right: [-along[1], along[0]],
        range,
    };
    let gun = Gun {
        world,
        heights,
        launcher,
    };
    let mut yaw = unreachable.yaw_deg;
    let mut pitch = aim_pitch_deg(from, target, launcher.muzzle_velocity_mps)
        .clamp(MIN_AIM_PITCH_DEG, MAX_AIM_PITCH_DEG);
    let (mut low, mut high) = (MIN_AIM_PITCH_DEG, MAX_AIM_PITCH_DEG);
    let mut budget = MAX_AIM_SUBSTEPS;
    let mut best: Option<Best> = None;
    for _ in 0..MAX_AIM_ITERATIONS {
        match gun.trial(&line, yaw, pitch, &mut budget) {
            Trial::Spent => break,
            Trial::Short => {
                low = low.max(pitch);
                if pitch >= high - 1e-9 {
                    break;
                }
                pitch = 0.5 * (pitch + high);
            }
            Trial::Crossed { point, time_s } => {
                let dy = point[1] - target[1];
                let lateral = line.lateral(point, target);
                let miss = (dy * dy + lateral * lateral).sqrt();
                if best.is_none_or(|b| miss < b.miss) {
                    best = Some(Best {
                        yaw_deg: yaw,
                        pitch_deg: pitch,
                        point,
                        time_s,
                        miss,
                    });
                }
                if miss <= AIM_TOLERANCE_M {
                    break;
                }
                if dy > 0.0 {
                    high = high.min(pitch);
                } else {
                    low = low.max(pitch);
                }
                let corrected = pitch - rad_to_deg(dy.patan2(range));
                pitch = if corrected > low && corrected < high {
                    corrected
                } else {
                    0.5 * (low + high)
                };
                // Passing to the right: turn left, counter-clockwise, a larger yaw.
                yaw += rad_to_deg(lateral.patan2(range));
            }
        }
    }
    let Some(b) = best else {
        return AimSolution {
            pitch_deg: pitch,
            ..unreachable
        };
    };
    // The line of departure where it reaches the target's plane.
    let [ux, uy, uz] = aim_direction(b.yaw_deg, b.pitch_deg);
    let horizontal = ux * along[0] + uz * along[1];
    let (drop, drift) = if horizontal > 0.0 {
        let t = range / horizontal;
        let bore = [from[0] + ux * t, from[1] + uy * t, from[2] + uz * t];
        (bore[1] - b.point[1], line.lateral(b.point, bore))
    } else {
        (0.0, 0.0)
    };
    AimSolution {
        yaw_deg: wrap_deg(b.yaw_deg),
        pitch_deg: b.pitch_deg,
        time_of_flight_s: b.time_s,
        drop_m: drop,
        drift_m: drift,
        miss_m: Some(b.miss),
        reachable: b.miss <= AIM_TOLERANCE_M,
    }
}
