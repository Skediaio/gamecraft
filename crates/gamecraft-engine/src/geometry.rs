//! Ray maths for the queries: a ray against a box turned about Y, a sphere and an upright
//! capped cylinder. Pure `f64` functions, deterministic (sines through
//! [`gamecraft_math::Portable`]), never panicking: every division and square root is guarded
//! and a non-finite result is a miss.
//!
//! **A ray that starts inside a shape** (or on its surface) hits it at distance 0, with the
//! normal pointing back along the ray (`-direction`): the ray is blocked where it starts.

use gamecraft_math::{Portable, deg_to_rad};

/// Where a ray meets a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RayHit {
    /// Distance along the ray, metres, from 0.
    pub distance_m: f64,
    /// Unit outward normal of the surface at the hit, world axes.
    pub normal: [f64; 3],
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ax - bx, ay - by, az - bz]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    ax * bx + ay * by + az * bz
}

/// The point `origin + t * direction`.
pub(crate) fn along(origin: [f64; 3], direction: [f64; 3], t: f64) -> [f64; 3] {
    let [ox, oy, oz] = origin;
    let [dx, dy, dz] = direction;
    [ox + t * dx, oy + t * dy, oz + t * dz]
}

/// `v` scaled to length 1; `None` for a zero or non-finite vector. Scaled by its largest
/// component first, so very large and very small finite vectors work.
pub(crate) fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let [x, y, z] = v;
    let m = x.abs().max(y.abs()).max(z.abs());
    if !(m.is_finite() && m > 0.0) {
        return None;
    }
    let (x, y, z) = (x / m, y / m, z / m);
    let len = (x * x + y * y + z * z).sqrt();
    if !(len.is_finite() && len > 0.0) {
        return None;
    }
    let n = [x / len, y / len, z / len];
    n.iter().all(|c| c.is_finite()).then_some(n)
}

/// A hit only when its distance is finite and not negative and its normal finite.
fn finish(t: f64, normal: [f64; 3]) -> Option<RayHit> {
    (t.is_finite() && t >= 0.0 && normal.iter().all(|c| c.is_finite())).then_some(RayHit {
        distance_m: t,
        normal,
    })
}

/// The hit of a ray that starts inside a shape.
fn inside(direction: [f64; 3]) -> Option<RayHit> {
    let [dx, dy, dz] = direction;
    finish(0.0, [-dx, -dy, -dz])
}

/// A ray from `origin` along `direction` (any non-zero length) against a box of half-extents
/// `half` centred on `center` and turned `yaw_deg` about +Y (counter-clockwise seen from
/// above). The ray is turned into the box's own frame by the negative yaw and cut by the three
/// slabs.
pub(crate) fn ray_box(
    origin: [f64; 3],
    direction: [f64; 3],
    center: [f64; 3],
    half: [f64; 3],
    yaw_deg: f64,
) -> Option<RayHit> {
    let dir = normalize(direction)?;
    let (s, c) = deg_to_rad(yaw_deg).psin_cos();
    // World from local: x = lx c + lz s, z = -lx s + lz c; local from world is the inverse.
    let to_local = |[x, y, z]: [f64; 3]| [x * c - z * s, y, x * s + z * c];
    let o = to_local(sub(origin, center));
    let d = to_local(dir);
    let mut t_near = f64::NEG_INFINITY;
    let mut t_far = f64::INFINITY;
    let mut entry: Option<(usize, f64)> = None;
    for (axis, ((o, d), h)) in o.into_iter().zip(d).zip(half).enumerate() {
        if d == 0.0 {
            if o < -h || o > h {
                return None;
            }
            continue;
        }
        let t1 = (-h - o) / d;
        let t2 = (h - o) / d;
        // Entering through the -h face (normal -axis) when moving towards +, else the +h face.
        let (ta, tb, sign) = if t1 <= t2 {
            (t1, t2, -1.0)
        } else {
            (t2, t1, 1.0)
        };
        if ta > t_near {
            t_near = ta;
            entry = Some((axis, sign));
        }
        t_far = t_far.min(tb);
    }
    if t_near.is_nan() || t_far.is_nan() || t_near > t_far || t_far < 0.0 {
        return None;
    }
    let Some((axis, sign)) = entry else {
        // Every direction component is zero in the box frame: cannot happen for a unit ray,
        // but an origin inside every slab is inside the box.
        return inside(dir);
    };
    if t_near <= 0.0 {
        return inside(dir);
    }
    let [lx, ly, lz] = match axis {
        0 => [sign, 0.0, 0.0],
        1 => [0.0, sign, 0.0],
        _ => [0.0, 0.0, sign],
    };
    finish(t_near, [lx * c + lz * s, ly, -lx * s + lz * c])
}

/// A ray against a sphere of `radius` around `center`.
pub(crate) fn ray_sphere(
    origin: [f64; 3],
    direction: [f64; 3],
    center: [f64; 3],
    radius: f64,
) -> Option<RayHit> {
    let dir = normalize(direction)?;
    if !(radius.is_finite() && radius > 0.0) {
        return None;
    }
    let oc = sub(origin, center);
    let b = dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    if c <= 0.0 {
        return inside(dir);
    }
    if b > 0.0 {
        // Outside and moving away.
        return None;
    }
    let disc = b * b - c;
    if disc.is_nan() || disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    let normal = normalize(sub(along(origin, dir, t), center))?;
    finish(t, normal)
}

/// A ray against an upright cylinder of `radius` and `height` whose bottom cap is centred on
/// `base`, with flat caps.
pub(crate) fn ray_cylinder(
    origin: [f64; 3],
    direction: [f64; 3],
    base: [f64; 3],
    radius: f64,
    height: f64,
) -> Option<RayHit> {
    let dir = normalize(direction)?;
    if !(radius.is_finite() && radius > 0.0 && height.is_finite() && height > 0.0) {
        return None;
    }
    let [ox, oy, oz] = sub(origin, base);
    let [dx, dy, dz] = dir;
    let r2 = radius * radius;
    if ox * ox + oz * oz <= r2 && (0.0..=height).contains(&oy) {
        return inside(dir);
    }
    let mut best: Option<RayHit> = None;
    let mut consider = |t: f64, normal: [f64; 3]| {
        if let Some(hit) = finish(t, normal)
            && best.is_none_or(|b| hit.distance_m < b.distance_m)
        {
            best = Some(hit);
        }
    };
    // The side: |(o + t d).xz| = r.
    let a = dx * dx + dz * dz;
    if a > 0.0 {
        let b = ox * dx + oz * dz;
        let c = ox * ox + oz * oz - r2;
        let disc = b * b - a * c;
        if disc >= 0.0 {
            let t = (-b - disc.sqrt()) / a;
            let y = oy + t * dy;
            if t >= 0.0 && (0.0..=height).contains(&y) {
                let (px, pz) = (ox + t * dx, oz + t * dz);
                if let Some(n) = normalize([px, 0.0, pz]) {
                    consider(t, n);
                }
            }
        }
    }
    // The caps.
    if dy != 0.0 {
        for (plane, ny) in [(0.0, -1.0), (height, 1.0)] {
            let t = (plane - oy) / dy;
            let (px, pz) = (ox + t * dx, oz + t * dz);
            if t >= 0.0 && px * px + pz * pz <= r2 {
                consider(t, [0.0, ny, 0.0]);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn close3(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| close(*x, y))
    }

    #[test]
    fn box_top_side_and_rotation() {
        let h = ray_box([0.0; 3], [0.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 0.0);
        assert_eq!(h, None, "zero direction");
        let h = ray_box(
            [0.2, 10.0, 0.1],
            [0.0, -3.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0; 3],
            0.0,
        )
        .unwrap();
        assert!(close(h.distance_m, 8.0) && close3(h.normal, [0.0, 1.0, 0.0]));
        let h = ray_box(
            [0.0, 1.0, -9.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [1.0; 3],
            0.0,
        )
        .unwrap();
        assert!(close(h.distance_m, 8.0) && close3(h.normal, [0.0, 0.0, -1.0]));
        // Turned 90 degrees: the long x side lies along z.
        let h = ray_box(
            [0.0, 0.0, 9.0],
            [0.0, 0.0, -1.0],
            [0.0; 3],
            [2.0, 1.0, 1.0],
            90.0,
        )
        .unwrap();
        assert!(
            close(h.distance_m, 7.0) && close3(h.normal, [0.0, 0.0, 1.0]),
            "{h:?}"
        );
        // Turned 45 degrees: the corner points along +x, at sqrt 2.
        let h = ray_box([5.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 45.0).unwrap();
        assert!(close(h.distance_m, 5.0 - 2f64.sqrt()), "{h:?}");
        // Misses: beside, behind, parallel outside a slab.
        assert!(ray_box([5.0, 0.0, 3.0], [-1.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 0.0).is_none());
        assert!(ray_box([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 0.0).is_none());
        assert!(ray_box([0.0, 5.0, 0.0], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 0.0).is_none());
    }

    #[test]
    fn inside_is_distance_zero_against_the_ray() {
        let h = ray_box([0.1, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3], 30.0).unwrap();
        assert_eq!(h.distance_m, 0.0);
        assert!(close3(h.normal, [-1.0, 0.0, 0.0]));
        let h = ray_sphere([0.1, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0; 3], 1.0).unwrap();
        assert_eq!((h.distance_m, h.normal), (0.0, [-0.0, -1.0, -0.0]));
        let h = ray_cylinder([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3], 1.0, 2.0).unwrap();
        assert_eq!(h.distance_m, 0.0);
    }

    #[test]
    fn sphere_through_the_centre_and_misses() {
        let h = ray_sphere([0.0, 0.0, 7.0], [0.0, 0.0, -1.0], [0.0; 3], 2.0).unwrap();
        assert!(close(h.distance_m, 5.0) && close3(h.normal, [0.0, 0.0, 1.0]));
        assert!(ray_sphere([0.0, 3.0, 7.0], [0.0, 0.0, -1.0], [0.0; 3], 2.0).is_none());
        assert!(ray_sphere([0.0, 0.0, 7.0], [0.0, 0.0, 1.0], [0.0; 3], 2.0).is_none());
        assert!(ray_sphere([0.0, 0.0, 7.0], [0.0, 0.0, -1.0], [0.0; 3], 0.0).is_none());
    }

    #[test]
    fn cylinder_side_caps_and_misses() {
        let base = [0.0, 1.0, 0.0];
        let h = ray_cylinder([4.0, 2.0, 0.0], [-1.0, 0.0, 0.0], base, 1.0, 2.0).unwrap();
        assert!(close(h.distance_m, 3.0) && close3(h.normal, [1.0, 0.0, 0.0]));
        let h = ray_cylinder([0.5, 9.0, 0.0], [0.0, -1.0, 0.0], base, 1.0, 2.0).unwrap();
        assert!(close(h.distance_m, 6.0) && close3(h.normal, [0.0, 1.0, 0.0]));
        let h = ray_cylinder([0.0, -4.0, 0.5], [0.0, 1.0, 0.0], base, 1.0, 2.0).unwrap();
        assert!(close(h.distance_m, 5.0) && close3(h.normal, [0.0, -1.0, 0.0]));
        // A slanted ray over the top edge hits the cap, not the side.
        let h = ray_cylinder([3.0, 6.0, 0.0], [-1.0, -1.0, 0.0], base, 1.0, 2.0).unwrap();
        assert!(close(h.distance_m, 3.0 * 2f64.sqrt()), "{h:?}");
        assert!(close3(h.normal, [0.0, 1.0, 0.0]));
        // Above the top, beside, and the side of the infinite cylinder below the base.
        assert!(ray_cylinder([4.0, 3.5, 0.0], [-1.0, 0.0, 0.0], base, 1.0, 2.0).is_none());
        assert!(ray_cylinder([4.0, 2.0, 2.0], [-1.0, 0.0, 0.0], base, 1.0, 2.0).is_none());
        assert!(ray_cylinder([4.0, 0.0, 0.0], [-1.0, 0.0, 0.0], base, 1.0, 2.0).is_none());
    }

    #[test]
    fn hostile_numbers_miss_or_hit_finitely() {
        let big = [1e300, -1e300, 1e300];
        for d in [big, [1e-300, 0.0, 0.0], [0.0, -1.0, 0.0]] {
            for o in [big, [0.0, 5.0, 0.0], [f64::MAX, 0.0, 0.0]] {
                for h in [
                    ray_box(o, d, [0.0; 3], [1e300; 3], 1e300),
                    ray_sphere(o, d, big, 1e300),
                    ray_cylinder(o, d, big, 1e300, 1e300),
                ]
                .into_iter()
                .flatten()
                {
                    assert!(h.distance_m.is_finite() && h.normal.iter().all(|c| c.is_finite()));
                }
            }
        }
        assert_eq!(normalize([0.0, 0.0, 0.0]), None);
        assert_eq!(normalize([f64::NAN, 0.0, 0.0]), None);
        assert_eq!(normalize([1e-300, 0.0, 0.0]), Some([1.0, 0.0, 0.0]));
        assert_eq!(normalize([0.0, -1e300, 0.0]), Some([0.0, -1.0, 0.0]));
    }
}
