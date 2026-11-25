//! Ray maths, one implementation for the engine's queries (`query.raycast`,
//! `query.line_of_sight`) and the simulation (projectile hits, perception): a ray against a box
//! turned about Y, a sphere, an upright capped cylinder and a capsule, and [`cast`] and
//! [`line_of_sight`] against the terrain, every entity's `shape`, every model's collider box
//! and the surface of every `road` ([`gamecraft_world::raycast_roads`]). Pure `f64` functions,
//! deterministic (sines through [`gamecraft_math::Portable`]), never panicking: every division
//! and square root is guarded and a non-finite result is a miss.
//!
//! **A ray that starts inside a shape** (or on its surface) hits it at distance 0, with the
//! normal pointing back along the ray (`-direction`): the ray is blocked where it starts.

use gamecraft_math::{Portable, deg_to_rad};
use gamecraft_world::{
    Entity, EntityId, HeightField, Road, Shape, ShapeKind, World, raycast_roads,
};

/// Where a ray meets a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
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
pub fn along(origin: [f64; 3], direction: [f64; 3], t: f64) -> [f64; 3] {
    let [ox, oy, oz] = origin;
    let [dx, dy, dz] = direction;
    [ox + t * dx, oy + t * dy, oz + t * dz]
}

/// `v` scaled to length 1; `None` for a zero or non-finite vector. Scaled by its largest
/// component first, so very large and very small finite vectors work.
pub fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
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
pub fn ray_box(
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
pub fn ray_sphere(
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
pub fn ray_cylinder(
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

/// A ray against a capsule: the points within `radius` of the segment from `a` to `b`. A ray
/// that starts inside hits at distance 0.
pub fn ray_capsule(
    origin: [f64; 3],
    direction: [f64; 3],
    a: [f64; 3],
    b: [f64; 3],
    radius: f64,
) -> Option<RayHit> {
    let dir = normalize(direction)?;
    if !(radius.is_finite() && radius > 0.0) {
        return None;
    }
    let ba = sub(b, a);
    let oa = sub(origin, a);
    let baba = dot(ba, ba);
    // The point of the axis nearest to `p`.
    let nearest = |p: [f64; 3]| {
        let t = if baba > 0.0 {
            (dot(sub(p, a), ba) / baba).clamp(0.0, 1.0)
        } else {
            0.0
        };
        along(a, ba, t)
    };
    let inside_sq = {
        let d = sub(origin, nearest(origin));
        dot(d, d)
    };
    if inside_sq <= radius * radius {
        return inside(dir);
    }
    let mut best: Option<f64> = None;
    let mut consider = |t: f64| {
        if t.is_finite() && t >= 0.0 && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    // The side: the infinite cylinder around the axis, cut to the segment.
    if baba > 0.0 {
        let bard = dot(ba, dir);
        let baoa = dot(ba, oa);
        let rdoa = dot(dir, oa);
        let oaoa = dot(oa, oa);
        let qa = baba - bard * bard;
        let qb = baba * rdoa - baoa * bard;
        let qc = baba * oaoa - baoa * baoa - radius * radius * baba;
        let h = qb * qb - qa * qc;
        if qa > 0.0 && h >= 0.0 {
            let t = (-qb - h.sqrt()) / qa;
            let y = baoa + t * bard;
            if y > 0.0 && y < baba {
                consider(t);
            }
        }
    }
    // The two caps.
    for end in [a, b] {
        if let Some(hit) = ray_sphere(origin, dir, end, radius) {
            consider(hit.distance_m);
        }
    }
    let t = best?;
    let point = along(origin, dir, t);
    let normal = normalize(sub(point, nearest(point)))?;
    finish(t, normal)
}

/// A ray against one entity's shape: sized by `size` times the entity's scale along its own
/// axes, standing on the entity's position, a box turned by its `rotation_y_deg`. A sphere or a
/// cylinder, which has one width, takes the middle of the three scales for it (and the y scale
/// for a cylinder's height).
pub fn ray_shape(origin: [f64; 3], dir: [f64; 3], e: &Entity, shape: &Shape) -> Option<RayHit> {
    let [px, py, pz] = e.position;
    let [sx, sy, sz] = e.scale.times(shape.size);
    let round = e.scale.middle() * shape.size[0];
    match shape.kind {
        ShapeKind::Box => ray_box(
            origin,
            dir,
            [px, py + sy / 2.0, pz],
            [sx / 2.0, sy / 2.0, sz / 2.0],
            e.rotation_y_deg,
        ),
        ShapeKind::Sphere => ray_sphere(origin, dir, [px, py + sy / 2.0, pz], round / 2.0),
        ShapeKind::Cylinder => ray_cylinder(origin, dir, e.position, round / 2.0, sy),
    }
}

/// What a [`cast`] hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    /// Where along the ray, and the surface's normal.
    pub hit: RayHit,
    /// The entity whose shape was hit, `None` for the terrain.
    pub entity: Option<EntityId>,
}

/// The nearest hit within `max_m` of a ray with a unit `dir`: the terrain (the triangles of
/// `heights`), the surface of every `road` on it (the ground lifted by the road's offset over
/// its footprint, exactly as drawn), and the `shape` and the model's collider box
/// ([`World::model_box`]) of every entity, solid or not, except those `skip` names; on a tie
/// the terrain, then the lowest id (an entity's shape before its model). A hit whose point is
/// not finite is no hit.
pub fn cast(
    world: &World,
    heights: Option<&HeightField>,
    origin: [f64; 3],
    dir: [f64; 3],
    max_m: f64,
    skip: impl Fn(EntityId) -> bool,
) -> Option<Found> {
    let mut best: Option<Found> = None;
    if let Some(field) = heights
        && let Some(t) = field.raycast(origin, dir, max_m)
    {
        let [x, _, z] = along(origin, dir, t);
        let normal = field.normal_at(x, z).unwrap_or([0.0, 1.0, 0.0]);
        best = Some(Found {
            hit: RayHit {
                distance_m: t,
                normal,
            },
            entity: None,
        });
    }
    // Roads are gathered in the one pass over the entities and cast against together after.
    let mut roads: Vec<(&Entity, &Road)> = Vec::new();
    for e in world.entities() {
        if skip(e.id) {
            continue;
        }
        if let Some(road) = e.road() {
            roads.push((e, road));
        }
        if e.shape().is_none() && e.model().is_none() {
            continue;
        }
        let shape_hit = e.shape().and_then(|shape| ray_shape(origin, dir, e, shape));
        let model_hit = world
            .model_box(e)
            .and_then(|b| ray_box(origin, dir, b.center, b.half, b.yaw_deg));
        for hit in [shape_hit, model_hit].into_iter().flatten() {
            if hit.distance_m > max_m {
                continue;
            }
            if best.is_none_or(|b| hit.distance_m < b.hit.distance_m) {
                best = Some(Found {
                    hit,
                    entity: Some(e.id),
                });
            }
        }
    }
    if let Some(field) = heights
        && !roads.is_empty()
        && let Some((t, id)) = raycast_roads(field, &roads, origin, dir, max_m)
        && best.is_none_or(|b| {
            t < b.hit.distance_m
                || (t == b.hit.distance_m && b.entity.is_some_and(|other| id < other))
        })
    {
        let [x, _, z] = along(origin, dir, t);
        best = Some(Found {
            hit: RayHit {
                distance_m: t,
                normal: field.normal_at(x, z).unwrap_or([0.0, 1.0, 0.0]),
            },
            entity: Some(id),
        });
    }
    best.filter(|f| {
        along(origin, dir, f.hit.distance_m)
            .iter()
            .all(|c| c.is_finite())
    })
}

/// A hit this close to the far end of a line of sight does not block it (the ground or the
/// surface the target point lies on), metres.
pub const LINE_OF_SIGHT_END_TOLERANCE_M: f64 = 0.001;

/// The answer of [`line_of_sight`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    /// Distance from `from` to `to`, metres.
    pub distance_m: f64,
    /// What blocks the line, `None` when it is clear.
    pub blocked_by: Option<Found>,
    /// The unit direction from `from` to `to` (`[0, 0, 0]` when they coincide).
    pub direction: [f64; 3],
}

/// Whether the straight line from `from` to `to` is clear, with the hit rules of [`cast`]: a hit
/// within [`LINE_OF_SIGHT_END_TOLERANCE_M`] of `to` does not block, a `from` inside a shape is
/// blocked at distance 0. `None` when the points are too far apart to measure.
pub fn line_of_sight(
    world: &World,
    heights: Option<&HeightField>,
    from: [f64; 3],
    to: [f64; 3],
    skip: impl Fn(EntityId) -> bool,
) -> Option<Sight> {
    let delta = sub(to, from);
    if delta.iter().any(|c| !c.is_finite()) {
        return None;
    }
    let Some(dir) = normalize(delta) else {
        return Some(Sight {
            distance_m: 0.0,
            blocked_by: None,
            direction: [0.0; 3],
        });
    };
    // The length along the unit direction, exact for the axis-aligned cases.
    let distance = dot(dir, delta);
    if !distance.is_finite() {
        return None;
    }
    let reach = distance - LINE_OF_SIGHT_END_TOLERANCE_M;
    let blocked_by = if reach > 0.0 {
        cast(world, heights, from, dir, reach, skip)
    } else {
        // Shorter than the tolerance: only a start inside a shape blocks.
        cast(
            world,
            heights,
            from,
            dir,
            LINE_OF_SIGHT_END_TOLERANCE_M,
            skip,
        )
        .filter(|f| f.hit.distance_m == 0.0 && f.entity.is_some())
    };
    Some(Sight {
        distance_m: distance,
        blocked_by,
        direction: dir,
    })
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
    fn capsule_side_caps_inside_and_misses() {
        let (a, b) = ([0.0, 0.3, 0.0], [0.0, 1.5, 0.0]);
        let h = ray_capsule([5.0, 1.0, 0.0], [-1.0, 0.0, 0.0], a, b, 0.3).unwrap();
        assert!(
            close(h.distance_m, 4.7) && close3(h.normal, [1.0, 0.0, 0.0]),
            "{h:?}"
        );
        let h = ray_capsule([0.0, 9.0, 0.0], [0.0, -1.0, 0.0], a, b, 0.3).unwrap();
        assert!(
            close(h.distance_m, 7.2) && close3(h.normal, [0.0, 1.0, 0.0]),
            "{h:?}"
        );
        let h = ray_capsule([0.0, 1.0, 0.1], [1.0, 0.0, 0.0], a, b, 0.3).unwrap();
        assert_eq!(h.distance_m, 0.0);
        assert!(ray_capsule([5.0, 2.0, 0.0], [-1.0, 0.0, 0.0], a, b, 0.3).is_none());
        assert!(ray_capsule([5.0, 1.0, 0.0], [1.0, 0.0, 0.0], a, b, 0.3).is_none());
        // A slanted ray grazing the top cap.
        let h = ray_capsule([3.0, 1.5 + 0.2, 0.0], [-1.0, 0.0, 0.0], a, b, 0.3).unwrap();
        assert!(h.distance_m > 2.7 && h.distance_m < 3.0, "{h:?}");
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
                    ray_capsule(o, d, big, [0.0; 3], 1e300),
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
