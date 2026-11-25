//! What the models of a frame drew, counted again on the CPU by the renderer's own rule, for
//! the numbers an agent reads with a picture ([`crate::ModelStats`]): the renderer counts the
//! instances and triangles of a frame, but not per model and per level, and an agent cannot
//! look at pictures.
//!
//! The rule of VisionCraft's renderer for an item with levels (`add_lod_item`, `lod.rs`
//! there): the instances are grouped in cells of 8 m on the ground; a cell is drawn at the
//! level for the distance from the eye to the cell's box (the nearest of its points, the box
//! of every level's mesh under every instance of the cell), and not at all beyond the last
//! distance; a cell whose box lies outside the view, beyond one of the six clip planes, is
//! not drawn. A picture starts settled (no hysteresis, no dissolve between two levels), so
//! the count of a picture is exact; a live frame differs for the few frames a hand-over
//! dissolves over, and where a camera lingers on a threshold. A live frame at a lower quality
//! divides every distance by the quality's share (40 % low, 50 % medium; a picture is drawn at
//! the highest, 100 %): the count of a live view does the same.

use glam::{Mat4, Vec3, Vec4};
use std::collections::BTreeMap;

/// The side of a cell of instances the renderer decides by, metres.
const CELL_M: f32 = 8.0;

/// The level a distance draws at: the first whose end is at or beyond it; `None` beyond the
/// last.
pub(crate) fn select(d: f32, until: &[f32]) -> Option<usize> {
    until.iter().position(|&end| d <= end)
}

/// The box holding two boxes.
fn union(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> (Vec3, Vec3) {
    (a.0.min(b.0), a.1.max(b.1))
}

/// The box holding a box moved and scaled by `m`: its eight corners through the matrix.
fn transform_bounds(m: Mat4, b: (Vec3, Vec3)) -> (Vec3, Vec3) {
    let corners = (0..8).map(|k| {
        Vec3::new(
            if k & 1 == 0 { b.0.x } else { b.1.x },
            if k & 2 == 0 { b.0.y } else { b.1.y },
            if k & 4 == 0 { b.0.z } else { b.1.z },
        )
    });
    // The first corner's box, grown by the others (the box itself is not moved along).
    let mut corners = corners.map(|c| m.transform_point3(c));
    let first = corners.next().map_or(b, |p| (p, p));
    corners.fold(first, |out, p| union(out, (p, p)))
}

/// Whether the box `lo..hi` lies entirely beyond one clip plane of the view-projection
/// `view_proj` (the near and far planes included): true when nothing of it is in the view.
/// The test of a box by its centre and its extents (the p-vertex test: for each plane, the
/// corner of the box furthest inside it), the same one the renderer applies, so the count
/// matches what it draws.
pub(crate) fn outside(view_proj: Mat4, b: (Vec3, Vec3)) -> bool {
    let (c, e) = ((b.0 + b.1) * 0.5, (b.1 - b.0) * 0.5);
    let rows = view_proj.transpose();
    let (x, y, z, w) = (rows.x_axis, rows.y_axis, rows.z_axis, rows.w_axis);
    let beyond = |n: Vec4| n.truncate().dot(c) + n.w + n.truncate().abs().dot(e) < 0.0;
    beyond(w + x) || beyond(w - x) || beyond(w + y) || beyond(w - y) || beyond(z) || beyond(w - z)
}

/// What one model item drew in a view: how many instances, and per level how many with
/// their triangles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Count {
    pub drawn: usize,
    pub culled: usize,
    /// (instances, triangles) per level, finest first.
    pub levels: Vec<(usize, u64)>,
}

/// Counts what the renderer draws of one model item in the view from `eye` through
/// `view_proj` (`projection * view`): `instances` are its instance matrices, `triangles`
/// the triangles each level submits per instance, `bounds` the box of the item's meshes in
/// their own axes, `until_m` where each level ends (the last: the draw distance) and
/// `lod_scale` what the renderer divides the distances by (1 for a picture; the live
/// quality's share for a live frame).
pub(crate) fn count(
    instances: &[Mat4],
    triangles: &[u64],
    bounds: (Vec3, Vec3),
    until_m: &[f64],
    view_proj: Mat4,
    eye: Vec3,
    lod_scale: f32,
) -> Count {
    // The renderer scales the distance, not the ends: the same choice.
    let lod_scale = if lod_scale.is_finite() && lod_scale > 0.0 {
        lod_scale
    } else {
        1.0
    };
    let until: Vec<f32> = until_m.iter().map(|&end| end as f32).collect();
    let mut cells: BTreeMap<(i32, i32), ((Vec3, Vec3), usize)> = BTreeMap::new();
    for &m in instances {
        let key = (
            (m.w_axis.x / CELL_M).floor() as i32,
            (m.w_axis.z / CELL_M).floor() as i32,
        );
        let moved = transform_bounds(m, bounds);
        cells
            .entry(key)
            .and_modify(|(box_, n)| {
                *box_ = union(*box_, moved);
                *n += 1;
            })
            .or_insert((moved, 1));
    }
    let mut out = Count {
        levels: vec![(0, 0); until.len()],
        ..Count::default()
    };
    for ((lo, hi), n) in cells.values() {
        if outside(view_proj, (*lo, *hi)) {
            out.culled += n;
            continue;
        }
        // The nearest point of the box to the eye (never a panic on a box that is not one).
        let nearest = eye.max(*lo).min(*hi);
        match select(nearest.distance(eye) / lod_scale, &until) {
            Some(k) => {
                out.drawn += n;
                if let Some(slot) = out.levels.get_mut(k) {
                    slot.0 += n;
                    slot.1 += *n as u64 * triangles.get(k).copied().unwrap_or(0);
                }
            }
            None => out.culled += n,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An instance at `x` on the ground: a unit box.
    fn at(x: f32) -> Mat4 {
        Mat4::from_translation(Vec3::new(x, 0.0, 0.0))
    }

    /// A view-projection looking down -z from the origin, 90 degrees, 0.1 to 100 m.
    fn looking_down_z() -> Mat4 {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0) * view
    }

    #[test]
    fn the_level_is_the_first_whose_end_reaches_the_distance() {
        let until = [20.0, 60.0, 150.0];
        assert_eq!(select(0.0, &until), Some(0));
        assert_eq!(select(20.0, &until), Some(0));
        assert_eq!(select(20.1, &until), Some(1));
        assert_eq!(select(150.0, &until), Some(2));
        assert_eq!(select(150.1, &until), None);
        assert_eq!(select(1e9, &[f32::INFINITY]), Some(0));
    }

    #[test]
    fn instances_take_the_level_of_their_cell_and_vanish_beyond_the_draw_distance() {
        let (view_proj, eye) = (looking_down_z(), Vec3::ZERO);
        // On the line of sight (z = -d), a unit box under each: near, middle, far, too far.
        let instances: Vec<Mat4> = [10.0, 40.0, 100.0, 1000.0]
            .iter()
            .map(|&d| Mat4::from_translation(Vec3::new(0.0, 0.0, -d)))
            .collect();
        let bounds = (Vec3::splat(-0.5), Vec3::splat(0.5));
        let c = count(
            &instances,
            &[36, 24, 12],
            bounds,
            &[20.0, 60.0, 150.0],
            view_proj,
            eye,
            1.0,
        );
        assert_eq!(c.drawn, 3);
        assert_eq!(c.culled, 1);
        assert_eq!(
            c.levels,
            vec![(1, 36), (1, 24), (1, 12)],
            "the box of the cell counts, not the origin of the instance: 10 - 0.5 is inside 20"
        );
        // Two instances in one cell share a level.
        let pair = [at(1.0), at(2.0)];
        let c = count(
            &pair,
            &[12],
            bounds,
            &[100.0],
            view_proj,
            Vec3::new(0.0, 0.0, 5.0),
            1.0,
        );
        assert_eq!((c.drawn, c.culled, c.levels.clone()), (2, 0, vec![(2, 24)]));
        // No instances: nothing drawn, and still a row per level.
        let c = count(&[], &[12], bounds, &[100.0], view_proj, Vec3::ZERO, 1.0);
        assert_eq!((c.drawn, c.culled, c.levels.clone()), (0, 0, vec![(0, 0)]));
    }

    #[test]
    fn a_box_outside_the_view_is_not_drawn() {
        let view_proj = looking_down_z();
        let box_ = |c: Vec3, h: f32| (c - Vec3::splat(h), c + Vec3::splat(h));
        assert!(!outside(view_proj, box_(Vec3::new(0.0, 0.0, -10.0), 1.0)));
        assert!(
            outside(view_proj, box_(Vec3::new(0.0, 0.0, 10.0), 1.0)),
            "behind"
        );
        assert!(
            outside(view_proj, box_(Vec3::new(0.0, 0.0, -500.0), 1.0)),
            "beyond far"
        );
        assert!(
            outside(view_proj, box_(Vec3::new(-40.0, 0.0, -10.0), 1.0)),
            "left out"
        );
        assert!(
            !outside(view_proj, box_(Vec3::new(-10.0, 0.0, -10.0), 1.0)),
            "at the edge, partly in"
        );
        // In the count: behind the camera is culled like beyond the draw distance.
        let c = count(
            &[Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0))],
            &[12],
            (Vec3::splat(-0.5), Vec3::splat(0.5)),
            &[100.0],
            view_proj,
            Vec3::ZERO,
            1.0,
        );
        assert_eq!((c.drawn, c.culled), (0, 1));
        // A live frame at a lower quality divides the distance: at half, a crate 100 m away
        // is beyond a draw distance of 150 m.
        let c = count(
            &[Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0))],
            &[12],
            (Vec3::splat(-0.5), Vec3::splat(0.5)),
            &[150.0],
            view_proj,
            Vec3::ZERO,
            0.5,
        );
        assert_eq!((c.drawn, c.culled), (0, 1), "half the distances");
        let c = count(
            &[Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0))],
            &[12],
            (Vec3::splat(-0.5), Vec3::splat(0.5)),
            &[150.0],
            view_proj,
            Vec3::ZERO,
            f32::NAN,
        );
        assert_eq!(c.drawn, 1, "a scale that is not a number counts as 1");
    }

    #[test]
    fn a_scaled_instance_has_the_box_of_its_size() {
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(10.0),
            glam::Quat::IDENTITY,
            Vec3::new(0.0, 0.0, -50.0),
        );
        let b = transform_bounds(m, (Vec3::splat(-0.5), Vec3::splat(0.5)));
        assert_eq!(b.0, Vec3::new(-5.0, -5.0, -55.0));
        assert_eq!(b.1, Vec3::new(5.0, 5.0, -45.0));
    }
}
