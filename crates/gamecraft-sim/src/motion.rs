//! One tick of one character: facing, wanted velocity, jump and gravity, slope limit, solids,
//! ground. Pure functions of their arguments, so every platform gives the same bits.

use gamecraft_math::{Portable, deg_to_rad, yaw_forward};
use gamecraft_world::limits::MAX_COORDINATE_M;
use gamecraft_world::{Character, HeightField};

use crate::solid::Solid;
use crate::{Body, DT, FALL_FLOOR_M, GRAVITY_MPS2, Input, MAX_PUSH_ITERATIONS, STEP_HEIGHT_M};

/// How far above the terrain, metres, a character's feet may be and still count as standing
/// on the terrain itself (for the slope limit) rather than on a solid.
const ON_TERRAIN_TOLERANCE_M: f64 = 1e-9;

/// What a character moves through: the terrain and the solid shapes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Surroundings<'a> {
    /// The terrain's height field, `None` without terrain.
    pub(crate) heights: Option<&'a HeightField>,
    /// The solid shapes, in id order.
    pub(crate) solids: &'a [Solid],
}

/// Where a character is and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct State {
    /// The entity's base, world metres.
    pub(crate) position: [f64; 3],
    /// The entity's `rotation_y_deg`.
    pub(crate) rotation_y_deg: f64,
    /// The body.
    pub(crate) body: Body,
}

/// `(forward, right)` scaled down onto the unit disc when longer than 1. Finite input stays
/// finite.
pub(crate) fn clamp_to_disc(forward: f64, right: f64) -> (f64, f64) {
    let len_sq = forward * forward + right * right;
    if len_sq > 1.0 {
        let len = len_sq.sqrt();
        (forward / len, right / len)
    } else {
        (forward, right)
    }
}

/// The horizontal velocity (x, z) an input asks of a character.
fn wanted_velocity(input: &Input, character: &Character) -> (f64, f64) {
    let (forward, right) = clamp_to_disc(input.forward, input.right);
    let f = yaw_forward(input.yaw_deg);
    // The facing turned 90 degrees clockwise seen from above.
    let (rx, rz) = (-f.z, f.x);
    // A unit-disc combination of two orthonormal vectors: each part is at most 1, so the
    // product with a finite speed stays finite.
    let dx = f.x * forward + rx * right;
    let dz = f.z * forward + rz * right;
    let speed = if input.run {
        character.run_speed_mps
    } else {
        character.walk_speed_mps
    };
    let (vx, vz) = (dx * speed, dz * speed);
    if vx.is_finite() && vz.is_finite() {
        (vx, vz)
    } else {
        (0.0, 0.0)
    }
}

impl Surroundings<'_> {
    /// The terrain height under `x`, `z`, `None` without terrain or outside it.
    fn terrain(&self, x: f64, z: f64) -> Option<f64> {
        self.heights.and_then(|h| h.height_at(x, z))
    }

    /// The ground under `x`, `z` for feet that can reach up to `reach`: the highest of the
    /// terrain and the tops of boxes and cylinders containing the point whose top is at most
    /// `reach`; [`FALL_FLOOR_M`] when there is none.
    fn ground(&self, x: f64, z: f64, reach: f64) -> f64 {
        let mut best = self.terrain(x, z);
        for solid in self.solids {
            if let Some(top) = solid.support_top(x, z)
                && top <= reach
            {
                best = Some(best.map_or(top, |b| b.max(top)));
            }
        }
        best.unwrap_or(FALL_FLOOR_M)
    }

    /// Clamps a centre into the terrain square (when there is one) and the coordinate limit
    /// of the world document.
    fn clamp_horizontal(&self, x: f64, z: f64) -> (f64, f64) {
        let (mut x, mut z) = (
            x.clamp(-MAX_COORDINATE_M, MAX_COORDINATE_M),
            z.clamp(-MAX_COORDINATE_M, MAX_COORDINATE_M),
        );
        if let Some(h) = self.heights {
            let [x0, z0] = h.min();
            let [x1, z1] = h.max();
            if x0 <= x1 && z0 <= z1 {
                x = x.clamp(x0, x1);
                z = z.clamp(z0, z1);
            }
        }
        (x, z)
    }

    /// Whether a character standing on the terrain at (`x0`, `z0`) with its feet at `feet`
    /// may not step to (`x1`, `z1`) because the terrain rises steeper than `max_slope_deg`.
    /// Downhill, level, standing on a solid, or no terrain: allowed.
    fn slope_blocks(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        feet: f64,
        max_slope_deg: f64,
    ) -> bool {
        let (Some(h0), Some(h1)) = (self.terrain(from.0, from.1), self.terrain(to.0, to.1)) else {
            return false;
        };
        if feet > h0 + ON_TERRAIN_TOLERANCE_M {
            return false;
        }
        let rise = h1 - h0;
        if rise <= 0.0 {
            return false;
        }
        let (dx, dz) = (to.0 - from.0, to.1 - from.1);
        let run = (dx * dx + dz * dz).sqrt();
        rise > run * deg_to_rad(max_slope_deg).ptan()
    }

    /// Pushes a character's circle out of every solid it overlaps, in up to
    /// [`MAX_PUSH_ITERATIONS`] rounds.
    fn push_out(&self, x: f64, z: f64, feet: f64, character: &Character) -> (f64, f64) {
        let mut p = [x, z];
        for _ in 0..MAX_PUSH_ITERATIONS {
            let mut moved = false;
            for solid in self.solids {
                if let Some(q) = solid.push(p, character.radius_m, feet, character.height_m) {
                    p = q;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        let [x, z] = p;
        (x, z)
    }

    /// Advances one character one tick.
    pub(crate) fn step(&self, state: State, character: &Character, input: Option<&Input>) -> State {
        let [x0, y0, z0] = state.position;
        let (rotation_y_deg, (vx, vz), jump) = match input {
            Some(i) => (i.yaw_deg, wanted_velocity(i, character), i.jump),
            None => (state.rotation_y_deg, (0.0, 0.0), false),
        };
        let mut grounded = state.body.grounded;
        let mut vy = state.body.velocity[1];
        let mut y = y0;

        // Vertical: jump, then gravity with semi-implicit Euler (velocity first).
        if grounded && jump {
            vy = character.jump_speed_mps;
            grounded = false;
        }
        if grounded {
            vy = 0.0;
        } else {
            vy -= GRAVITY_MPS2 * DT;
            y = y0 + vy * DT;
        }

        // Horizontal: the wanted step, the slope limit, the solids, the edge of the world.
        let (mut x, mut z) = self.clamp_horizontal(x0 + vx * DT, z0 + vz * DT);
        if grounded && self.slope_blocks((x0, z0), (x, z), y, character.max_slope_deg) {
            (x, z) = (x0, z0);
        }
        (x, z) = self.push_out(x, z, y, character);
        (x, z) = self.clamp_horizontal(x, z);

        // Ground: follow it, land on it, or leave it.
        let ground = self.ground(x, z, y0.max(y) + STEP_HEIGHT_M);
        if grounded {
            if ground >= y - STEP_HEIGHT_M {
                y = ground;
            } else {
                grounded = false;
            }
        } else if y <= ground {
            y = ground;
            if vy <= 0.0 {
                vy = 0.0;
                grounded = true;
            }
        }

        // The document's limits.
        if y > MAX_COORDINATE_M {
            y = MAX_COORDINATE_M;
            vy = vy.min(0.0);
        }
        y = y.max(-MAX_COORDINATE_M);

        let next = State {
            position: [x, y, z],
            rotation_y_deg,
            body: Body {
                velocity: [vx, vy, vz],
                grounded,
            },
        };
        let finite = next.position.iter().all(|v| v.is_finite())
            && next.body.velocity.iter().all(|v| v.is_finite())
            && next.rotation_y_deg.is_finite();
        if finite {
            next
        } else {
            // Never write a non-finite number: stay, at rest.
            State {
                body: Body::default(),
                ..state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_to_disc_keeps_short_and_scales_long() {
        assert_eq!(clamp_to_disc(0.5, -0.5), (0.5, -0.5));
        assert_eq!(clamp_to_disc(3.0, 4.0), (0.6, 0.8));
        assert_eq!(clamp_to_disc(1.0, 0.0), (1.0, 0.0));
    }

    #[test]
    fn right_is_the_facing_turned_clockwise() {
        let c = Character::default();
        let input = Input {
            right: 1.0,
            ..Input::default()
        };
        let (vx, vz) = wanted_velocity(&input, &c);
        assert_eq!(
            (vx, vz.abs()),
            (c.walk_speed_mps, 0.0),
            "facing north, right is east"
        );
    }

    #[test]
    fn huge_speeds_stay_finite() {
        let c = Character {
            walk_speed_mps: f64::MAX,
            ..Character::default()
        };
        let input = Input {
            forward: 1.0,
            right: 1.0,
            yaw_deg: 45.0,
            ..Input::default()
        };
        let (vx, vz) = wanted_velocity(&input, &c);
        assert!(vx.is_finite() && vz.is_finite());
    }
}
