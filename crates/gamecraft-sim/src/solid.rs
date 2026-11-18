//! Solid shapes as the simulation sees them: footprints, tops, and pushing a character's circle
//! out of them.

use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::{Entity, ShapeKind, World};

use crate::STEP_HEIGHT_M;

/// Below this horizontal distance, metres, two centres count as one point and a push takes the
/// fixed +X axis instead of the (undefined) direction between them.
const MIN_PUSH_DISTANCE_M: f64 = 1e-12;

/// The horizontal outline of a solid, in its own frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Footprint {
    /// A box: half extents along its local x and z, and the sine and cosine of its rotation.
    Rect {
        half_x: f64,
        half_z: f64,
        sin: f64,
        cos: f64,
    },
    /// An upright cylinder of this radius.
    Circle { radius: f64 },
    /// A sphere of this radius, centred half way between bottom and top.
    Ball { radius: f64 },
}

/// One solid shape of the running world, sized and placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Solid {
    /// World x and z of the centre of the footprint (the entity's position).
    centre: [f64; 2],
    /// World y of the base.
    bottom: f64,
    /// World y of the top.
    top: f64,
    footprint: Footprint,
}

/// Every solid shape of `world` in id order. Shapes on character entities are left out:
/// characters do not collide with each other in M0.
pub(crate) fn solids_of(world: &World) -> Vec<Solid> {
    world.entities().filter_map(Solid::from_entity).collect()
}

impl Solid {
    /// The solid of an entity: a solid shape on an entity that is not a character.
    fn from_entity(entity: &Entity) -> Option<Solid> {
        let shape = entity.shape()?;
        if !shape.solid || entity.character().is_some() {
            return None;
        }
        let s = entity.scale;
        let [sx, sy, sz] = shape.size;
        let [px, py, pz] = entity.position;
        let (footprint, height) = match shape.kind {
            ShapeKind::Box => {
                let (sin, cos) = deg_to_rad(entity.rotation_y_deg).psin_cos();
                (
                    Footprint::Rect {
                        half_x: sx * s / 2.0,
                        half_z: sz * s / 2.0,
                        sin,
                        cos,
                    },
                    sy * s,
                )
            }
            ShapeKind::Cylinder => (
                Footprint::Circle {
                    radius: sx * s / 2.0,
                },
                sy * s,
            ),
            ShapeKind::Sphere => (
                Footprint::Ball {
                    radius: sx * s / 2.0,
                },
                sx * s,
            ),
        };
        let solid = Solid {
            centre: [px, pz],
            bottom: py,
            top: py + height,
            footprint,
        };
        let finite = solid.centre.iter().all(|v| v.is_finite())
            && solid.bottom.is_finite()
            && solid.top.is_finite();
        finite.then_some(solid)
    }

    /// World offset (x, z) to the solid's local frame: the inverse of its rotation.
    fn to_local(sin: f64, cos: f64, dx: f64, dz: f64) -> (f64, f64) {
        (dx * cos - dz * sin, dx * sin + dz * cos)
    }

    /// Local (x, z) to a world offset: the rotation counter-clockwise seen from above.
    fn to_world(sin: f64, cos: f64, lx: f64, lz: f64) -> (f64, f64) {
        (lx * cos + lz * sin, -lx * sin + lz * cos)
    }

    /// The top of a box or cylinder whose footprint contains world `x`, `z`; `None` for a
    /// sphere or a point outside.
    pub(crate) fn support_top(&self, x: f64, z: f64) -> Option<f64> {
        let [cx, cz] = self.centre;
        let (dx, dz) = (x - cx, z - cz);
        let inside = match self.footprint {
            Footprint::Rect {
                half_x,
                half_z,
                sin,
                cos,
            } => {
                let (lx, lz) = Self::to_local(sin, cos, dx, dz);
                lx.abs() <= half_x && lz.abs() <= half_z
            }
            Footprint::Circle { radius } => dx * dx + dz * dz <= radius * radius,
            Footprint::Ball { .. } => false,
        };
        inside.then_some(self.top)
    }

    /// Where a character's circle (centre `p` as x, z, `radius`) standing with its feet at
    /// `feet` and `height` tall must go to leave this solid's footprint, or `None` when the
    /// solid does not block it: no vertical overlap, a top within [`STEP_HEIGHT_M`] of the
    /// feet (a step, not a wall), or no horizontal overlap.
    pub(crate) fn push(
        &self,
        p: [f64; 2],
        radius: f64,
        feet: f64,
        height: f64,
    ) -> Option<[f64; 2]> {
        let head = feet + height;
        let overlaps = self.bottom < head && self.top > feet;
        let is_wall = self.top > feet + STEP_HEIGHT_M;
        if !overlaps || !is_wall {
            return None;
        }
        let [cx, cz] = self.centre;
        let [px, pz] = p;
        let (dx, dz) = (px - cx, pz - cz);
        let out = match self.footprint {
            Footprint::Rect {
                half_x,
                half_z,
                sin,
                cos,
            } => {
                let (lx, lz) = Self::to_local(sin, cos, dx, dz);
                let (nx, nz) = push_from_rect(lx, lz, half_x, half_z, radius)?;
                let (wx, wz) = Self::to_world(sin, cos, nx, nz);
                [cx + wx, cz + wz]
            }
            Footprint::Circle { radius: r } => {
                let (wx, wz) = push_from_circle(dx, dz, r + radius)?;
                [cx + wx, cz + wz]
            }
            Footprint::Ball { radius: r } => {
                let section = self.ball_section(r, feet, head)?;
                let (wx, wz) = push_from_circle(dx, dz, section + radius)?;
                [cx + wx, cz + wz]
            }
        };
        out.iter().all(|v| v.is_finite()).then_some(out)
    }

    /// The radius of the widest horizontal circle of a ball of radius `r` between heights
    /// `feet` and `head` (clipped to the ball), `None` when there is none.
    fn ball_section(&self, r: f64, feet: f64, head: f64) -> Option<f64> {
        let lo = feet.max(self.bottom);
        let hi = head.min(self.top);
        if lo > hi {
            return None;
        }
        let cy = (self.bottom + self.top) / 2.0;
        let d = if cy < lo {
            lo - cy
        } else if cy > hi {
            cy - hi
        } else {
            0.0
        };
        let sq = r * r - d * d;
        (sq > 0.0).then(|| sq.sqrt())
    }
}

/// Pushes a point at offset (`dx`, `dz`) from a circle's centre out to distance `reach`;
/// `None` when it is already that far. Two coinciding centres push along +X.
fn push_from_circle(dx: f64, dz: f64, reach: f64) -> Option<(f64, f64)> {
    let dist = (dx * dx + dz * dz).sqrt();
    if !dist.is_finite() || dist >= reach {
        return None;
    }
    if dist < MIN_PUSH_DISTANCE_M {
        return Some((reach, 0.0));
    }
    let k = reach / dist;
    Some((dx * k, dz * k))
}

/// Pushes a circle of `radius` centred at local (`lx`, `lz`) out of the rectangle of half
/// extents `hx`, `hz` centred on the origin; `None` when they do not overlap.
fn push_from_rect(lx: f64, lz: f64, hx: f64, hz: f64, radius: f64) -> Option<(f64, f64)> {
    let qx = lx.clamp(-hx, hx);
    let qz = lz.clamp(-hz, hz);
    let (ex, ez) = (lx - qx, lz - qz);
    let dist = (ex * ex + ez * ez).sqrt();
    if dist > 0.0 {
        if !dist.is_finite() || dist >= radius {
            return None;
        }
        let k = radius / dist;
        return Some((qx + ex * k, qz + ez * k));
    }
    // The centre is inside: leave by the nearest side.
    let pen_x = hx - lx.abs();
    let pen_z = hz - lz.abs();
    if pen_x <= pen_z {
        let side = if lx < 0.0 { -1.0 } else { 1.0 };
        Some((side * (hx + radius), lz))
    } else {
        let side = if lz < 0.0 { -1.0 } else { 1.0 };
        Some((lx, side * (hz + radius)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_push_from_a_face_and_a_corner() {
        assert_eq!(push_from_rect(1.2, 0.0, 1.0, 1.0, 0.3), Some((1.3, 0.0)));
        assert_eq!(push_from_rect(1.4, 0.0, 1.0, 1.0, 0.3), None);
        let (x, z) = push_from_rect(1.1, 1.1, 1.0, 1.0, 0.3).unwrap();
        let d = ((x - 1.0) * (x - 1.0) + (z - 1.0) * (z - 1.0)).sqrt();
        assert!((d - 0.3).abs() < 1e-12);
    }

    #[test]
    fn rect_push_from_inside_takes_the_nearest_side() {
        assert_eq!(push_from_rect(0.0, 0.0, 1.0, 1.0, 0.5), Some((1.5, 0.0)));
        assert_eq!(push_from_rect(-0.1, 0.9, 1.0, 1.0, 0.5), Some((-0.1, 1.5)));
        assert_eq!(push_from_rect(-0.9, 0.1, 1.0, 1.0, 0.5), Some((-1.5, 0.1)));
    }

    #[test]
    fn circle_push_with_coinciding_centres_is_finite() {
        assert_eq!(push_from_circle(0.0, 0.0, 1.0), Some((1.0, 0.0)));
        assert_eq!(push_from_circle(0.0, 2.0, 1.0), None);
        assert_eq!(push_from_circle(f64::NAN, 0.0, 1.0), None);
    }

    #[test]
    fn local_and_world_frames_are_inverse() {
        let (s, c) = deg_to_rad(30.0).psin_cos();
        let (lx, lz) = Solid::to_local(s, c, 0.7, -1.3);
        let (wx, wz) = Solid::to_world(s, c, lx, lz);
        assert!((wx - 0.7).abs() < 1e-12 && (wz + 1.3).abs() < 1e-12);
        // Local -Z (an entity's front) turns like `yaw_forward`.
        let (fx, fz) = Solid::to_world(s, c, 0.0, -1.0);
        let f = gamecraft_math::yaw_forward(30.0);
        assert!((fx - f.x).abs() < 1e-12 && (fz - f.z).abs() < 1e-12);
    }
}
