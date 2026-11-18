//! The GPU-free half of the adapter: triangle meshes in plain arrays.
//!
//! - [`terrain_chunks`]: a height field as chunks of at most [`TERRAIN_CHUNK_CELLS`] cells per
//!   side, each cell split along the diagonal from sample (col, row) to (col + 1, row + 1), the
//!   same two triangles `HeightField::height_at` interpolates over, with layer weights by slope.
//! - [`terrain_apron`]: a ring of extra mesh round the terrain that eases its edge down (or up)
//!   to the endless ground plane. It is scenery only, **not walkable ground**: the height field
//!   and the simulation know nothing of it, and nothing may stand on it or be placed by it.
//! - [`unit_box`], [`unit_sphere`], [`unit_cylinder`]: the shapes, one metre across and tall,
//!   standing on the origin (base at y = 0), to be scaled per entity.
//! - [`character`]: a character's body and the visor that shows where it faces.
//!
//! Triangles are counter-clockwise seen from outside (from above, for the terrain); normals
//! are unit length.

use gamecraft_world::{Character, HeightField};

/// Most cells along one side of a terrain chunk.
pub const TERRAIN_CHUNK_CELLS: usize = 64;
/// Terrain layer of gentle ground (grass).
pub const LAYER_GRASS: usize = 0;
/// Terrain layer between gentle and steep ground (bare earth).
pub const LAYER_EARTH: usize = 1;
/// Terrain layer of steep ground (rock).
pub const LAYER_ROCK: usize = 2;
/// How many terrain layers the weights use.
pub const TERRAIN_LAYERS: usize = 3;
/// Up to this slope (degrees) the ground is all grass; grass is gone at `GRASS_END_DEG`.
pub const GRASS_FULL_DEG: f64 = 20.0;
/// The slope (degrees) where grass has given way.
pub const GRASS_END_DEG: f64 = 35.0;
/// The slope (degrees) where rock starts to show.
pub const ROCK_START_DEG: f64 = 30.0;
/// From this slope (degrees) the ground is all rock.
pub const ROCK_FULL_DEG: f64 = 45.0;

/// Segments round the sphere and the cylinder.
const ROUND_SEGMENTS: usize = 32;
/// Rings from pole to pole of the sphere.
const SPHERE_RINGS: usize = 16;
/// Head radius as a share of a character's height (capped by its radius).
const HEAD_SHARE: f64 = 0.12;
/// The visor's width, height and depth as shares of the head radius.
const VISOR_SIZE: [f64; 3] = [1.4, 0.5, 0.6];

/// A triangle mesh in plain arrays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    /// Vertex positions, metres.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Three indices per triangle, counter-clockwise seen from outside.
    pub indices: Vec<u32>,
    /// Texture coordinates, one per position, when the mesh has any.
    pub uvs: Option<Vec<[f32; 2]>>,
}

impl MeshData {
    /// How many triangles the mesh holds.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The smallest and largest coordinates of the positions; `None` for an empty mesh.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.positions.iter();
        let first = *it.next()?;
        Some(it.fold(
            (first, first),
            |([lx, ly, lz], [hx, hy, hz]), &[x, y, z]| {
                (
                    [lx.min(x), ly.min(y), lz.min(z)],
                    [hx.max(x), hy.max(y), hz.max(z)],
                )
            },
        ))
    }

    /// The triangles as index triples.
    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        // A trailing incomplete triangle is ignored.
        self.indices.as_chunks::<3>().0.iter().copied()
    }

    /// Adds a vertex and returns its index.
    fn vertex(&mut self, position: [f64; 3], normal: [f64; 3]) -> u32 {
        let index = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        self.positions.push(position.map(|v| v as f32));
        self.normals.push(normal.map(|v| v as f32));
        index
    }

    /// Adds a triangle.
    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend([a, b, c]);
    }

    /// Appends `other`, scaled per axis and then moved by `offset`. Normals follow the scale
    /// (divided by it and normalised), so a stretched shape still shades right.
    fn append_transformed(&mut self, other: &MeshData, scale: [f64; 3], offset: [f64; 3]) {
        let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        for (p, n) in other.positions.iter().zip(&other.normals) {
            let position = [0, 1, 2].map(|k| {
                f64::from(p.get(k).copied().unwrap_or(0.0)) * scale.get(k).copied().unwrap_or(1.0)
                    + offset.get(k).copied().unwrap_or(0.0)
            });
            let bent = [0, 1, 2].map(|k| {
                f64::from(n.get(k).copied().unwrap_or(0.0)) / scale.get(k).copied().unwrap_or(1.0)
            });
            self.vertex(position, normalised(bent));
        }
        self.indices
            .extend(other.indices.iter().map(|i| i.saturating_add(base)));
    }
}

/// A unit vector along `v`, or +Y for a zero or non-finite one.
fn normalised(v: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = v;
    let len = (x * x + y * y + z * z).sqrt();
    if len.is_finite() && len > 0.0 {
        v.map(|c| c / len)
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// One terrain chunk: its mesh and the layer weights of its vertices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainChunkMesh {
    /// The height field column of the chunk's first vertex.
    pub first_col: usize,
    /// The height field row of the chunk's first vertex.
    pub first_row: usize,
    /// Positions in world coordinates, normals from the height field.
    pub mesh: MeshData,
    /// Per vertex the weights of up to eight layers, summing to 255 ([`layer_weights`]).
    pub weights: Vec<[u8; 8]>,
}

/// The height field as chunks of at most [`TERRAIN_CHUNK_CELLS`] x [`TERRAIN_CHUNK_CELLS`]
/// cells, in row-major chunk order.
///
/// Every sample is a vertex; neighbouring chunks share the samples on their border (the same
/// positions, normals and weights). Each cell is split along the diagonal from sample
/// (col, row) to (col + 1, row + 1), so the mesh is exactly the surface `height_at` answers
/// for. A field with fewer than two samples per side gives no chunks.
pub fn terrain_chunks(heights: &HeightField) -> Vec<TerrainChunkMesh> {
    let (cols, rows) = (heights.cols(), heights.rows());
    if cols < 2 || rows < 2 {
        return Vec::new();
    }
    let (cells_x, cells_z) = (cols - 1, rows - 1);
    let mut chunks = Vec::new();
    for first_row in (0..cells_z).step_by(TERRAIN_CHUNK_CELLS) {
        for first_col in (0..cells_x).step_by(TERRAIN_CHUNK_CELLS) {
            let nx = TERRAIN_CHUNK_CELLS.min(cells_x - first_col);
            let nz = TERRAIN_CHUNK_CELLS.min(cells_z - first_row);
            chunks.push(terrain_chunk(heights, first_col, first_row, nx, nz));
        }
    }
    chunks
}

/// The chunk of `nx` x `nz` cells starting at sample (`first_col`, `first_row`).
fn terrain_chunk(
    heights: &HeightField,
    first_col: usize,
    first_row: usize,
    nx: usize,
    nz: usize,
) -> TerrainChunkMesh {
    let [x0, z0] = heights.min();
    let cell = heights.cell_m();
    let mut mesh = MeshData::default();
    let mut weights = Vec::with_capacity((nx + 1) * (nz + 1));
    for r in 0..=nz {
        for c in 0..=nx {
            let (col, row) = (first_col + c, first_row + r);
            let h = heights.sample(col, row).map_or(0.0, f64::from);
            // The same formula as the height field's node position, so the bits agree.
            let x = x0 + col as f64 * cell;
            let z = z0 + row as f64 * cell;
            let normal = terrain_normal(heights, col, row);
            let [_, normal_y, _] = normal;
            mesh.vertex([x, h, z], normal);
            weights.push(layer_weights(normal_y));
        }
    }
    let stride = u32::try_from(nx + 1).unwrap_or(u32::MAX);
    for r in 0..nz {
        for c in 0..nx {
            let i00 = u32::try_from(r * (nx + 1) + c).unwrap_or(u32::MAX);
            let i10 = i00.saturating_add(1);
            let i01 = i00.saturating_add(stride);
            let i11 = i01.saturating_add(1);
            // (col, row), (col + 1, row + 1), (col + 1, row) and (col, row), (col, row + 1),
            // (col + 1, row + 1): the height field's two triangles, wound to face up.
            mesh.triangle(i00, i11, i10);
            mesh.triangle(i00, i01, i11);
        }
    }
    TerrainChunkMesh {
        first_col,
        first_row,
        mesh,
        weights,
    }
}

/// Rings of vertices of the apron beyond the terrain's own border ring.
pub const APRON_RINGS: usize = 6;
/// The narrowest apron, metres.
pub const APRON_MIN_WIDTH_M: f64 = 16.0;
/// The apron's width as a share of the terrain's size.
pub const APRON_WIDTH_SHARE: f64 = 0.15;

/// How wide the apron round `heights` is, metres: `max(16 m, 15 % of the size)`.
pub fn apron_width_m(heights: &HeightField) -> f64 {
    let [x0, z0] = heights.min();
    let [x1, z1] = heights.max();
    let size = (x1 - x0).max(z1 - z0);
    APRON_MIN_WIDTH_M.max(APRON_WIDTH_SHARE * size)
}

/// `t` eased: 0 at 0, 1 at 1, flat at both ends.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A ring of mesh round the height field that blends its border down (or up) to the endless
/// ground plane at `plane_height_m`, so the terrain does not end in a wall.
///
/// Ring 0 is the terrain's border samples, bit for bit (same positions, normals and weights as
/// [`terrain_chunks`]); ring [`APRON_RINGS`] lies [`apron_width_m`] outside the square at the
/// plane's height with the grass layer. In between the height follows a smoothstep from the
/// border height to the plane height, and the layer weights fade to grass the same way.
/// Normals come from the apron's own surface (the terrain's at ring 0). The mesh is one chunk
/// at (0, 0); an empty chunk when the field has fewer than two samples per side.
///
/// The apron is scenery, **not walkable ground**: the height field and the simulation are
/// unchanged and know nothing of it.
pub fn terrain_apron(heights: &HeightField, plane_height_m: f64) -> TerrainChunkMesh {
    let (cols, rows) = (heights.cols(), heights.rows());
    if cols < 2 || rows < 2 {
        return TerrainChunkMesh::default();
    }
    let (lc, lr) = (cols - 1, rows - 1);
    // The border samples clockwise seen from above, starting at the north-west corner, each
    // with its outward direction (diagonal at the corners).
    let mut ring: Vec<(usize, usize, [f64; 2])> = Vec::new();
    let out = |c: usize, r: usize| {
        let ox = if c == 0 {
            -1.0
        } else if c == lc {
            1.0
        } else {
            0.0
        };
        let oz = if r == 0 {
            -1.0
        } else if r == lr {
            1.0
        } else {
            0.0
        };
        [ox, oz]
    };
    for c in 0..lc {
        ring.push((c, 0, out(c, 0)));
    }
    for r in 0..lr {
        ring.push((lc, r, out(lc, r)));
    }
    for c in (1..=lc).rev() {
        ring.push((c, lr, out(c, lr)));
    }
    for r in (1..=lr).rev() {
        ring.push((0, r, out(0, r)));
    }
    let per = ring.len();
    let width = apron_width_m(heights);
    let [x0, z0] = heights.min();
    let cell = heights.cell_m();
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(per * (APRON_RINGS + 1));
    let mut border_weights: Vec<[u8; 8]> = Vec::with_capacity(per);
    for k in 0..=APRON_RINGS {
        let t = k as f64 / APRON_RINGS as f64;
        let s = smoothstep(t);
        for &(c, r, [ox, oz]) in &ring {
            let h = heights.sample(c, r).map_or(0.0, f64::from);
            // The same formula as the terrain's vertices, so ring 0 agrees bit for bit.
            let x = x0 + c as f64 * cell;
            let z = z0 + r as f64 * cell;
            if k == 0 {
                let [_, ny, _] = terrain_normal(heights, c, r);
                border_weights.push(layer_weights(ny));
                positions.push([x, h, z]);
            } else {
                positions.push([
                    x + ox * width * t,
                    h + (plane_height_m - h) * s,
                    z + oz * width * t,
                ]);
            }
        }
    }
    // Triangles: quads between ring k and k + 1, wound to face up.
    let idx = |k: usize, i: usize| u32::try_from(k * per + i % per).unwrap_or(u32::MAX);
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for k in 0..APRON_RINGS {
        for i in 0..per {
            let (a, b) = (idx(k, i), idx(k, i + 1));
            let (c, d) = (idx(k + 1, i), idx(k + 1, i + 1));
            tris.push([a, b, d]);
            tris.push([a, d, c]);
        }
    }
    // Normals: area-weighted from the apron's triangles; the terrain's own on ring 0.
    let mut sums = vec![[0.0f64; 3]; positions.len()];
    for tri in &tris {
        let p = tri.map(|i| positions.get(i as usize).copied().unwrap_or([0.0; 3]));
        let [a, b, c] = p;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for i in tri {
            if let Some(sum) = sums.get_mut(*i as usize) {
                for (s, n) in sum.iter_mut().zip(n) {
                    *s += n;
                }
            }
        }
    }
    let mut mesh = MeshData::default();
    let mut weights = Vec::with_capacity(positions.len());
    for (n, (position, sum)) in positions.iter().zip(&sums).enumerate() {
        let (k, i) = (n / per, n % per);
        let normal = match (k, ring.get(i)) {
            (0, Some(&(c, r, _))) => terrain_normal(heights, c, r),
            _ => normalised(*sum),
        };
        let w = if k == 0 {
            border_weights.get(i).copied().unwrap_or([0; 8])
        } else {
            let [_, ny, _] = normal;
            fade_to_grass(layer_weights(ny), smoothstep(k as f64 / APRON_RINGS as f64))
        };
        mesh.vertex(*position, normal);
        weights.push(w);
    }
    for [a, b, c] in tris {
        mesh.triangle(a, b, c);
    }
    TerrainChunkMesh {
        first_col: 0,
        first_row: 0,
        mesh,
        weights,
    }
}

/// `weights` moved `share` (0 to 1) of the way to all grass, still summing to 255.
fn fade_to_grass(weights: [u8; 8], share: f64) -> [u8; 8] {
    let keep = 1.0 - share.clamp(0.0, 1.0);
    let scaled = |layer: usize| {
        let w = weights.get(layer).copied().map_or(0.0, f64::from);
        (w * keep).round().clamp(0.0, 255.0) as u8
    };
    let (e, r) = (scaled(LAYER_EARTH), scaled(LAYER_ROCK));
    let mut out = [0u8; 8];
    for (layer, w) in [
        (LAYER_GRASS, 255u8.saturating_sub(e).saturating_sub(r)),
        (LAYER_EARTH, e),
        (LAYER_ROCK, r),
    ] {
        if let Some(slot) = out.get_mut(layer) {
            *slot = w;
        }
    }
    out
}

/// The unit upward normal of the height field at sample (`col`, `row`), from central
/// differences of the neighbouring samples (one-sided at the border).
pub fn terrain_normal(heights: &HeightField, col: usize, row: usize) -> [f64; 3] {
    let h = |c: usize, r: usize| heights.sample(c, r).map_or(0.0, f64::from);
    let cell = heights.cell_m();
    let (c0, c1) = (
        col.saturating_sub(1),
        (col + 1).min(heights.cols().saturating_sub(1)),
    );
    let (r0, r1) = (
        row.saturating_sub(1),
        (row + 1).min(heights.rows().saturating_sub(1)),
    );
    let dx = if c1 > c0 {
        (h(c1, row) - h(c0, row)) / ((c1 - c0) as f64 * cell)
    } else {
        0.0
    };
    let dz = if r1 > r0 {
        (h(col, r1) - h(col, r0)) / ((r1 - r0) as f64 * cell)
    } else {
        0.0
    };
    normalised([-dx, 1.0, -dz])
}

/// The layer weights of ground whose normal has `normal_y` as its upward part: grass up to
/// 20 degrees fading out by 35, rock from 30 degrees full at 45, bare earth in between. The
/// weights sum to 255 exactly; layers past [`TERRAIN_LAYERS`] are 0.
pub fn layer_weights(normal_y: f64) -> [u8; 8] {
    let slope_deg = normal_y.clamp(-1.0, 1.0).acos().to_degrees();
    let grass =
        1.0 - ((slope_deg - GRASS_FULL_DEG) / (GRASS_END_DEG - GRASS_FULL_DEG)).clamp(0.0, 1.0);
    let rock = ((slope_deg - ROCK_START_DEG) / (ROCK_FULL_DEG - ROCK_START_DEG)).clamp(0.0, 1.0);
    let to_byte = |w: f64| (w * 255.0).round().clamp(0.0, 255.0) as u8;
    let (g, r) = (to_byte(grass), to_byte(rock));
    let e = 255u8.saturating_sub(g).saturating_sub(r);
    // Rounding cannot overshoot (grass and rock never both exceed a third), but keep the sum.
    let r = 255u8.saturating_sub(g).saturating_sub(e);
    let mut weights = [0u8; 8];
    for (layer, w) in [(LAYER_GRASS, g), (LAYER_EARTH, e), (LAYER_ROCK, r)] {
        if let Some(slot) = weights.get_mut(layer) {
            *slot = w;
        }
    }
    weights
}

/// A box 1 m on each side standing on the origin: x and z from -0.5 to 0.5, y from 0 to 1.
/// Flat normals: four vertices per face.
pub fn unit_box() -> MeshData {
    let mut mesh = MeshData::default();
    // Each face: its normal and two in-face axes u, v with u x v = normal.
    let faces: [([f64; 3], [f64; 3], [f64; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    for (n, u, v) in faces {
        let corner = |su: f64, sv: f64| {
            [0, 1, 2].map(|k| {
                let at = |a: [f64; 3]| a.get(k).copied().unwrap_or(0.0);
                0.5 * at(n) + 0.5 * su * at(u) + 0.5 * sv * at(v) + if k == 1 { 0.5 } else { 0.0 }
            })
        };
        let a = mesh.vertex(corner(-1.0, -1.0), n);
        let b = mesh.vertex(corner(1.0, -1.0), n);
        let c = mesh.vertex(corner(1.0, 1.0), n);
        let d = mesh.vertex(corner(-1.0, 1.0), n);
        mesh.triangle(a, b, c);
        mesh.triangle(a, c, d);
    }
    mesh
}

/// A sphere 1 m across standing on the origin (centre at y = 0.5): smooth normals, a single
/// vertex at each pole.
pub fn unit_sphere() -> MeshData {
    let mut mesh = MeshData::default();
    let r = 0.5;
    let bottom = mesh.vertex([0.0, 0.0, 0.0], [0.0, -1.0, 0.0]);
    let mut rings = Vec::new();
    for ring in 1..SPHERE_RINGS {
        // Polar angle from the bottom pole.
        let theta = std::f64::consts::PI * ring as f64 / SPHERE_RINGS as f64;
        let (y, horizontal) = (-theta.cos(), theta.sin());
        let first = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
        for s in 0..ROUND_SEGMENTS {
            let phi = std::f64::consts::TAU * s as f64 / ROUND_SEGMENTS as f64;
            let (nx, nz) = (horizontal * phi.cos(), horizontal * phi.sin());
            mesh.vertex([r * nx, r + r * y, r * nz], [nx, y, nz]);
        }
        rings.push(first);
    }
    let top = mesh.vertex([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]);
    let segments = u32::try_from(ROUND_SEGMENTS).unwrap_or(u32::MAX);
    let next = |s: u32| (s + 1) % segments;
    if let (Some(&first), Some(&last)) = (rings.first(), rings.last()) {
        for s in 0..segments {
            mesh.triangle(bottom, first + s, first + next(s));
            mesh.triangle(top, last + next(s), last + s);
        }
    }
    for pair in rings.windows(2) {
        if let [lower, upper] = *pair {
            for s in 0..segments {
                let (a, b) = (lower + s, lower + next(s));
                let (c, d) = (upper + next(s), upper + s);
                mesh.triangle(a, d, c);
                mesh.triangle(a, c, b);
            }
        }
    }
    mesh
}

/// An upright cylinder 1 m across and 1 m tall standing on the origin: a smooth side and flat
/// caps.
pub fn unit_cylinder() -> MeshData {
    let mut mesh = MeshData::default();
    let r = 0.5;
    let segments = u32::try_from(ROUND_SEGMENTS).unwrap_or(u32::MAX);
    let next = |s: u32| (s + 1) % segments;
    let around = |s: usize| {
        let phi = std::f64::consts::TAU * s as f64 / ROUND_SEGMENTS as f64;
        (phi.cos(), phi.sin())
    };
    // Side: a bottom and a top ring with outward normals.
    let side_bottom = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
    for s in 0..ROUND_SEGMENTS {
        let (c, sn) = around(s);
        mesh.vertex([r * c, 0.0, r * sn], [c, 0.0, sn]);
    }
    let side_top = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
    for s in 0..ROUND_SEGMENTS {
        let (c, sn) = around(s);
        mesh.vertex([r * c, 1.0, r * sn], [c, 0.0, sn]);
    }
    for s in 0..segments {
        let (a, b) = (side_bottom + s, side_bottom + next(s));
        let (c, d) = (side_top + next(s), side_top + s);
        mesh.triangle(a, d, c);
        mesh.triangle(a, c, b);
    }
    // Caps: a centre and a ring each, with flat normals.
    for (y, ny) in [(0.0, -1.0), (1.0, 1.0)] {
        let centre = mesh.vertex([0.0, y, 0.0], [0.0, ny, 0.0]);
        let ring = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
        for s in 0..ROUND_SEGMENTS {
            let (c, sn) = around(s);
            mesh.vertex([r * c, y, r * sn], [0.0, ny, 0.0]);
        }
        for s in 0..segments {
            if ny > 0.0 {
                mesh.triangle(centre, ring + next(s), ring + s);
            } else {
                mesh.triangle(centre, ring + s, ring + next(s));
            }
        }
    }
    mesh
}

/// A character's meshes in its own frame: base at the origin, facing -Z (yaw 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterMeshes {
    /// A cylinder of the character's radius topped by a head sphere, `height_m` tall.
    pub body: MeshData,
    /// A small box on the front (-Z side) at eye height, so the facing is visible.
    pub visor: MeshData,
}

/// The body and visor of a character of the component's height, radius and eye height.
///
/// The head is a sphere of radius `min(radius_m, 0.12 * height_m)` whose top is at
/// `height_m`; the body is a cylinder of `radius_m` from the ground to the head's centre.
/// The visor sits on the front of the head (or of the body, below the head) at
/// `eye_height_m`.
pub fn character(c: &Character) -> CharacterMeshes {
    let height = c.height_m;
    let head_r = c.radius_m.min(HEAD_SHARE * height);
    let head_centre = height - head_r;
    let mut body = MeshData::default();
    body.append_transformed(
        &unit_cylinder(),
        [2.0 * c.radius_m, head_centre, 2.0 * c.radius_m],
        [0.0; 3],
    );
    body.append_transformed(
        &unit_sphere(),
        [2.0 * head_r; 3],
        [0.0, head_centre - head_r, 0.0],
    );
    let dy = c.eye_height_m - head_centre;
    let front = if dy.abs() < head_r {
        (head_r * head_r - dy * dy).sqrt()
    } else {
        c.radius_m
    };
    let size = VISOR_SIZE.map(|s| s * head_r);
    let [_, visor_h, _] = size;
    let mut visor = MeshData::default();
    // Centred on the surface in front: half of it stands out.
    visor.append_transformed(
        &unit_box(),
        size,
        [0.0, c.eye_height_m - visor_h / 2.0, -front],
    );
    CharacterMeshes { body, visor }
}
