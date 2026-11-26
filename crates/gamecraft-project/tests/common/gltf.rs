//! Small glTF models built for a test: meshes of unit cubes side by side, nodes with names,
//! extensions and extras, packed into a GLB.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gamecraft_project::textures::pack_glb;
use serde_json::{Value, json};

/// A mesh of unit cubes side by side along x (12 triangles each).
#[derive(Clone, Copy)]
pub struct Mesh(pub usize);

/// A node of the scene.
#[derive(Clone)]
pub struct Node {
    pub name: String,
    pub mesh: Option<usize>,
    pub translation: [f64; 3],
    /// A turn about an axis: `[x, y, z, w]`.
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
    /// A node given as a matrix (column-major) instead of a translation, a rotation and a
    /// scale.
    pub matrix: Option<[f64; 16]>,
    pub children: Vec<usize>,
    pub extensions: Value,
    pub extras: Value,
}

impl Node {
    pub fn new(name: &str, mesh: Option<usize>) -> Node {
        Node {
            name: name.to_string(),
            mesh,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            matrix: None,
            children: Vec::new(),
            extensions: json!({}),
            extras: Value::Null,
        }
    }

    /// Turned `yaw_deg` about +Y.
    pub fn turned(mut self, yaw_deg: f64) -> Node {
        let half = yaw_deg * std::f64::consts::PI / 360.0;
        let (sin, cos) = gamecraft_math::Portable::psin_cos(half);
        self.rotation = [0.0, sin, 0.0, cos];
        self
    }

    /// Scaled by `s` on every axis.
    pub fn scaled(mut self, s: f64) -> Node {
        self.scale = [s; 3];
        self
    }

    /// Scaled by `[x, y, z]`.
    pub fn scaled3(mut self, s: [f64; 3]) -> Node {
        self.scale = s;
        self
    }

    pub fn at(mut self, x: f64, y: f64, z: f64) -> Node {
        self.translation = [x, y, z];
        self
    }

    pub fn children(mut self, children: &[usize]) -> Node {
        self.children = children.to_vec();
        self
    }

    pub fn msft_lod(mut self, ids: &[usize], coverage: &[f64]) -> Node {
        self.extensions = json!({"MSFT_lod": {"ids": ids}});
        self.extras = json!({"MSFT_screencoverage": coverage});
        self
    }
}

/// A model: its meshes, its nodes and the nodes its scene shows.
pub struct Model {
    pub meshes: Vec<Mesh>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub asset_extras: Value,
    /// The glTF mode of each mesh (4: triangles, the default; 5: a strip; 6: a fan); shorter
    /// than the meshes: the rest are triangles.
    pub modes: Vec<u16>,
}

impl Model {
    pub fn new(meshes: &[Mesh], nodes: Vec<Node>, roots: &[usize]) -> Model {
        Model {
            meshes: meshes.to_vec(),
            nodes,
            roots: roots.to_vec(),
            asset_extras: Value::Null,
            modes: Vec::new(),
        }
    }

    /// `Mesh(3), Mesh(2), Mesh(1)` as the levels of a box thing.
    pub fn level_meshes() -> Vec<Mesh> {
        vec![Mesh(3), Mesh(2), Mesh(1)]
    }

    /// A thing of three levels named Unity style, all in the scene.
    pub fn named() -> Model {
        Model::new(
            &Self::level_meshes(),
            vec![
                Node::new("Thing_LOD0", Some(0)),
                Node::new("Thing_LOD1", Some(1)),
                Node::new("Thing_LOD2", Some(2)),
            ],
            &[0, 1, 2],
        )
    }

    /// The same by `MSFT_lod`: the finest node carries the extension, the others stand
    /// outside the scene.
    pub fn msft() -> Model {
        Model::new(
            &Self::level_meshes(),
            vec![
                Node::new("High", Some(0)).msft_lod(&[1, 2], &[1.0, 0.25, 0.02]),
                Node::new("Medium", Some(1)),
                Node::new("Low", Some(2)),
            ],
            &[0],
        )
    }

    /// The GLB file of the model.
    pub fn glb(&self) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        let mut views: Vec<Value> = Vec::new();
        let mut accessors: Vec<Value> = Vec::new();
        let mut meshes: Vec<Value> = Vec::new();
        for (m, Mesh(boxes)) in self.meshes.iter().enumerate() {
            let offset = bin.len();
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut count = 0usize;
            for b in 0..*boxes {
                for p in cube() {
                    let p = [p[0] + b as f32, p[1], p[2]];
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                        bin.extend_from_slice(&p[k].to_le_bytes());
                    }
                    count += 1;
                }
            }
            views
                .push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bin.len() - offset}));
            accessors.push(json!({
                "bufferView": views.len() - 1,
                "componentType": 5126,
                "count": count,
                "type": "VEC3",
                "min": lo,
                "max": hi,
            }));
            let mut primitive = json!({"attributes": {"POSITION": accessors.len() - 1}});
            if let Some(&mode) = self.modes.get(m) {
                primitive["mode"] = json!(mode);
            }
            meshes.push(json!({"primitives": [primitive]}));
        }
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| {
                let mut node = json!({"name": n.name});
                if let Some(mesh) = n.mesh {
                    node["mesh"] = json!(mesh);
                }
                if let Some(matrix) = n.matrix {
                    node["matrix"] = json!(matrix);
                } else {
                    if n.translation != [0.0; 3] {
                        node["translation"] = json!(n.translation);
                    }
                    if n.rotation != [0.0, 0.0, 0.0, 1.0] {
                        node["rotation"] = json!(n.rotation);
                    }
                    if n.scale != [1.0; 3] {
                        node["scale"] = json!(n.scale);
                    }
                }
                if !n.children.is_empty() {
                    node["children"] = json!(n.children);
                }
                if !n.extensions.as_object().is_some_and(|o| o.is_empty()) {
                    node["extensions"] = n.extensions.clone();
                }
                if !n.extras.is_null() {
                    node["extras"] = n.extras.clone();
                }
                node
            })
            .collect();
        let mut doc = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": self.roots}],
            "nodes": nodes,
            "meshes": meshes,
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": views,
            "accessors": accessors,
        });
        if !self.asset_extras.is_null() {
            doc["asset"]["extras"] = self.asset_extras.clone();
        }
        let used: Vec<String> = self
            .nodes
            .iter()
            .flat_map(|n| {
                n.extensions
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();
        if !used.is_empty() {
            doc["extensionsUsed"] = json!(used);
        }
        pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap()
    }

    /// What [`gamecraft_project::inspect`] reads in it.
    pub fn inspect(&self) -> Result<gamecraft_project::ModelSummary, String> {
        let bytes = self.glb();
        gamecraft_project::inspect(&bytes, &|_: &str| None)
    }
}

/// The 36 corners of a unit cube standing on y = 0, two triangles per side, counter-clockwise
/// from outside.
fn cube() -> Vec<[f32; 3]> {
    // Each side: its normal axis and sign; the four corners follow from the other two axes.
    let mut out = Vec::new();
    for (axis, sign) in [(0, 0.0), (0, 1.0), (1, 0.0), (1, 1.0), (2, 0.0), (2, 1.0)] {
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let corner = |u: f32, v: f32| {
            let mut p = [0.0; 3];
            p[axis] = sign;
            p[a] = u;
            p[b] = v;
            p
        };
        let c00 = corner(0.0, 0.0);
        let c10 = corner(1.0, 0.0);
        let c11 = corner(1.0, 1.0);
        let c01 = corner(0.0, 1.0);
        // (a, b) is a right-handed pair around `axis`, so this winding faces outwards where
        // sign is 1 and inwards where it is 0.
        let quad = if sign == 1.0 {
            [c00, c10, c11, c00, c11, c01]
        } else {
            [c00, c11, c10, c00, c01, c11]
        };
        out.extend(quad);
    }
    out
}
