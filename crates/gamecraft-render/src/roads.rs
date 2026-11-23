//! Road surfaces as meshes for the renderer, GPU-free: every `road` of a world draped on its
//! terrain ([`gamecraft_world::Road::drape`]) and merged into one mesh per terrain chunk of
//! [`mesh::TERRAIN_CHUNK_CELLS`] cells and colour, so a city of thousands of roads is a few
//! dozen draws that the renderer culls chunk by chunk.

use crate::mesh::{self, MeshData};
use gamecraft_world::{HeightField, World};
use std::collections::BTreeMap;

/// The surfaces of one colour in one terrain chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadChunk {
    /// The chunk: its column and row, in chunks of [`mesh::TERRAIN_CHUNK_CELLS`] cells.
    pub chunk: [usize; 2],
    /// Linear RGB.
    pub color: [f32; 3],
    /// The polygons as fans, world positions.
    pub mesh: MeshData,
}

/// Every road of a world as meshes, with what it took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadMeshes {
    /// In (chunk row, chunk column, colour) order.
    pub chunks: Vec<RoadChunk>,
    /// Roads drawn.
    pub roads: u64,
    /// Triangles of all chunks.
    pub triangles: u64,
    /// Terrain triangles the roads were clipped against (the work, AGENTS.md rule 18).
    pub clips: u64,
    /// Roads that are not drawn, as sentences.
    pub problems: Vec<String>,
}

/// The meshes of every road of `world` over `field` (`None`: the world has no terrain, and
/// roads are not drawn, with a problem line). Deterministic: the same world and field give
/// the same meshes, to the bit.
pub fn road_meshes(world: &World, field: Option<&HeightField>) -> RoadMeshes {
    let mut out = RoadMeshes::default();
    let roads: Vec<_> = world
        .entities()
        .filter_map(|e| Some((e, e.road()?)))
        .collect();
    if roads.is_empty() {
        return out;
    }
    let Some(field) = field else {
        out.problems.push(format!(
            "{} roads are not drawn: a road lies on the terrain and the world has none; add one \
             with terrain.set",
            roads.len()
        ));
        return out;
    };
    let mut groups: BTreeMap<(usize, usize, [u32; 3]), MeshData> = BTreeMap::new();
    for (e, road) in roads {
        let color = road.color.map(f32::to_bits);
        let result = road.drape(e, field, &mut |p| {
            let key = (
                p.row / mesh::TERRAIN_CHUNK_CELLS,
                p.col / mesh::TERRAIN_CHUNK_CELLS,
                color,
            );
            let m = groups.entry(key).or_default();
            let Ok(base) = u32::try_from(m.positions.len()) else {
                return;
            };
            let Ok(n) = u32::try_from(p.positions.len()) else {
                return;
            };
            if base.checked_add(n).is_none() {
                return;
            }
            m.positions
                .extend(p.positions.iter().map(|v| v.map(|c| c as f32)));
            m.normals
                .extend(p.normals.iter().map(|v| v.map(|c| c as f32)));
            for i in 1..n.saturating_sub(1) {
                m.indices.extend([base, base + i, base + i + 1]);
            }
        });
        match result {
            Ok(stats) => {
                out.roads += 1;
                out.triangles += stats.triangles;
                out.clips += stats.clips;
            }
            Err(err) => out.problems.push(format!(
                "The road of entity {} ({:?}) is not drawn: {err}.",
                e.id, e.name
            )),
        }
    }
    out.chunks = groups
        .into_iter()
        .filter(|(_, m)| !m.indices.is_empty())
        .map(|((row, col, color), mesh)| RoadChunk {
            chunk: [col, row],
            color: color.map(f32::from_bits),
            mesh,
        })
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_world::{Component, EntityDraft, Road, Terrain, TerrainSource};

    /// A slope of 1 in 12 rising to the north with a cross slope, 256 m at 2 m cells.
    fn sloped() -> (World, HeightField) {
        let cols = 17u32;
        let heights: Vec<f64> = (0..cols * cols)
            .map(|i| {
                let (c, r) = (f64::from(i % cols), f64::from(i / cols));
                (16.0 - r) * 16.0 / 12.0 + c * 0.3 + if i % 4 == 0 { 0.8 } else { 0.0 }
            })
            .collect();
        let terrain = Terrain::new(
            256.0,
            2.0,
            TerrainSource::Grid {
                cols,
                rows: cols,
                heights: heights.into(),
            },
        );
        let field = terrain.build().unwrap();
        let mut world = World::new("roads", 1);
        world.terrain = Some(terrain);
        (world, field)
    }

    fn add(world: &mut World, road: Road) {
        world
            .add_entity(EntityDraft {
                components: vec![Component::Road(road)],
                ..EntityDraft::default()
            })
            .unwrap();
    }

    #[test]
    fn roads_are_merged_per_chunk_and_colour_and_counted() {
        let (mut world, field) = sloped();
        add(&mut world, Road::new(vec![[0.0, 120.0], [0.0, -120.0]]));
        add(
            &mut world,
            Road {
                tier: 1,
                ..Road::new(vec![[-120.0, 3.0], [120.0, -3.0]])
            },
        );
        add(
            &mut world,
            Road {
                color: [0.2, 0.15, 0.1],
                tier: 2,
                ..Road::new(vec![[-100.0, -100.0], [100.0, 100.0]])
            },
        );
        let built = road_meshes(&world, Some(&field));
        assert!(built.problems.is_empty(), "{:?}", built.problems);
        assert_eq!(built.roads, 3);
        // 64-cell chunks of 2 m: a 2 by 2 grid of chunks, the roads crossing at its centre, so
        // each chunk holds asphalt and a corner of the brown diagonal.
        let chunks: Vec<([usize; 2], bool)> = built
            .chunks
            .iter()
            .map(|c| (c.chunk, c.color == [0.2, 0.15, 0.1]))
            .collect();
        assert_eq!(
            chunks,
            [
                ([0, 0], false),
                ([0, 0], true),
                ([1, 0], false),
                ([1, 0], true),
                ([0, 1], false),
                ([0, 1], true),
                ([1, 1], false),
                ([1, 1], true),
            ]
        );
        let triangles: usize = built.chunks.iter().map(|c| c.mesh.triangle_count()).sum();
        assert_eq!(triangles as u64, built.triangles);
        // Each road on its own gives the same triangles.
        let alone: u64 = world
            .entities()
            .map(|e| e.road().unwrap().mesh(e, &field).unwrap().stats.triangles)
            .sum();
        assert_eq!(alone, built.triangles);
        assert!(built.clips >= built.triangles / 4, "{built:?}");
        // Every vertex lies the road's offset above the ground (f32 at the GPU boundary).
        for c in &built.chunks {
            for p in &c.mesh.positions {
                let ground = field.height_at(f64::from(p[0]), f64::from(p[2])).unwrap();
                let above = f64::from(p[1]) - ground;
                assert!(above > 0.029 && above < 0.051, "{p:?}: {above}");
            }
        }
        // The same world, the same bytes.
        assert_eq!(road_meshes(&world, Some(&field)), built);
    }

    #[test]
    fn without_terrain_or_too_big_a_road_says_why() {
        let (mut world, field) = sloped();
        add(&mut world, Road::new(vec![[0.0, 10.0], [0.0, -10.0]]));
        let mut bare = world.clone();
        bare.terrain = None;
        let none = road_meshes(&bare, None);
        assert!(none.chunks.is_empty());
        assert!(
            none.problems[0].contains("terrain.set"),
            "{:?}",
            none.problems
        );
        // 200 m wide, corner to corner 64 times: far more clipping than one road may take.
        let zigzag = (0..64)
            .map(|i| {
                if i % 2 == 0 {
                    [-120.0, -120.0]
                } else {
                    [120.0, 120.0]
                }
            })
            .collect();
        add(
            &mut world,
            Road {
                width_m: 200.0,
                ..Road::new(zigzag)
            },
        );
        let built = road_meshes(&world, Some(&field));
        assert_eq!(built.roads, 1, "the small road is still drawn");
        assert!(
            built.problems[0].contains("entity 2"),
            "{:?}",
            built.problems
        );
    }
}
