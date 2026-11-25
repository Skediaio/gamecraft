//! Roads: a centre line with a width that follows the terrain (the `road` component), and the
//! surface derived from it.
//!
//! **The recipe** is a polyline in the ground plane (x, z offsets from the entity's position,
//! turned by its `rotation_y_deg` and multiplied by its `scale`), a width (per point if wanted),
//! a colour, a small lift above the ground and a tier. Heights are not stored: the road lies on
//! whatever ground the terrain builds to, so editing the terrain moves the road with it.
//!
//! **The footprint** ([`Road::footprint`]) is a set of convex pieces in the ground plane: a
//! trapezoid per segment, a circular sector on the outer side of every bend (a round join) and
//! a half disc at each end (`ends: round`), so roads that meet end to end at a point leave no
//! gap. Pieces of one road overlap at bends; they are one surface at one height, so that
//! cannot be seen.
//!
//! **The surface** ([`Road::drape`]) is the footprint clipped against the terrain's triangles
//! (the very ones of [`HeightField`]: each cell split along its diagonal), every clipped
//! polygon lifted by [`Road::surface_offset_m`]. It is therefore exactly the ground plus the
//! offset everywhere, with no sample spacing to choose: no ridge of the terrain can poke
//! through it. Normals are the terrain's vertex normals interpolated, so the road is shaded
//! like the ground under it.
//!
//! **Junctions.** Where two roads overlap, their tiers differ (the engine's `road.add` picks the
//! lowest tier no overlapping road uses, `road.retier` redoes every road), and the higher tier
//! lies [`ROAD_TIER_STEP_M`] per tier higher: never coplanar, so no z-fighting, and the order is
//! data an author can set.
//!
//! **Rays** ([`raycast_roads`]) meet the surface exactly as drawn: the ground lifted by a
//! road's offset is cast against, and the hit counts when it lies in the road's footprint.
//!
//! Everything is `f64` with `+`, `-`, `*`, `/` and `sqrt` only: deterministic on every
//! platform. Nothing here is stored in the document.

use crate::error::Ctx;
use crate::limits::{
    MAX_COORDINATE_M, ROAD_DRAPE_WORK_MAX, ROAD_LIFT_MAX_M, ROAD_LIFT_MIN_M, ROAD_POINT_GAP_MIN_M,
    ROAD_POINTS_MAX, ROAD_POINTS_MIN, ROAD_TIER_MAX, ROAD_TIER_STEP_M, ROAD_WIDTH_MAX_M,
    ROAD_WIDTH_MIN_M,
};
use crate::{Entity, EntityId, HeightField, World, WorldError};
use gamecraft_math::{Portable, deg_to_rad};
use serde::{Deserialize, Serialize};

/// The cosine of the largest angle one straight edge of a round join or end spans (22.5
/// degrees: 16 edges for a full turn).
const ARC_MIN_COS: f64 = 0.923_879_532_511_286_7;
/// Deepest halving of an arc (2^6 edges at most; a half turn needs 8).
const ARC_MAX_DEPTH: u32 = 6;
/// A clipped polygon with less area than this, in square cells, is dropped.
const MIN_CELL_AREA: f64 = 1e-12;
/// Two directions whose cross product is within this are parallel.
const PARALLEL: f64 = 1e-12;

/// How a road ends at its first and last point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoadEnds {
    /// A half disc: roads that meet end to end at a point join without a gap.
    #[default]
    Round,
    /// Cut square across the last segment.
    Flat,
}

impl RoadEnds {
    /// Both, in the order the engine lists them.
    pub const ALL: [RoadEnds; 2] = [RoadEnds::Round, RoadEnds::Flat];

    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            RoadEnds::Round => "round",
            RoadEnds::Flat => "flat",
        }
    }
}

fn default_width() -> f64 {
    6.0
}

fn default_color() -> [f32; 3] {
    [0.05, 0.05, 0.055]
}

fn default_lift() -> f64 {
    0.03
}

/// A road: a centre line with a width, lying on the terrain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Road {
    /// The centre line, `[x, z]` offsets from the entity's position in metres (turned by its
    /// `rotation_y_deg`, times its scale), 2 to 4096 points, consecutive points at least
    /// 0.01 m apart. Required.
    pub points: Vec<[f64; 2]>,
    /// Width, metres, 0.1 to 200 (times the entity's scale). Default 6.
    #[serde(default = "default_width")]
    pub width_m: f64,
    /// A width per point, metres, each 0.1 to 200, replacing `width_m`; empty (the default)
    /// or exactly one per point. The width changes linearly along a segment.
    #[serde(default)]
    pub widths_m: Vec<f64>,
    /// Linear RGB, each channel 0 to 1. Default asphalt `[0.05, 0.05, 0.055]`.
    #[serde(default = "default_color")]
    pub color: [f32; 3],
    /// Height of the surface above the ground, metres, 0.005 to 1. Default 0.03.
    #[serde(default = "default_lift")]
    pub lift_m: f64,
    /// Which road is on top where roads overlap, 0 to 7: each tier lies 0.01 m higher.
    /// Default 0.
    #[serde(default)]
    pub tier: u32,
    /// `round` (default) or `flat`.
    #[serde(default)]
    pub ends: RoadEnds,
}

impl Road {
    /// A road along `points` with every other field at its default.
    pub fn new(points: Vec<[f64; 2]>) -> Road {
        Road {
            points,
            width_m: default_width(),
            widths_m: Vec::new(),
            color: default_color(),
            lift_m: default_lift(),
            tier: 0,
            ends: RoadEnds::Round,
        }
    }

    pub(crate) fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        let n = self.points.len();
        if !(ROAD_POINTS_MIN..=ROAD_POINTS_MAX).contains(&n) {
            return Err(ctx.invalid(
                "road.points",
                format!("{n} points"),
                format!(
                    "a road has {ROAD_POINTS_MIN} to {ROAD_POINTS_MAX} points; split a longer \
                     one into several roads"
                ),
            ));
        }
        for (i, point) in self.points.iter().enumerate() {
            for (axis, &v) in point.iter().enumerate() {
                ctx.range(
                    format!("road.points[{i}][{axis}]"),
                    v,
                    -MAX_COORDINATE_M,
                    MAX_COORDINATE_M,
                    " metres",
                )?;
            }
        }
        for (i, pair) in self.points.windows(2).enumerate() {
            let &[a, b] = pair else { continue };
            let gap = length(sub(b, a));
            if gap.is_nan() || gap < ROAD_POINT_GAP_MIN_M {
                return Err(ctx.invalid(
                    format!("road.points[{}]", i + 1),
                    format!("[{}, {}], {gap} m from the point before it", b[0], b[1]),
                    format!(
                        "consecutive points must be at least {ROAD_POINT_GAP_MIN_M} m apart; \
                         leave out the repeated point"
                    ),
                ));
            }
        }
        ctx.range(
            "road.width_m",
            self.width_m,
            ROAD_WIDTH_MIN_M,
            ROAD_WIDTH_MAX_M,
            " metres",
        )?;
        if !self.widths_m.is_empty() && self.widths_m.len() != n {
            return Err(ctx.invalid(
                "road.widths_m",
                format!("{} widths for {n} points", self.widths_m.len()),
                "give exactly one width per point, or none to use road.width_m everywhere",
            ));
        }
        for (i, &w) in self.widths_m.iter().enumerate() {
            ctx.range(
                format!("road.widths_m[{i}]"),
                w,
                ROAD_WIDTH_MIN_M,
                ROAD_WIDTH_MAX_M,
                " metres",
            )?;
        }
        for (channel, &c) in self.color.iter().enumerate() {
            ctx.range(
                format!("road.color[{channel}]"),
                f64::from(c),
                0.0,
                1.0,
                " (linear RGB)",
            )?;
        }
        ctx.range(
            "road.lift_m",
            self.lift_m,
            ROAD_LIFT_MIN_M,
            ROAD_LIFT_MAX_M,
            " metres",
        )?;
        if self.tier > ROAD_TIER_MAX {
            return Err(ctx.invalid(
                "road.tier",
                self.tier,
                format!("it must be a whole number from 0 to {ROAD_TIER_MAX}"),
            ));
        }
        Ok(())
    }

    /// How high the surface lies above the ground, metres: the lift plus
    /// [`ROAD_TIER_STEP_M`] per tier.
    pub fn surface_offset_m(&self) -> f64 {
        self.lift_m + f64::from(self.tier.min(ROAD_TIER_MAX)) * ROAD_TIER_STEP_M
    }

    /// The width at point `i`, metres, before the entity's scale.
    pub fn width_at(&self, i: usize) -> f64 {
        self.widths_m.get(i).copied().unwrap_or(self.width_m)
    }

    /// The centre line in world `[x, z]`, metres, with half the width at each point.
    fn nodes(&self, e: &Entity) -> Vec<Node> {
        let (s, c) = deg_to_rad(e.rotation_y_deg).psin_cos();
        let [px, _, pz] = e.position;
        self.points
            .iter()
            .enumerate()
            .map(|(i, &[x, z])| {
                // The points follow the scale along x and z; the width, which lies across
                // the road whichever way it runs, the middle of the three.
                let (lx, lz) = (x * e.scale.x, z * e.scale.z);
                Node {
                    at: [px + lx * c + lz * s, pz - lx * s + lz * c],
                    half: self.width_at(i) * e.scale.middle() / 2.0,
                }
            })
            .collect()
    }

    /// The centre line of the road on `e` in world `[x, z]`, metres.
    pub fn centre_line(&self, e: &Entity) -> Vec<[f64; 2]> {
        self.nodes(e).into_iter().map(|n| n.at).collect()
    }

    /// The length of the centre line of the road on `e`, metres.
    pub fn length_m(&self, e: &Entity) -> f64 {
        self.nodes(e)
            .windows(2)
            .map(|w| match w {
                [a, b] => length(sub(b.at, a.at)),
                _ => 0.0,
            })
            .sum()
    }

    /// The smallest world `[x, z]` box around the road on `e`: `(min, max)`. `None` for a
    /// road without points or with a number that is not finite.
    pub fn bounds(&self, e: &Entity) -> Option<([f64; 2], [f64; 2])> {
        let nodes = self.nodes(e);
        nodes.first()?;
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for n in &nodes {
            let [x, z] = n.at;
            lo = [lo[0].min(x - n.half), lo[1].min(z - n.half)];
            hi = [hi[0].max(x + n.half), hi[1].max(z + n.half)];
        }
        let finite = lo.iter().chain(&hi).all(|v| v.is_finite());
        finite.then_some((lo, hi))
    }

    /// The road on `e` as convex pieces in world `[x, z]`: a trapezoid per segment, a sector
    /// on the outer side of every bend and, with round ends, a half disc at each end.
    pub fn footprint(&self, e: &Entity) -> Vec<Vec<[f64; 2]>> {
        self.pieces(e)
            .into_iter()
            .map(|p| match p {
                Piece::Segment(quad) => quad.to_vec(),
                Piece::Round(ring) => ring,
            })
            .collect()
    }

    /// [`Road::footprint`], segments kept apart from the round pieces.
    fn pieces(&self, e: &Entity) -> Vec<Piece> {
        let nodes = self.nodes(e);
        let segments: Vec<Segment> = nodes
            .windows(2)
            .filter_map(|w| {
                let &[a, b] = w else { return None };
                Some(Segment {
                    a,
                    b,
                    u: unit(sub(b.at, a.at))?,
                })
            })
            .collect();
        let mut pieces = Vec::with_capacity(segments.len() * 2 + 2);
        for s in &segments {
            let n = perp(s.u);
            pieces.push(Piece::Segment([
                add(s.a.at, scale(n, s.a.half)),
                add(s.b.at, scale(n, s.b.half)),
                sub(s.b.at, scale(n, s.b.half)),
                sub(s.a.at, scale(n, s.a.half)),
            ]));
        }
        for pair in segments.windows(2) {
            let [s0, s1] = pair else { continue };
            let (u0, u1) = (s0.u, s1.u);
            let turn = cross(u0, u1);
            let (from, to, through) = if turn > PARALLEL {
                (neg(perp(u0)), neg(perp(u1)), None)
            } else if turn < -PARALLEL {
                (perp(u0), perp(u1), None)
            } else if dot(u0, u1) < 0.0 {
                // Turning back: the whole half disc ahead is open.
                (perp(u0), neg(perp(u0)), Some(u0))
            } else {
                continue;
            };
            pieces.push(Piece::Round(sector(s1.a, from, to, through)));
        }
        if self.ends == RoadEnds::Round
            && let (Some(first), Some(last)) = (segments.first(), segments.last())
        {
            let n = perp(first.u);
            pieces.push(Piece::Round(sector(first.a, n, neg(n), Some(neg(first.u)))));
            let n = perp(last.u);
            pieces.push(Piece::Round(sector(last.b, n, neg(n), Some(last.u))));
        }
        pieces
    }

    /// Whether world `x`, `z` lies on the road on `e` (in one of its [`Road::footprint`]
    /// pieces, edges included).
    pub fn contains(&self, e: &Entity, x: f64, z: f64) -> bool {
        let Some((lo, hi)) = self.bounds(e) else {
            return false;
        };
        if x < lo[0] || x > hi[0] || z < lo[1] || z > hi[1] {
            return false;
        }
        self.footprint(e)
            .iter()
            .any(|piece| convex_contains(piece, [x, z]))
    }

    /// Whether the road on `e` and the road `other` on `other_e` overlap or touch.
    pub fn overlaps(&self, e: &Entity, other: &Road, other_e: &Entity) -> bool {
        let (Some(a), Some(b)) = (self.bounds(e), other.bounds(other_e)) else {
            return false;
        };
        if !boxes_meet(a, b) {
            return false;
        }
        let mine: Vec<(Box2, Vec<[f64; 2]>)> = self
            .footprint(e)
            .into_iter()
            .filter_map(|p| Some((box_of(&p)?, p)))
            .filter(|(bx, _)| boxes_meet(*bx, b))
            .collect();
        other.footprint(other_e).iter().any(|q| {
            let Some(qb) = box_of(q) else { return false };
            mine.iter()
                .any(|(pb, p)| boxes_meet(*pb, qb) && convex_overlap(p, q))
        })
    }

    /// Builds the surface of the road on `e` over the terrain `field` and hands each polygon
    /// to `emit`, in a fixed order: convex, counter-clockwise seen from above, every vertex at
    /// the ground plus [`Road::surface_offset_m`], with the terrain's interpolated normal.
    /// Parts of the road beyond the terrain are left out. An error, before anything is
    /// emitted, when the road would take more than [`ROAD_DRAPE_WORK_MAX`] clips.
    pub fn drape(
        &self,
        e: &Entity,
        field: &HeightField,
        emit: &mut dyn FnMut(&DrapedPolygon<'_>),
    ) -> Result<DrapeStats, WorldError> {
        let cell = field.cell_m();
        let too_big = |what: String| WorldError::Invalid {
            entity: Some(e.id),
            field: "road".into(),
            value: what,
            expected: format!(
                "a road may cover at most {} terrain triangles; split it into shorter roads",
                ROAD_DRAPE_WORK_MAX
            ),
        };
        if !(cell.is_finite() && cell > 0.0) || field.cols() < 2 || field.rows() < 2 {
            return Ok(DrapeStats::default());
        }
        // The pieces, segments cut into slices about a cell long, so a slice's box holds few
        // cells. Slice corners are computed once per segment, so neighbours share them exactly.
        let mut pieces: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut slices_total: f64 = 0.0;
        for piece in self.pieces(e) {
            let [l0, l1, r1, r0] = match piece {
                Piece::Segment(quad) => quad,
                Piece::Round(ring) => {
                    pieces.push(ring);
                    continue;
                }
            };
            let len = length(sub(l1, l0)).max(length(sub(r1, r0)));
            let slices = (len / cell).ceil().max(1.0);
            slices_total += slices;
            if slices_total.is_nan() || slices_total > ROAD_DRAPE_WORK_MAX as f64 {
                return Err(too_big(format!("{slices_total} slices or more")));
            }
            let k = slices as usize;
            let at = |a: [f64; 2], b: [f64; 2], j: usize| -> [f64; 2] {
                if j == 0 {
                    a
                } else if j == k {
                    b
                } else {
                    add(a, scale(sub(b, a), j as f64 / k as f64))
                }
            };
            for j in 0..k {
                pieces.push(vec![
                    at(l0, l1, j),
                    at(l0, l1, j + 1),
                    at(r0, r1, j + 1),
                    at(r0, r1, j),
                ]);
            }
        }

        let [x0, z0] = field.min();
        let last_col = field.cols() - 2;
        let last_row = field.rows() - 2;
        // The cells under each piece, in grid coordinates; the work is counted first.
        let mut spans = Vec::with_capacity(pieces.len());
        let mut work: u64 = 0;
        for piece in &pieces {
            let grid: Vec<[f64; 2]> = piece
                .iter()
                .map(|&[x, z]| [(x - x0) / cell, (z - z0) / cell])
                .collect();
            let span = box_of(&grid).and_then(|(lo, hi)| {
                let c0 = lo[0].floor().max(0.0);
                let r0 = lo[1].floor().max(0.0);
                let c1 = hi[0].floor().min(last_col as f64);
                let r1 = hi[1].floor().min(last_row as f64);
                (c0 <= c1 && r0 <= r1).then_some((
                    c0 as usize,
                    r0 as usize,
                    c1 as usize,
                    r1 as usize,
                ))
            });
            if let Some((c0, r0, c1, r1)) = span {
                let cells = ((c1 - c0 + 1) as u64).saturating_mul((r1 - r0 + 1) as u64);
                work = work.saturating_add(cells.saturating_mul(2));
                if work > ROAD_DRAPE_WORK_MAX {
                    return Err(too_big(format!("{work} terrain triangles or more")));
                }
            }
            spans.push((grid, span));
        }

        let offset = self.surface_offset_m();
        let mut stats = DrapeStats {
            pieces: pieces.len() as u64,
            clips: work,
            ..DrapeStats::default()
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        for (grid, span) in spans {
            let Some((c0, r0, c1, r1)) = span else {
                continue;
            };
            for row in r0..=r1 {
                for col in c0..=c1 {
                    let Some(corners) = Corners::of(field, col, row) else {
                        continue;
                    };
                    let (fc, fr) = (col as f64, row as f64);
                    let local: Vec<[f64; 2]> =
                        grid.iter().map(|&[u, v]| [u - fc, v - fr]).collect();
                    for lower in [true, false] {
                        // The triangle with (col + 1, row) or the one with (col, row + 1).
                        let planes: [[f64; 3]; 3] = if lower {
                            [[0.0, 1.0, 0.0], [-1.0, 0.0, 1.0], [1.0, -1.0, 0.0]]
                        } else {
                            [[1.0, 0.0, 0.0], [0.0, -1.0, 1.0], [-1.0, 1.0, 0.0]]
                        };
                        a.clone_from(&local);
                        for plane in planes {
                            clip(&a, plane, &mut b);
                            std::mem::swap(&mut a, &mut b);
                            if a.len() < 3 {
                                break;
                            }
                        }
                        dedup_ring(&mut a);
                        if a.len() < 3 || ring_area(&a).abs() < MIN_CELL_AREA {
                            continue;
                        }
                        positions.clear();
                        normals.clear();
                        for &[u, v] in &a {
                            let (h, n) = corners.at(lower, u, v);
                            positions.push([
                                x0 + (fc + u) * cell,
                                h + offset,
                                z0 + (fr + v) * cell,
                            ]);
                            normals.push(n);
                        }
                        if facing(&positions) < 0.0 {
                            positions.reverse();
                            normals.reverse();
                        }
                        stats.polygons += 1;
                        stats.triangles += positions.len() as u64 - 2;
                        emit(&DrapedPolygon {
                            col,
                            row,
                            positions: &positions,
                            normals: &normals,
                        });
                    }
                }
            }
        }
        Ok(stats)
    }

    /// The whole surface of the road on `e` as one indexed mesh ([`Road::drape`], each
    /// polygon a fan).
    pub fn mesh(&self, e: &Entity, field: &HeightField) -> Result<RoadMesh, WorldError> {
        let mut mesh = RoadMesh::default();
        let stats = self.drape(e, field, &mut |p| mesh.push(p))?;
        mesh.stats = stats;
        Ok(mesh)
    }
}

/// One polygon of a road's surface, inside one triangle of the terrain cell (`col`, `row`).
#[derive(Clone, Copy, Debug)]
pub struct DrapedPolygon<'a> {
    /// The terrain cell's column.
    pub col: usize,
    /// The terrain cell's row.
    pub row: usize,
    /// World positions, metres: convex, counter-clockwise seen from above.
    pub positions: &'a [[f64; 3]],
    /// Unit upward normals, one per position.
    pub normals: &'a [[f64; 3]],
}

/// What building a road's surface took and gave (AGENTS.md rule 18).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrapeStats {
    /// Convex pieces of the footprint, segments cut into slices.
    pub pieces: u64,
    /// Terrain triangles the pieces were clipped against.
    pub clips: u64,
    /// Polygons of the surface.
    pub polygons: u64,
    /// Triangles of the surface (each polygon a fan).
    pub triangles: u64,
}

/// A road's surface as one indexed triangle mesh, world positions in metres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadMesh {
    /// World positions.
    pub positions: Vec<[f64; 3]>,
    /// Unit normals, one per position.
    pub normals: Vec<[f64; 3]>,
    /// Three indices per triangle, counter-clockwise seen from above.
    pub indices: Vec<u32>,
    /// What it took.
    pub stats: DrapeStats,
}

impl RoadMesh {
    /// Appends a polygon as a fan.
    pub fn push(&mut self, p: &DrapedPolygon<'_>) {
        let Ok(base) = u32::try_from(self.positions.len()) else {
            return;
        };
        let Ok(n) = u32::try_from(p.positions.len()) else {
            return;
        };
        if base.checked_add(n).is_none() {
            return;
        }
        self.positions.extend_from_slice(p.positions);
        self.normals.extend_from_slice(p.normals);
        for i in 1..n.saturating_sub(1) {
            self.indices
                .extend_from_slice(&[base, base + i, base + i + 1]);
        }
    }

    /// The triangles, by their three positions.
    pub fn triangles(&self) -> impl Iterator<Item = [[f64; 3]; 3]> + '_ {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|&[a, b, c]| {
                let p = |k: u32| self.positions.get(k as usize).copied();
                Some([p(a)?, p(b)?, p(c)?])
            })
    }
}

/// The nearest hit of a ray on the surface of one of `roads` (each with the entity it is on)
/// over the terrain `field`, within `max_m`: `(distance along the unit direction, entity)`; on
/// a tie the lowest id. `direction` has any non-zero length.
///
/// The surface of a road is the ground lifted by its offset, over its footprint. Roads are
/// grouped by offset; per group the lifted ground is cast against once ([`HeightField::raycast`],
/// exact for the triangles) and the hit counts for the lowest-id road of the group whose
/// footprint holds it. A ray that dips under a lifted surface beside a road and comes up
/// again before reaching it is not followed further: it is within the lift (centimetres) of
/// the ground there.
pub fn raycast_roads(
    field: &HeightField,
    roads: &[(&Entity, &Road)],
    origin: [f64; 3],
    direction: [f64; 3],
    max_m: f64,
) -> Option<(f64, EntityId)> {
    let [dx, dy, dz] = direction;
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if !(len.is_finite() && len > 0.0 && max_m.is_finite() && max_m > 0.0) {
        return None;
    }
    let dir = [dx / len, dy / len, dz / len];
    let [ox, oy, oz] = origin;
    let end = [ox + dir[0] * max_m, oz + dir[2] * max_m];
    let reach = (
        [ox.min(end[0]), oz.min(end[1])],
        [ox.max(end[0]), oz.max(end[1])],
    );
    let mut near: Vec<(f64, &Entity, &Road)> = roads
        .iter()
        .filter(|(e, r)| r.bounds(e).is_some_and(|b| boxes_meet(b, reach)))
        .map(|&(e, r)| (r.surface_offset_m(), e, r))
        .collect();
    near.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let mut best: Option<(f64, EntityId)> = None;
    let mut rest = near.as_slice();
    while let Some(&(offset, _, _)) = rest.first() {
        let split = rest
            .iter()
            .position(|(o, _, _)| o.to_bits() != offset.to_bits())
            .unwrap_or(rest.len());
        let (group, after) = rest.split_at(split);
        rest = after;
        let Some(t) = field.raycast([ox, oy - offset, oz], dir, max_m) else {
            continue;
        };
        if best.is_some_and(|(b, _)| b < t) {
            continue;
        }
        let (x, z) = (ox + dir[0] * t, oz + dir[2] * t);
        // The group is in id order: the first road holding the point is the lowest id.
        if let Some((_, e, _)) = group.iter().find(|(_, e, r)| r.contains(e, x, z))
            && best.is_none_or(|(b, id)| t < b || (t == b && e.id < id))
        {
            best = Some((t, e.id));
        }
    }
    best
}

/// The lowest tier no road of `world` that overlaps the road `road` on `e` uses (`e` itself,
/// by id, left out); [`ROAD_TIER_MAX`] when every tier is taken there.
pub fn free_road_tier(world: &World, e: &Entity, road: &Road) -> u32 {
    let mut used = [false; ROAD_TIER_MAX as usize + 1];
    for other in world.entities() {
        if other.id == e.id {
            continue;
        }
        let Some(r) = other.road() else { continue };
        if let Some(slot) = used.get_mut(r.tier as usize)
            && !*slot
            && road.overlaps(e, r, other)
        {
            *slot = true;
        }
    }
    used.iter()
        .position(|u| !u)
        .map_or(ROAD_TIER_MAX, |t| t as u32)
}

/// What [`retier_roads`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retiered {
    /// Roads in the world.
    pub roads: u64,
    /// Roads whose tier changed.
    pub changed: u64,
    /// The highest tier given.
    pub highest: u32,
    /// Pairs of roads found to overlap.
    pub overlaps: u64,
}

/// Gives every road of `world` a tier again: in id order, the lowest tier no road before it
/// that overlaps it uses ([`ROAD_TIER_MAX`] when all are taken). Candidate pairs are found by a
/// sweep over the roads' boxes along x, so a city of thousands of roads is quick.
pub fn retier_roads(world: &mut World) -> Retiered {
    struct Item {
        id: EntityId,
        bounds: ([f64; 2], [f64; 2]),
        entity: Entity,
        road: Road,
    }
    let items: Vec<Item> = world
        .entities()
        .filter_map(|e| {
            let road = e.road()?.clone();
            Some(Item {
                id: e.id,
                bounds: road.bounds(e)?,
                entity: e.clone(),
                road,
            })
        })
        .collect();
    // Candidate pairs: boxes that meet, by a sweep along x.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let (ka, kb) = (items.get(a), items.get(b));
        let xa = ka.map_or(0.0, |i| i.bounds.0[0]);
        let xb = kb.map_or(0.0, |i| i.bounds.0[0]);
        xa.total_cmp(&xb).then(a.cmp(&b))
    });
    let mut earlier: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    let mut active: Vec<usize> = Vec::new();
    let mut overlaps = 0u64;
    for &i in &order {
        let Some(item) = items.get(i) else { continue };
        active.retain(|&j| {
            items
                .get(j)
                .is_some_and(|o| o.bounds.1[0] >= item.bounds.0[0])
        });
        for &j in &active {
            let Some(other) = items.get(j) else { continue };
            if boxes_meet(item.bounds, other.bounds)
                && item.road.overlaps(&item.entity, &other.road, &other.entity)
            {
                overlaps += 1;
                // Items are in id order, so the lower index is the earlier road.
                let (lo, hi) = if i < j { (i, j) } else { (j, i) };
                if let Some(list) = earlier.get_mut(hi) {
                    list.push(lo);
                }
            }
        }
        active.push(i);
    }
    let mut tiers: Vec<u32> = Vec::with_capacity(items.len());
    let mut report = Retiered {
        roads: items.len() as u64,
        overlaps,
        ..Retiered::default()
    };
    for (i, item) in items.iter().enumerate() {
        let mut used = [false; ROAD_TIER_MAX as usize + 1];
        for &j in earlier.get(i).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = tiers.get(j).and_then(|&t| used.get_mut(t as usize)) {
                *slot = true;
            }
        }
        let tier = used
            .iter()
            .position(|u| !u)
            .map_or(ROAD_TIER_MAX, |t| t as u32);
        tiers.push(tier);
        report.highest = report.highest.max(tier);
        if tier != item.road.tier
            && let Some(e) = world.entity_mut(item.id)
            && let Some(r) = e.road_mut()
        {
            r.tier = tier;
            report.changed += 1;
        }
    }
    report
}

/// A convex piece of a footprint.
enum Piece {
    /// A segment's trapezoid: left start, left end, right end, right start.
    Segment([[f64; 2]; 4]),
    /// A round join or end: the centre, then the rim.
    Round(Vec<[f64; 2]>),
}

/// A point of the centre line in world `[x, z]` with half the width there.
#[derive(Clone, Copy, Debug)]
struct Node {
    at: [f64; 2],
    half: f64,
}

/// A segment of the centre line with its unit direction.
#[derive(Clone, Copy, Debug)]
struct Segment {
    a: Node,
    b: Node,
    u: [f64; 2],
}

/// A world `[x, z]` box: `(min, max)`.
type Box2 = ([f64; 2], [f64; 2]);

/// The heights and normals of the four samples of a terrain cell.
struct Corners {
    h: [f64; 4],
    n: [[f64; 3]; 4],
}

impl Corners {
    /// Samples (col, row), (col + 1, row), (col, row + 1), (col + 1, row + 1).
    fn of(field: &HeightField, col: usize, row: usize) -> Option<Corners> {
        let at = [
            (col, row),
            (col + 1, row),
            (col, row + 1),
            (col + 1, row + 1),
        ];
        let mut h = [0.0; 4];
        for (slot, &(c, r)) in h.iter_mut().zip(&at) {
            *slot = f64::from(field.sample(c, r)?);
        }
        Some(Corners {
            h,
            n: at.map(|(c, r)| field.sample_normal(c, r)),
        })
    }

    /// The height and normal at cell coordinates `u`, `v` (0 to 1) on the lower triangle
    /// (with sample (col + 1, row)) or the upper one, as `HeightField::height_at` computes it.
    fn at(&self, lower: bool, u: f64, v: f64) -> (f64, [f64; 3]) {
        let [h00, h10, h01, h11] = self.h;
        let [n00, n10, n01, n11] = self.n;
        let (h, weights) = if lower {
            (
                h00 + u * (h10 - h00) + v * (h11 - h10),
                [(n00, 1.0 - u), (n10, u - v), (n11, v)],
            )
        } else {
            (
                h00 + u * (h11 - h01) + v * (h01 - h00),
                [(n00, 1.0 - v), (n01, v - u), (n11, u)],
            )
        };
        let mut n = [0.0; 3];
        for (corner, w) in weights {
            for (acc, c) in n.iter_mut().zip(corner) {
                *acc += c * w;
            }
        }
        let [x, y, z] = n;
        let len = (x * x + y * y + z * z).sqrt();
        let normal = if len.is_finite() && len > 0.0 {
            n.map(|c| c / len)
        } else {
            [0.0, 1.0, 0.0]
        };
        (h, normal)
    }
}

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] * s, a[1] * s]
}

fn neg(a: [f64; 2]) -> [f64; 2] {
    [-a[0], -a[1]]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// `a` turned a quarter turn in the (x, z) plane.
fn perp(a: [f64; 2]) -> [f64; 2] {
    [-a[1], a[0]]
}

fn length(a: [f64; 2]) -> f64 {
    dot(a, a).sqrt()
}

/// `a` of length 1; `None` for a zero or non-finite vector.
fn unit(a: [f64; 2]) -> Option<[f64; 2]> {
    let len = length(a);
    (len.is_finite() && len > 0.0).then(|| scale(a, 1.0 / len))
}

/// The circular sector around `centre` (radius its half width) from unit direction `from` to
/// `to`, the short way or `through` a given direction: `[centre, rim...]`, convex.
fn sector(centre: Node, from: [f64; 2], to: [f64; 2], through: Option<[f64; 2]>) -> Vec<[f64; 2]> {
    let mut dirs = vec![from];
    match through {
        Some(m) => {
            arc(from, m, &mut dirs, 0);
            arc(m, to, &mut dirs, 0);
        }
        None => arc(from, to, &mut dirs, 0),
    }
    std::iter::once(centre.at)
        .chain(
            dirs.into_iter()
                .map(|d| add(centre.at, scale(d, centre.half))),
        )
        .collect()
}

/// Appends the directions after `a` up to and including `b`, halving until each step spans
/// at most 22.5 degrees.
fn arc(a: [f64; 2], b: [f64; 2], out: &mut Vec<[f64; 2]>, depth: u32) {
    if depth >= ARC_MAX_DEPTH || dot(a, b) >= ARC_MIN_COS {
        out.push(b);
        return;
    }
    let Some(m) = unit(add(a, b)) else {
        out.push(b);
        return;
    };
    arc(a, m, out, depth + 1);
    arc(m, b, out, depth + 1);
}

/// The box around `points`; `None` for none or a number that is not finite.
fn box_of(points: &[[f64; 2]]) -> Option<Box2> {
    let first = *points.first()?;
    let (mut lo, mut hi) = (first, first);
    for p in points {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    lo.iter()
        .chain(&hi)
        .all(|v| v.is_finite())
        .then_some((lo, hi))
}

fn boxes_meet(a: Box2, b: Box2) -> bool {
    a.0[0] <= b.1[0] && b.0[0] <= a.1[0] && a.0[1] <= b.1[1] && b.0[1] <= a.1[1]
}

/// Each edge of a ring with the point after it.
fn edges(ring: &[[f64; 2]]) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
    ring.iter()
        .copied()
        .zip(ring.iter().copied().cycle().skip(1))
}

/// Whether convex `ring` (either orientation) holds `p`, edges included.
fn convex_contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let (mut left, mut right) = (false, false);
    for (a, b) in edges(ring) {
        let c = cross(sub(b, a), sub(p, a));
        if c > 0.0 {
            left = true;
        } else if c < 0.0 {
            right = true;
        }
        if left && right {
            return false;
        }
    }
    true
}

/// Whether two convex rings overlap or touch (separating axes).
fn convex_overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let project = |ring: &[[f64; 2]], axis: [f64; 2]| {
        ring.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &p| {
                let d = dot(p, axis);
                (lo.min(d), hi.max(d))
            })
    };
    for ring in [a, b] {
        for (p, q) in edges(ring) {
            let axis = perp(sub(q, p));
            if axis == [0.0, 0.0] {
                continue;
            }
            let (a0, a1) = project(a, axis);
            let (b0, b1) = project(b, axis);
            if a1 < b0 || b1 < a0 {
                return false;
            }
        }
    }
    true
}

/// Clips `ring` to the half plane `pa * u + pb * v + pc >= 0` into `out` (Sutherland and
/// Hodgman). An edge is cut from its lexicographically smaller end, so the two polygons on
/// either side of a line cut a shared edge at the very same point.
fn clip(ring: &[[f64; 2]], [pa, pb, pc]: [f64; 3], out: &mut Vec<[f64; 2]>) {
    out.clear();
    let side = |p: [f64; 2]| pa * p[0] + pb * p[1] + pc;
    let cut = |p: [f64; 2], q: [f64; 2]| {
        let (lo, hi) = if (p[0], p[1]) <= (q[0], q[1]) {
            (p, q)
        } else {
            (q, p)
        };
        let (fl, fh) = (side(lo), side(hi));
        let t = fl / (fl - fh);
        if t.is_finite() {
            add(lo, scale(sub(hi, lo), t.clamp(0.0, 1.0)))
        } else {
            lo
        }
    };
    let Some(&last) = ring.last() else { return };
    let mut prev = last;
    for &cur in ring {
        let (fp, fc) = (side(prev), side(cur));
        if fc >= 0.0 {
            if fp < 0.0 {
                out.push(cut(prev, cur));
            }
            out.push(cur);
        } else if fp >= 0.0 {
            out.push(cut(prev, cur));
        }
        prev = cur;
    }
}

/// Drops consecutive repeated points of a ring (the last against the first too).
fn dedup_ring(ring: &mut Vec<[f64; 2]>) {
    ring.dedup();
    while ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
}

/// Twice the signed area of a ring in its own coordinates.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    edges(ring).map(|(a, b)| cross(a, b)).sum()
}

/// Positive when a fan over `positions` faces up (counter-clockwise seen from above, +Y).
fn facing(positions: &[[f64; 3]]) -> f64 {
    let Some(&[ax, _, az]) = positions.first() else {
        return 0.0;
    };
    positions
        .windows(2)
        .skip(1)
        .map(|w| match w {
            [[bx, _, bz], [cx, _, cz]] => {
                let (e1x, e1z) = (bx - ax, bz - az);
                let (e2x, e2z) = (cx - ax, cz - az);
                e1z * e2x - e1x * e2z
            }
            _ => 0.0,
        })
        .sum()
}
