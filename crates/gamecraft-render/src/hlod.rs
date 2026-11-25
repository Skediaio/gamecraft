//! Far levels of detail for groups: the static shapes and models of a cell of the ground
//! merged into one mesh per material, drawn where the cell is far and its own items are
//! culled. Per-model levels (step 2) still draw every instance; a district of a hundred
//! thousand draws per pass needs a level per *group*.
//!
//! The ground is cut in cells of [`CELL_M`] metres. A cell's merged mesh holds every static
//! shape under its entity's transform and every static model at its coarsest level, sorted by
//! material; it takes over where the cell is farther than the largest draw distance of its own
//! items (at least [`NEAR_MIN_M`]) and is drawn until [`FAR_M`]. The real items of the cell
//! draw until a cell's diagonal past that, so that a cluster at the cell's far corner never
//! disappears before the merged mesh shows; in that band both draw, and every merged entity is
//! pulled towards its own origin ([`SHRINK`], capped at [`INSET_M`] a side), so its copy stays
//! inside the real one.
//!
//! A cell's content is hashed; a rebuild draws a cell's merged mesh again only when its hash,
//! its assets or its take-over distance changed, and keeps the rest as they were uploaded.
//! Nothing of this reaches a simulation: the renderer keeps it in memory (the cooked cache of
//! a project is the place for a copy on disk, not built yet).

use crate::mesh::MeshData;
use gamecraft_math::StateHasher;
use gamecraft_world::{Entity, Model, Shape};
use glam::{Mat4, Vec3};

/// The side of a cell of the ground, metres.
pub const CELL_M: f64 = 128.0;
/// A merged mesh takes over no nearer than this, metres from the cell.
pub const NEAR_MIN_M: f64 = 300.0;
/// Beyond this a cell draws nothing, metres (the far plane of a view).
pub const FAR_M: f64 = 20_000.0;
/// Most vertices in the merged meshes of one cell: past it the cell keeps its own items only.
pub const MAX_CELL_VERTICES: usize = 4_000_000;
/// The most an axis of a merged entity is inset, as a share of its length: `1 - SHRINK` over
/// both sides, so 0.25 % a side. Where the real items and their merged copy draw together,
/// the copy is inside the real one.
pub const SHRINK: f32 = 0.995;
/// How far each side of a merged entity is inset towards its own origin, metres, on an axis
/// longer than 8 m (where [`SHRINK`]'s share would be more): a 200 m wall loses 2 cm at each
/// end, not the half a metre a side that the share would take, which showed as a gap where
/// its merged copy met a block beyond the band where both draw. Short axes keep the share: a
/// 0.3 m wall is inset 0.75 mm.
pub const INSET_M: f32 = 0.02;

/// A cell of the ground on the grid of [`CELL_M`]: `cx` covers `cx * CELL_M` to
/// `(cx + 1) * CELL_M` in x, the same in z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cell {
    /// The column.
    pub x: i64,
    /// The row.
    pub z: i64,
}

impl Cell {
    /// The cell of the point `(x, z)`.
    pub fn of(x: f64, z: f64) -> Cell {
        let grid = |v: f64| -> i64 {
            if !v.is_finite() {
                return 0;
            }
            (v / CELL_M).floor() as i64
        };
        Cell {
            x: grid(x),
            z: grid(z),
        }
    }

    /// The centre of the cell on the ground.
    #[cfg(test)]
    pub fn centre(&self) -> Vec3 {
        gamecraft_math::DVec3::new(
            (self.x as f64 + 0.5) * CELL_M,
            0.0,
            (self.z as f64 + 0.5) * CELL_M,
        )
        .as_vec3()
    }

    /// The length of the cell's diagonal on the ground.
    pub fn diagonal_m(&self) -> f64 {
        CELL_M * 2f64.sqrt()
    }

    /// Its name in the stats: `1_-2`.
    pub fn name(&self) -> String {
        format!("{}_{}", self.x, self.z)
    }
}

/// What a cell holds, as far as its merged mesh goes: a hash of everything that shapes it and
/// the distances it is drawn at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellPlan {
    /// The cell.
    pub cell: Cell,
    /// Over its static entities (id, pose, shape or model) and the assets revision.
    pub hash: u64,
    /// Where its merged mesh takes over: the largest draw distance of its own items, at least
    /// [`NEAR_MIN_M`].
    pub near_m: f64,
}

impl CellPlan {
    /// The cell's diagonal past its take-over: its own items draw until here, so that no
    /// cluster of the cell leaves a hole before the merged mesh shows.
    pub fn until_m(&self) -> f64 {
        self.near_m + self.cell.diagonal_m()
    }

    /// What decides whether a merged mesh may stay as it was uploaded.
    pub fn key(&self) -> (Cell, u64, u64) {
        (self.cell, self.hash, self.near_m.to_bits())
    }
}

/// The hash of a cell's content.
pub struct Content {
    hasher: StateHasher,
}

impl Content {
    /// The content of `cell`, with the assets at `assets_revision` (another revision of any
    /// model rebuilds every merged mesh that shows a model).
    pub fn new(cell: Cell, assets_revision: u64) -> Content {
        let mut hasher = StateHasher::new();
        hasher.write_i64(cell.x);
        hasher.write_i64(cell.z);
        hasher.write_u64(assets_revision);
        Content { hasher }
    }

    fn pose(&mut self, e: &Entity) {
        self.hasher.write_u64(e.id.0);
        for v in e.position {
            self.hasher.write_f64(v);
        }
        self.hasher.write_f64(e.rotation_y_deg);
        for axis in e.scale.get() {
            self.hasher.write_f64(axis);
        }
    }

    /// A shape entity.
    pub fn shape(&mut self, e: &Entity, s: &Shape) {
        self.pose(e);
        self.hasher.write_u8(0);
        self.hasher.write_u8(s.kind as u8);
        for v in s.size {
            self.hasher.write_f64(v);
        }
        for c in s.color {
            self.hasher.write_f32(c);
        }
    }

    /// A model entity (the asset's own geometry is covered by the assets revision).
    pub fn model(&mut self, e: &Entity, m: &Model) {
        self.pose(e);
        self.hasher.write_u8(1);
        self.hasher.write_f64(m.scale);
        self.hasher.write_str(&m.asset);
    }

    /// The hash.
    pub fn finish(self) -> u64 {
        self.hasher.finish()
    }
}

/// One mesh under construction: the triangles of many entities put where their transforms
/// send them, in world coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    /// Vertex positions, metres.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates (zeros where the source has none).
    pub uvs: Vec<[f32; 2]>,
    /// Triangles.
    pub indices: Vec<u32>,
}

/// By what the unit mesh is scaled along an axis `size` metres long in the world, so that it
/// is inset [`INSET_M`] a side, and never more than the share [`SHRINK`] leaves (so nothing
/// changes for an entity up to 8 m long, and a tiny one keeps its share): [`SHRINK`] itself
/// for a size that is not finite or not positive.
fn inset_factor(size: f32) -> f32 {
    let longest_shared = 2.0 * INSET_M / (1.0 - SHRINK);
    if size.is_finite() && size > longest_shared {
        1.0 - 2.0 * INSET_M / size
    } else {
        SHRINK
    }
}

impl Merged {
    /// Adds the triangles of `mesh` (its vertex arrays) where `matrix` sends them, every
    /// vertex pulled towards the mesh's own origin: by [`SHRINK`] on an axis up to 8 m long, by
    /// [`INSET_M`] a side on a longer one ([`inset_factor`]). `false` when the mesh's vertices
    /// would pass [`MAX_CELL_VERTICES`] or the index range: nothing is added then.
    pub fn push(
        &mut self,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: Option<&[[f32; 2]]>,
        indices: &[u32],
        matrix: Mat4,
    ) -> bool {
        let count = positions.len();
        if self.positions.len().saturating_add(count) > MAX_CELL_VERTICES {
            return false;
        }
        let Some(base) = u32::try_from(self.positions.len()).ok() else {
            return false;
        };
        let normal_xf = glam::Mat3::from_mat4(matrix).inverse().transpose()
            * (1.0 / matrix.x_axis.truncate().length().max(1e-9));
        let pull = |p: Vec3| {
            Vec3::new(
                p.x * inset_factor(matrix.x_axis.truncate().length()),
                p.y * inset_factor(matrix.y_axis.truncate().length()),
                p.z * inset_factor(matrix.z_axis.truncate().length()),
            )
        };
        for (v, at) in positions.iter().zip(normals.iter()) {
            self.positions
                .push(matrix.transform_point3(pull(Vec3::from(*v))).to_array());
            self.normals.push(
                (normal_xf * Vec3::from(*at))
                    .normalize_or(Vec3::Y)
                    .to_array(),
            );
        }
        let no_uvs = vec![[0.0; 2]; count];
        let uvs = uvs.unwrap_or(&no_uvs);
        self.uvs.extend(uvs.iter().take(count).copied());
        self.indices.extend(indices.iter().map(|&i| i + base));
        true
    }

    /// [`Merged::push`] of a plain mesh.
    pub fn push_mesh(&mut self, mesh: &MeshData, matrix: Mat4) -> bool {
        self.push(
            &mesh.positions,
            &mesh.normals,
            mesh.uvs.as_deref(),
            &mesh.indices,
            matrix,
        )
    }

    /// Its triangles.
    pub fn triangles(&self) -> u64 {
        (self.indices.len() / 3) as u64
    }

    /// Its box, `None` while empty.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = Vec3::from(*self.positions.first()?);
        let mut lo = first;
        let mut hi = first;
        for p in &self.positions {
            let p = Vec3::from(*p);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        Some((lo, hi))
    }

    /// As a mesh for the renderer.
    pub fn mesh(&self) -> MeshData {
        MeshData {
            positions: self.positions.clone(),
            normals: self.normals.clone(),
            indices: self.indices.clone(),
            uvs: Some(self.uvs.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_holds_its_square_and_names_itself() {
        let c = Cell::of(130.0, -1.0);
        assert_eq!((c.x, c.z), (1, -1));
        assert_eq!(c.name(), "1_-1");
        assert_eq!(c.centre(), Vec3::new(192.0, 0.0, -64.0));
        assert!((c.diagonal_m() - 181.019).abs() < 1e-3);
        assert_eq!(Cell::of(f64::NAN, 0.0), Cell::of(0.0, 0.0));
        let far = Cell::of(-2000.0, 99_000.0);
        assert!(
            far.centre().x < -1900.0 && far.centre().z > 98_900.0,
            "{:?}",
            far.centre()
        );
    }

    #[test]
    fn a_plan_says_how_far_the_own_items_draw() {
        let plan = CellPlan {
            cell: Cell::of(0.0, 0.0),
            hash: 7,
            near_m: 400.0,
        };
        assert!(plan.until_m() > 580.0 && plan.until_m() < 582.0);
        assert_eq!(plan.key(), (plan.cell, 7, 400.0f64.to_bits()));
    }

    #[test]
    fn a_long_merged_entity_is_inset_by_two_centimetres_and_no_more() {
        // A wall 200 m long, 3 m high and 0.3 m thick: its copy loses 2 cm at each end (not
        // half a metre, which showed as a gap where it met a block), and keeps the share of
        // 0.25 % on the axes that are shorter than 8 m.
        let mut merged = Merged::default();
        let at = Mat4::from_scale_rotation_translation(
            Vec3::new(200.0, 3.0, 0.3),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
        );
        assert!(merged.push_mesh(&crate::mesh::unit_box(), at));
        let (lo, hi) = merged.bounds().unwrap();
        let near = |got: f32, want: f64| (f64::from(got) - want).abs() < 1e-3;
        assert!(near(lo.x, -99.98) && near(hi.x, 99.98), "{lo:?} {hi:?}");
        assert!(near(hi.y, 3.0 * 0.995), "{lo:?} {hi:?}");
        assert!(
            near(lo.z, -0.15 * 0.995) && near(hi.z, 0.15 * 0.995),
            "{lo:?} {hi:?}"
        );
        // The turn and the move of the entity apply as before.
        let mut turned = Merged::default();
        let at = Mat4::from_scale_rotation_translation(
            Vec3::new(200.0, 3.0, 0.3),
            glam::Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
            Vec3::new(1000.0, 5.0, -7.0),
        );
        assert!(turned.push_mesh(&crate::mesh::unit_box(), at));
        let (lo, hi) = turned.bounds().unwrap();
        assert!(
            near(hi.z, -7.0 + 99.98) && near(lo.z, -7.0 - 99.98),
            "{lo:?} {hi:?}"
        );
        assert!(
            near(lo.x, 1000.0 - 0.15 * 0.995) && near(hi.x, 1000.0 + 0.15 * 0.995),
            "{lo:?} {hi:?}"
        );
        assert!(near(lo.y, 5.0), "{lo:?} {hi:?}");
    }

    #[test]
    fn a_merged_mesh_puts_triangles_where_the_transform_sends_them() {
        let mut merged = Merged::default();
        let unit = crate::mesh::unit_box();
        let at = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 3.0, 2.0),
            glam::Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
        );
        assert!(merged.push_mesh(&unit, at));
        assert_eq!(merged.triangles(), 12);
        assert_eq!(merged.positions.len(), unit.positions.len());
        let (lo, hi) = merged.bounds().unwrap();
        // The unit box stands on y = 0 and is half a metre across: scaled 2 x 3 x 2 about
        // the origin at (10, 0, 0), then shrunk half a percent about that origin.
        assert!(
            (f64::from(lo.x) - (10.0 - 0.995)).abs() < 0.01,
            "{lo:?} {hi:?}"
        );
        assert!(
            (f64::from(hi.x) - (10.0 + 0.995)).abs() < 0.01,
            "{lo:?} {hi:?}"
        );
        assert!(
            lo.y.abs() < 1e-4 && (f64::from(hi.y) - 3.0 * 0.995).abs() < 0.01,
            "{lo:?} {hi:?}"
        );
        // Every index points at a vertex, the normals are unit, the texture coordinates are
        // zeros where the mesh had none.
        assert!(
            merged
                .indices
                .iter()
                .all(|&i| (i as usize) < merged.positions.len())
        );
        assert!(merged.uvs.iter().all(|uv| *uv == [0.0; 2]));
        assert!(
            merged
                .normals
                .iter()
                .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-4)
        );
        // A second mesh joins the first.
        assert!(merged.push_mesh(&unit, Mat4::IDENTITY));
        assert_eq!(merged.triangles(), 24);
        assert_eq!(merged.mesh().indices.len(), 72);
    }

    #[test]
    fn a_merged_mesh_stops_at_the_cap() {
        let mut merged = Merged::default();
        let big = MeshData {
            positions: vec![[0.0; 3]; MAX_CELL_VERTICES],
            normals: vec![[0.0, 1.0, 0.0]; MAX_CELL_VERTICES],
            indices: vec![0; 3],
            uvs: None,
        };
        assert!(merged.push_mesh(&big, Mat4::IDENTITY));
        let one = crate::mesh::unit_box();
        assert!(
            !merged.push_mesh(&one, Mat4::IDENTITY),
            "past the cap nothing is added"
        );
        assert_eq!(merged.positions.len(), MAX_CELL_VERTICES);
    }

    #[test]
    fn the_content_hash_follows_every_change() {
        let mut w = gamecraft_world::World::new("c", 1);
        let shape = gamecraft_world::Shape {
            kind: gamecraft_world::ShapeKind::Box,
            size: [1.0, 2.0, 3.0],
            color: [0.5, 0.5, 0.5],
            solid: false,
        };
        let id = w
            .add_entity(gamecraft_world::EntityDraft {
                name: "Box".into(),
                position: [10.0, 0.0, 20.0],
                components: vec![gamecraft_world::Component::Shape(shape.clone())],
                ..gamecraft_world::EntityDraft::default()
            })
            .unwrap();
        // The content hash of the box's cell, at a revision of the assets.
        fn hash(w: &gamecraft_world::World, id: gamecraft_world::EntityId, revision: u64) -> u64 {
            let e = w.entity(id).unwrap();
            let mut c = Content::new(Cell::of(e.position[0], e.position[2]), revision);
            c.shape(e, e.shape().unwrap());
            c.finish()
        }
        assert_eq!(hash(&w, id, 1), hash(&w, id, 1));
        assert_ne!(
            hash(&w, id, 1),
            hash(&w, id, 2),
            "another revision of the assets"
        );
        let before = hash(&w, id, 1);
        w.entity_mut(id).unwrap().position[0] += 1.0;
        assert_ne!(hash(&w, id, 1), before, "a moved entity");
        let before = hash(&w, id, 1);
        w.entity_mut(id)
            .unwrap()
            .set_component(gamecraft_world::Component::Shape(gamecraft_world::Shape {
                size: [1.0, 2.0, 4.0],
                ..shape
            }));
        assert_ne!(hash(&w, id, 1), before, "another size");
        let model = gamecraft_world::Model::of("models/crate.glb");
        let mut c = Content::new(Cell::of(0.0, 0.0), 1);
        c.model(w.entity(id).unwrap(), &model);
        let h = c.finish();
        let mut c = Content::new(Cell::of(0.0, 0.0), 1);
        c.model(
            w.entity(id).unwrap(),
            &gamecraft_world::Model {
                scale: 2.0,
                ..model.clone()
            },
        );
        assert_ne!(c.finish(), h, "another model scale");
    }
}
