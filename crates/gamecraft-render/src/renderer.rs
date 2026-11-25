//! The world renderer: the only module that touches VisionCraft's renderer
//! (`visioncraft-render`). A VisionCraft API change lands here.

use crate::blit::Blit;
use crate::culling;
use crate::effects::{self, Effects, Segment};
use crate::light;
use crate::mesh::{self, MeshData};
use crate::model::{self, Alpha, ModelMaterial, TexRef};
use crate::plants::{self, PartKind, PlantImage};
use crate::{Camera, FrameStats, LevelCount, ModelStats, Quality, RenderError};
#[cfg(not(target_arch = "wasm32"))]
use crate::{Picture, PictureRequest, PictureStats};
use gamecraft_math::{DQuat, DVec3, deg_to_rad};
use gamecraft_project::lod::{Levels, LodDistances, LodOverride, follows_scale};
use gamecraft_project::model::resolve_uri;
use gamecraft_world::{
    Character, Entity, EntityId, Environment, HeightField, Model, Plant, Road, Shape, ShapeKind,
    Terrain, TerrainLayer, World,
};
use glam::{Mat4, Vec3};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use visioncraft_render as vc;

/// The output format: sRGB-encoded RGBA8.
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The size of the renderer's live targets until [`WorldRenderer::resize`]. Pictures are
/// offscreen stills at their own size; a renderer that only makes pictures keeps them tiny.
const LIVE_SIZE: (u32, u32) = (64, 36);
/// The largest side of the live targets, pixels.
const LIVE_MAX_SIDE: u32 = 8192;
/// Roughness of shapes and character bodies.
const SHAPE_ROUGHNESS: f32 = 0.6;
/// Roughness of road surfaces (worn asphalt and packed earth).
const ROAD_ROUGHNESS: f32 = 0.85;
/// A character's body: safety orange, linear RGB, clear against grass, rock and sky.
const CHARACTER_COLOR: [f32; 3] = [0.85, 0.30, 0.03];
/// A character's visor: near black.
const VISOR_COLOR: [f32; 3] = [0.02, 0.02, 0.025];
/// The terrain layers' linear albedo: grass, bare earth, rock (`mesh::LAYER_*` order).
const LAYER_COLORS: [[f32; 4]; mesh::TERRAIN_LAYERS] = [
    [0.075, 0.13, 0.035, 1.0],
    [0.20, 0.15, 0.095, 1.0],
    [0.28, 0.27, 0.25, 1.0],
];
/// The endless ground plane's linear albedo: the grass layer, a shade darker (far land).
const GROUND_COLOR: [f32; 3] = [0.065, 0.11, 0.03];
/// With painted layers the ground plane is the first layer's colour times this (far land).
const GROUND_SHADE: f32 = 0.87;
/// How much the shading of a painted layer varies (VisionCraft's `variation`).
const LAYER_VARIATION: f32 = 0.5;
/// How far sun shadows reach: twice the distance to the point looked at, within these
/// bounds (metres).
const SHADOW_SHARE: f64 = 2.0;
const SHADOW_MIN_M: f64 = 80.0;
const SHADOW_MAX_M: f64 = 3000.0;
/// The share of the sun's glow in the haze by day (VisionCraft's `studio.rs`).
const SUN_GLOW: f64 = 0.5;

/// How bright an impact puff glows, cd/m2 per unit of its emissive colour.
const PUFF_EMISSIVE_STRENGTH: f32 = 6_000.0;
/// A glTF emissive colour of 1 glows with this luminance, cd/m2 (glTF gives no unit; this is a
/// lit panel's, bright at dusk and readable by day).
const GLTF_EMISSIVE_CD_M2: f32 = 1_000.0;
/// The stand-in of a model that cannot be drawn: magenta, so it is seen.
const MISSING_MODEL_COLOR: [f32; 3] = [1.0, 0.0, 1.0];
/// The stand-in's side, metres (times the entity's and the model's scale).
const MISSING_MODEL_SIZE_M: f64 = 0.5;
/// Most entities a problem line about one model names.
const MAX_NAMED_ENTITIES: usize = 8;
/// What is passed as the renderer's `band_m` of a model item: any value above 0 switches on
/// the dissolve of the cells that cross the draw distance while the camera moves (a parked
/// camera shows no dither); the value is not a distance.
const MODEL_CULL_FADE: f32 = 1.0;
/// The step of the buckets of scale that entities sharing a model item are sorted into when
/// the item's distances follow the scale (a scattered field of copies, each its own size):
/// about a tenth of the draw distance between one bucket and the next.
const SCALE_STEP: f64 = 1.25;

/// A character that is down lies on its side; the base of its body is turned about this.
const LYING_DEG: f64 = 90.0;

/// A device without a window (native), or an error that says why there is none.
#[cfg(not(target_arch = "wasm32"))]
pub fn headless_gpu() -> Result<(wgpu::Device, wgpu::Queue), RenderError> {
    pollster::block_on(vc::headless_device()).map_err(RenderError::NoGpu)
}

/// The unit meshes every shape is drawn with.
#[derive(Clone, Copy)]
struct UnitMeshes {
    cube: vc::MeshId,
    sphere: vc::MeshId,
    cylinder: vc::MeshId,
}

impl UnitMeshes {
    fn of(&self, kind: ShapeKind) -> vc::MeshId {
        match kind {
            ShapeKind::Box => self.cube,
            ShapeKind::Sphere => self.sphere,
            ShapeKind::Cylinder => self.cylinder,
        }
    }
}

/// What the last sync added to the renderer, freed by the next rebuild.
#[derive(Default)]
struct Built {
    items: Vec<vc::ItemId>,
    meshes: Vec<vc::MeshId>,
    materials: Vec<vc::MaterialId>,
    textures: Vec<vc::TextureId>,
    /// The character items, whose instances follow the characters without a rebuild.
    characters: Vec<CharacterGroup>,
    /// The model items, for the count of what a frame drew ([`WorldRenderer::render`]).
    models: Vec<ModelItem>,
}

/// One character item: the characters it draws, in instance order, and where it drew them.
struct CharacterGroup {
    item: vc::ItemId,
    ids: Vec<EntityId>,
    instances: Vec<Mat4>,
    body: Body,
    /// Its row among [`Built::models`] when it draws a model.
    model: Option<usize>,
}

/// One model item: what is needed to count again what the renderer draws of it.
struct ModelItem {
    asset: String,
    /// The item's instances, as last handed to the renderer.
    instances: Vec<Mat4>,
    /// Triangles each level submits per instance, finest first.
    triangles: Vec<u64>,
    /// The level of the file each of those is (a level without geometry is left out).
    drawn: Vec<usize>,
    /// The box of every level's meshes, in the model's own axes.
    bounds: (Vec3, Vec3),
    /// Where each level ends, metres; the last is the draw distance.
    until_m: Vec<f64>,
}

/// What decides the levels of detail of the models an item draws. Entities that agree share
/// the item; the numbers are kept as their bits, so that a map can order them.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModelKey {
    /// The component's switch distances, as their bits (none given: empty).
    lod_distances_m: Vec<u64>,
    /// The component's draw distance, as its bits.
    draw_distance_m: Option<u64>,
    /// Where a distance follows the model's scale: the upper bound of the bucket of scales
    /// (steps of [`SCALE_STEP`]) the item's distances are computed for, as its bits, so that
    /// a model of the bucket never disappears early. `None` when every distance is given in
    /// metres: entities of every scale then share the item.
    scale: Option<u64>,
    /// The item draws the character the camera follows, which is never culled by distance.
    followed: bool,
}

impl ModelKey {
    /// The switch distances the component gives.
    fn distances(&self) -> LodOverride {
        LodOverride {
            switch_m: Some(
                self.lod_distances_m
                    .iter()
                    .map(|b| f64::from_bits(*b))
                    .collect(),
            ),
            draw_m: self.draw_distance_m.map(f64::from_bits),
            coverage: None,
        }
    }

    /// The scale the item's distances are computed for: the bucket's upper bound, or 1 when
    /// they do not follow the scale.
    fn scale(&self) -> f64 {
        self.scale.map_or(1.0, f64::from_bits)
    }
}

/// The upper bound of the bucket of scales in steps of [`SCALE_STEP`] that holds `scale`.
/// A scale that is not a positive finite number counts as 1.
fn scale_bucket(scale: f64) -> f64 {
    if !(scale.is_finite() && scale > 0.0) {
        return 1.0;
    }
    let mut upper = 1.0;
    let mut steps = 0usize;
    while scale > upper && steps < 512 {
        upper *= SCALE_STEP;
        steps += 1;
    }
    while scale <= upper / SCALE_STEP && steps < 1024 {
        upper /= SCALE_STEP;
        steps += 1;
    }
    upper
}

/// What a character item draws, which says how it places its characters.
#[derive(Clone, Copy)]
enum Body {
    /// The capsule and visor of one character size, unscaled; a downed body lies `radius_m`
    /// (the body's radius, metres) above the ground.
    Capsule { radius_m: f64 },
    /// The character's model, or its magenta stand-in: `size` times the model's scale times
    /// the entity's, as a model without a character is placed.
    Model { size: f64 },
}

/// Where a character item draws `e`: standing at its position, turned to its facing, or
/// lying on its side when it is down.
fn character_pose(e: &Entity, body: Body) -> Mat4 {
    let (scale, radius_m) = match body {
        Body::Capsule { radius_m } => (1.0, radius_m),
        Body::Model { size } => (
            size * e.model().map_or(1.0, |m| m.scale) * e.scale,
            e.character().map_or(0.0, |c| c.radius_m),
        ),
    };
    if e.is_down() {
        lying(e.position, e.rotation_y_deg, radius_m, scale)
    } else {
        placement(e.position, e.rotation_y_deg, [scale; 3])
    }
}

/// What of an entity the scene's geometry is built from. A character's position and rotation
/// are left out (unless it also has a shape or a plant): a character that walks moves its
/// instance (its capsule's or its model's), it does not rebuild the scene.
#[derive(PartialEq)]
struct DrawnEntity {
    id: EntityId,
    pose: Option<([f64; 3], f64)>,
    scale: f64,
    shape: Option<Shape>,
    plant: Option<Plant>,
    character: Option<Character>,
    model: Option<Model>,
    road: Option<Road>,
}

/// Whether an entity draws anything (a zone, a light or tags alone draw nothing).
fn draws(e: &Entity) -> bool {
    e.shape().is_some()
        || e.plant().is_some()
        || e.character().is_some()
        || e.model().is_some()
        || e.road().is_some()
}

impl DrawnEntity {
    /// What of `e` is drawn; `None` when it draws nothing (a zone, a light, tags).
    fn of(e: &Entity) -> Option<DrawnEntity> {
        let (shape, plant, character) = (e.shape(), e.plant(), e.character());
        if !draws(e) {
            return None;
        }
        let walks = character.is_some() && shape.is_none() && plant.is_none();
        Some(DrawnEntity {
            id: e.id,
            pose: (!walks).then_some((e.position, e.rotation_y_deg)),
            scale: e.scale,
            shape: shape.cloned(),
            plant: plant.cloned(),
            character: character.cloned(),
            model: e.model().cloned(),
            road: e.road().cloned(),
        })
    }

    /// Whether `e` draws the same as this.
    fn is(&self, e: &Entity) -> bool {
        let walks = self.pose.is_none();
        self.id == e.id
            && self.scale == e.scale
            && self.shape.as_ref() == e.shape()
            && self.plant.as_ref() == e.plant()
            && self.character.as_ref() == e.character()
            && self.model.as_ref() == e.model()
            && self.road.as_ref() == e.road()
            && (walks || self.pose == Some((e.position, e.rotation_y_deg)))
    }
}

/// What the scene's geometry was built from; a sync with the same skips the rebuild. The
/// height field is derived from the terrain recipe, so the recipe and the field's shape stand
/// for its samples (comparing millions of samples every frame would not be cheap).
struct GeometryKey {
    terrain: Option<Terrain>,
    heights: Option<FieldShape>,
    entities: Vec<DrawnEntity>,
    /// [`WorldRenderer::provide_asset`] calls so far: a new file rebuilds.
    assets: u64,
}

/// The grid of a height field, without its samples.
#[derive(Clone, Copy, PartialEq)]
struct FieldShape {
    cols: usize,
    rows: usize,
    cell_m: f64,
    min: [f64; 2],
    layers: usize,
}

impl FieldShape {
    fn of(h: &HeightField) -> FieldShape {
        FieldShape {
            cols: h.cols(),
            rows: h.rows(),
            cell_m: h.cell_m(),
            min: h.min(),
            layers: h.layer_count(),
        }
    }
}

impl GeometryKey {
    fn of(world: &World, heights: Option<&HeightField>, assets: u64) -> GeometryKey {
        GeometryKey {
            terrain: world.terrain.clone(),
            heights: heights.map(FieldShape::of),
            entities: world.entities().filter_map(DrawnEntity::of).collect(),
            assets,
        }
    }

    fn matches(&self, world: &World, heights: Option<&HeightField>, assets: u64) -> bool {
        if self.terrain != world.terrain
            || self.heights != heights.map(FieldShape::of)
            || self.assets != assets
        {
            return false;
        }
        let mut drawn = world.entities().filter(|e| draws(e));
        self.entities
            .iter()
            .all(|k| drawn.next().is_some_and(|e| k.is(e)))
            && drawn.next().is_none()
    }
}

/// A species' uploaded textures.
#[derive(Clone, Copy)]
struct PlantTextures {
    atlas: vc::TextureId,
    atlas_normal: Option<vc::TextureId>,
    bark: vc::TextureId,
    bark_normal: vc::TextureId,
}

/// Draws GameCraft worlds with VisionCraft's renderer on one wgpu device.
///
/// [`WorldRenderer::sync`] makes the renderer's scene match a world; [`WorldRenderer::render`]
/// draws it offscreen and reads the pixels back; [`WorldRenderer::draw`] records a live frame
/// into a window's surface (after [`WorldRenderer::resize`] and
/// [`WorldRenderer::set_quality`]).
pub struct WorldRenderer {
    renderer: vc::Renderer,
    device: wgpu::Device,
    blit: Blit,
    /// The live targets' size, pixels.
    size: (u32, u32),
    /// The environment and whether lamps shone at the last sync: unchanged, nothing is set.
    environment: Option<(Environment, bool)>,
    /// The lights handed to the renderer at the last sync.
    spots: Vec<light::Spot>,
    /// Lights the renderer had to leave out at the last change of the lights, as a sentence.
    light_problem: Option<String>,
    /// The character left out of the picture (a first-person view's own body).
    hidden: Option<EntityId>,
    /// How many times the scene's geometry was built.
    rebuilds: u64,
    units: Option<UnitMeshes>,
    /// Terrain chunks are shaded by the layers; the renderer still wants a material per item.
    terrain_material: vc::MaterialId,
    /// The painted layers handed to the renderer (empty: the built-in grass, earth and rock).
    layers: Vec<TerrainLayer>,
    built: Built,
    /// The item of impact puffs and its material (kept for the renderer's life), when the
    /// sphere mesh could be uploaded.
    puffs: Option<vc::ItemId>,
    /// The puffs' instances now, so an unchanged frame uploads nothing.
    puff_instances: Vec<Mat4>,
    /// The effects' overlay lines now.
    tracer_lines: Vec<vc::LineVertex>,
    /// Whether the live frames carry a crosshair.
    crosshair: bool,
    synced: Option<GeometryKey>,
    problems: Vec<String>,
    /// Problems of setting the renderer up, repeated with every sync.
    setup_problems: Vec<String>,
    /// Asset files the host handed over, by path inside a pack's `assets/` folder.
    asset_files: BTreeMap<String, Vec<u8>>,
    /// [`WorldRenderer::provide_asset`] calls so far.
    assets_revision: u64,
    /// The models uploaded, by asset path, kept from one rebuild to the next; or why one
    /// cannot be drawn.
    models: BTreeMap<String, Result<ResidentModel, FailedModel>>,
    /// Model uploads so far: one per asset (and again after its files change).
    model_uploads: u64,
    /// The character the camera follows (a third-person or first-person view): its model is
    /// never culled by distance.
    followed: Option<EntityId>,
    /// The counts of the last live frame, kept here: the renderer's own statistics are also
    /// written by a picture, which is drawn through the same frame path.
    live_stats: Option<FrameStats>,
    /// What the renderer divides the distances of the levels by in a live frame: the live
    /// quality's share (40 % low, 50 % medium, 100 % high), as for everything it draws. A
    /// picture is always drawn at the highest quality, where it is 1.
    lod_scale: f32,
}

/// A model that could not be read or uploaded: why, and the asset files it was being read
/// from (a new version of one of them tries again, as for a model that was drawn).
struct FailedModel {
    why: String,
    files: Vec<String>,
}

/// A model on the GPU: its levels of detail, and everything to free with it.
struct ResidentModel {
    /// One entry per level of detail, finest first. A model without any has one.
    levels: Vec<ResidentLevel>,
    /// The box of every level's vertices, the model's own axes (the box the renderer
    /// culls by, of the meshes as uploaded).
    bounds: (Vec3, Vec3),
    /// The extent of the model across, metres: the size the rule from the file follows.
    size_m: f64,
    /// The levels of the file: the ones that draw (in the order of `levels`), what the file
    /// says of the distances, and the sentences about a broken set (said once per build that
    /// draws the model).
    read: Levels,
    meshes: Vec<vc::MeshId>,
    materials: Vec<vc::MaterialId>,
    textures: Vec<vc::TextureId>,
    /// The asset files it was read from (a new version of one uploads it again).
    files: Vec<String>,
    /// Textures that could not be decoded, as a sentence; the model is drawn without them.
    note: Option<String>,
}

/// One level of detail of a model on the GPU: the meshes it draws (one per material and set
/// of levels that share them) and their triangles. A mesh every level draws is in each.
struct ResidentLevel {
    parts: Vec<(vc::MeshId, vc::MaterialId)>,
    triangles: u64,
}

impl ResidentModel {
    fn free(self, r: &mut vc::Renderer) {
        for id in self.meshes {
            r.remove_mesh(id);
        }
        for id in self.materials {
            r.remove_material(id);
        }
        for id in self.textures {
            r.remove_texture(id);
        }
    }
}

impl WorldRenderer {
    /// A renderer on `device`, with the physical sky, the three terrain layers and the unit
    /// shapes uploaded, and the default environment. Pipelines are compiled here, so this
    /// takes a moment.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> WorldRenderer {
        let mut renderer = vc::Renderer::new(&device, &queue, OUTPUT_FORMAT);
        renderer.resize(LIVE_SIZE.0, LIVE_SIZE.1);
        let blit = Blit::new(&device);
        renderer.set_quality(vc::Quality::LOW);
        let mut setup_problems = Vec::new();
        if let Err(e) = renderer.set_sky(&vc::Sky::Physical) {
            setup_problems.push(format!("The physical sky could not be set up: {e}."));
        }
        if let Err(e) = renderer.set_terrain_layers(&default_layers()) {
            setup_problems.push(format!("The terrain layers could not be set up: {e}."));
        }
        let mut unit = |m: MeshData| upload_mesh(&mut renderer, &m);
        let units = match (
            unit(mesh::unit_box()),
            unit(mesh::unit_sphere()),
            unit(mesh::unit_cylinder()),
        ) {
            (Ok(cube), Ok(sphere), Ok(cylinder)) => Some(UnitMeshes {
                cube,
                sphere,
                cylinder,
            }),
            (a, b, c) => {
                let why: Vec<String> = [a, b, c].into_iter().filter_map(Result::err).collect();
                setup_problems.push(format!(
                    "The shape meshes could not be uploaded, so shapes are not drawn: {}.",
                    why.join("; ")
                ));
                None
            }
        };
        let terrain_material = renderer.add_material(&vc::Material::default());
        let puffs = units.map(|u| {
            let material = renderer.add_material(&vc::Material {
                base_color: [0.9, 0.4, 0.1, 1.0],
                roughness: 1.0,
                emissive: effects::IMPACT_EMISSIVE,
                emissive_strength: PUFF_EMISSIVE_STRENGTH,
                ..Default::default()
            });
            renderer.add_lod_item(&[(u.sphere, material, f32::INFINITY)], &[])
        });
        // A camera sees the sun's glare and the air's own colour far off (VisionCraft's
        // studio sets both for every picture).
        renderer.set_sun_glare(1.0);
        renderer.set_aerial_perspective(true);
        let (power, saturation) = light::NATURAL_GRADE;
        renderer.set_grade(power, saturation);
        let mut world_renderer = WorldRenderer {
            renderer,
            device,
            blit,
            size: LIVE_SIZE,
            environment: None,
            spots: Vec::new(),
            light_problem: None,
            hidden: None,
            rebuilds: 0,
            units,
            terrain_material,
            layers: Vec::new(),
            built: Built::default(),
            puffs,
            puff_instances: Vec::new(),
            tracer_lines: Vec::new(),
            crosshair: false,
            synced: None,
            problems: setup_problems.clone(),
            setup_problems,
            asset_files: BTreeMap::new(),
            assets_revision: 0,
            models: BTreeMap::new(),
            model_uploads: 0,
            followed: None,
            live_stats: None,
            lod_scale: quality_lod_scale(&vc::Quality::LOW),
        };
        world_renderer.apply_environment(&Environment::default(), false);
        world_renderer
    }

    /// Makes the renderer's scene match `world`. Returns problems an agent can act on (an
    /// unknown plant species with the nearest known id, a mesh that could not be uploaded);
    /// they also come back with every picture until the next sync.
    ///
    /// `heights` is the world's height field when the caller has it; without it the terrain
    /// recipe is built here. The environment and the lights (every `light` component that is
    /// on) follow the world on every call. The geometry is rebuilt only when the terrain or
    /// what an entity draws changed, and everything the previous build added is freed first;
    /// a character that only walks or turns moves its instance, cheap enough for every frame.
    pub fn sync(&mut self, world: &World, heights: Option<&HeightField>) -> Vec<String> {
        let spots = light::spots(world);
        if spots != self.spots {
            let lights: Vec<vc::Light> = spots.iter().map(vc_light).collect();
            let dropped = self.renderer.set_lights(&lights);
            self.light_problem = (dropped > 0).then(|| {
                format!(
                    "{dropped} lights were left out where too many shine on one spot; spread \
                     the lights out or turn some off."
                )
            });
            self.spots = spots;
        }
        let lamps = !self.spots.is_empty();
        let environment_changed = self
            .environment
            .as_ref()
            .is_none_or(|(env, l)| *env != world.environment || *l != lamps);
        if environment_changed {
            self.apply_environment(&world.environment, lamps);
        }
        let unchanged = self
            .synced
            .as_ref()
            .is_some_and(|key| key.matches(world, heights, self.assets_revision));
        if unchanged {
            self.move_characters(world);
        } else {
            self.rebuild(world, heights);
            self.rebuilds += 1;
            self.synced = Some(GeometryKey::of(world, heights, self.assets_revision));
            self.move_characters(world);
        }
        self.problems()
    }

    /// The problems of the last sync, the lights' included.
    fn problems(&self) -> Vec<String> {
        let mut problems = self.problems.clone();
        problems.extend(self.light_problem.iter().cloned());
        problems
    }

    /// Leaves character `id` out of the picture from the next [`WorldRenderer::sync`] on (a
    /// first-person view's own body, which would fill the view from inside), or none. Its
    /// light still shines. No rebuild.
    pub fn set_hidden_character(&mut self, id: Option<EntityId>) {
        self.hidden = id;
    }

    /// Names the character the camera follows from the next [`WorldRenderer::sync`] on (a
    /// first-person or third-person view), or none. Its model is drawn at every distance: the
    /// draw distance that culls the other models never takes it away, though its levels change
    /// as those of any model. The scene is built again when the character changes.
    pub fn set_followed_character(&mut self, id: Option<EntityId>) {
        if self.followed != id {
            self.followed = id;
            self.synced = None;
        }
    }

    /// Hands the renderer what is in flight and what just hit: tracers become thin bright
    /// streaks (overlay lines, hidden behind terrain and shapes) and impacts small glowing puffs
    /// that swell and shrink away. Replaces the last effects; empty ones clear them. Cheap
    /// enough for every frame; no rebuild.
    pub fn set_effects(&mut self, effects: &Effects) {
        let lines: Vec<vc::LineVertex> = effects
            .tracers
            .iter()
            .flat_map(effects::tracer_segments)
            .flat_map(|s| line_pair(&s))
            .collect();
        if lines != self.tracer_lines {
            self.renderer.set_overlay_lines(&lines);
            self.tracer_lines = lines;
        }
        let instances: Vec<Mat4> = effects
            .impacts
            .iter()
            .filter_map(effects::impact_puff)
            .map(|(centre, radius)| effects::puff_matrix(centre, radius))
            .collect();
        if let Some(item) = self.puffs
            && instances != self.puff_instances
        {
            self.renderer.set_instances(item, &instances);
            self.puff_instances = instances;
        }
    }

    /// Whether live frames ([`WorldRenderer::draw`]) carry a crosshair at the centre: a small
    /// plus with a gap, drawn over everything. Off by default; pictures never carry one.
    pub fn set_crosshair(&mut self, on: bool) {
        self.crosshair = on;
    }

    /// How many times [`WorldRenderer::sync`] built the scene's geometry: a count of work for
    /// tests and measurements.
    pub fn rebuild_count(&self) -> u64 {
        self.rebuilds
    }

    /// Hands over the bytes of an asset file, by its path inside a pack's `assets/` folder
    /// (`models/crate.glb`; a `.gltf`'s buffers under their own paths beside it; the textures
    /// of the pack's store as `textures/<content-hash>.<ext>`, whatever model refers to them).
    /// A path with `..` in it is resolved first (`models/../textures/x.png` is
    /// `textures/x.png`). A model whose file this replaces, or that refers to it, is read and
    /// uploaded again at the next [`WorldRenderer::sync`]. The host reads the files (the CLI
    /// and the window from the open project, the browser by fetching them).
    pub fn provide_asset(&mut self, path: &str, bytes: Vec<u8>) {
        let plain = resolve_uri("", path).unwrap_or_else(|_| path.to_string());
        let path = plain.as_str();
        if self.asset_files.get(path) == Some(&bytes) {
            return;
        }
        self.asset_files.insert(path.to_string(), bytes);
        self.assets_revision += 1;
        let stale: Vec<String> = self
            .models
            .iter()
            .filter(|(asset, m)| {
                *asset == path
                    || m.as_ref().is_ok_and(|m| m.files.iter().any(|f| f == path))
                    || m.as_ref().is_err_and(|f| f.files.iter().any(|f| f == path))
            })
            .map(|(asset, _)| asset.clone())
            .collect();
        for asset in stale {
            if let Some(Ok(m)) = self.models.remove(&asset) {
                m.free(&mut self.renderer);
            }
        }
    }

    /// How many models were read and uploaded so far: one per asset, however many entities
    /// show it (a count of work for tests).
    pub fn model_uploads(&self) -> u64 {
        self.model_uploads
    }

    /// Sets the live frames' size in pixels (each side 1 to 8192): call it when the window's
    /// surface changes size. Pictures from [`WorldRenderer::render`] keep their own size.
    pub fn resize(&mut self, width: u32, height: u32) {
        let size = (
            width.clamp(1, LIVE_MAX_SIDE),
            height.clamp(1, LIVE_MAX_SIDE),
        );
        if size != self.size {
            self.size = size;
            self.renderer.resize(size.0, size.1);
        }
    }

    /// The live frames' size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The effects of live frames ([`Quality`]); pictures are always drawn at the highest.
    pub fn set_quality(&mut self, quality: Quality) {
        let preset = match quality {
            Quality::Low => vc::Quality::LOW,
            Quality::Medium => vc::Quality::MEDIUM,
            Quality::High => vc::Quality::HIGH,
        };
        // The presets' fast-shading shaders leave holes in surfaces on Intel's Mesa Vulkan
        // driver (seen 2026-11-18 on Iris Xe, Mesa 26.2: the sky shows through a box), so live
        // frames use the full shaders at every quality until VisionCraft fixes that.
        self.renderer.set_quality(vc::Quality {
            fast_shading: false,
            ..preset
        });
        self.lod_scale = quality_lod_scale(&preset);
    }

    /// Records a live frame of the synced scene from `camera` into `target`, stretched to its
    /// size, with the wind's sway at `time_s`; submit `encoder` before the next frame.
    ///
    /// The target is a colour texture view in its texture's own format (a window's surface,
    /// or any single-sampled texture made with `RENDER_ATTACHMENT`); an sRGB format and a
    /// plain one give the same picture. The frame is drawn at the size of
    /// [`WorldRenderer::resize`]. Errors name what is wrong: a bad camera or clock, a target
    /// that cannot be drawn into.
    pub fn draw(
        &mut self,
        camera: &Camera,
        time_s: f64,
        target: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), RenderError> {
        camera.validate()?;
        if !time_s.is_finite() {
            return Err(RenderError::BadRequest(format!(
                "time_s {time_s} is not a finite number of seconds; use for example 0"
            )));
        }
        let format = Blit::check_target(&self.device, target)?;
        let aspect = f64::from(self.size.0) / f64::from(self.size.1);
        let (view, proj, eye) = self.aim(camera, aspect, time_s);
        self.renderer.set_camera(view, proj, eye);
        let sights: Vec<vc::LineVertex> = if self.crosshair {
            effects::crosshair_segments(camera, self.size.1)
                .iter()
                .flat_map(line_pair)
                .collect()
        } else {
            Vec::new()
        };
        self.renderer.set_overlay_lines_on_top(&sights);
        // Kept as soon as the frame is recorded: the renderer's statistics are overwritten by
        // the next picture, which goes through the same frame path.
        let stats = {
            let output = self.renderer.render(encoder);
            self.blit
                .record(&self.device, encoder, output, target, format);
            self.renderer.stats()
        };
        self.live_stats = Some(FrameStats {
            instances: u64::from(stats.instances),
            forward_triangles: forward_triangles(&stats),
        });
        Ok(())
    }

    /// Shadows and the clock for a view from `camera`; its matrices and eye.
    fn aim(&mut self, camera: &Camera, aspect: f64, time_s: f64) -> (Mat4, Mat4, Vec3) {
        let reach =
            (DVec3::from_array(camera.target) - DVec3::from_array(camera.position)).length();
        self.renderer
            .set_shadow_distance((reach * SHADOW_SHARE).clamp(SHADOW_MIN_M, SHADOW_MAX_M) as f32);
        self.renderer.set_time(time_s);
        camera.matrices(aspect)
    }

    /// Moves every character item's instances to where its characters are now.
    fn move_characters(&mut self, world: &World) {
        let hidden = self.hidden;
        let Built {
            characters, models, ..
        } = &mut self.built;
        for group in characters {
            let instances: Vec<Mat4> = group
                .ids
                .iter()
                .filter(|&&id| Some(id) != hidden)
                .filter_map(|&id| world.entity(id))
                .map(|e| character_pose(e, group.body))
                .collect();
            if instances != group.instances {
                self.renderer.set_instances(group.item, &instances);
                if let Some(row) = group.model.and_then(|i| models.get_mut(i)) {
                    row.instances.clone_from(&instances);
                }
                group.instances = instances;
            }
        }
    }

    /// Draws the synced scene and waits for the pixels (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&mut self, request: &PictureRequest) -> Result<Picture, RenderError> {
        request.validate()?;
        let (width, height) = (request.width, request.height);
        let aspect = f64::from(width) / f64::from(height);
        let (view, proj, eye) = self.aim(&request.camera, aspect, request.time_s);
        let rgba = self
            .renderer
            .render_still(width, height, view, proj, eye)
            .and_then(vc::Still::wait)
            .map_err(RenderError::Failed)?;
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            return Err(RenderError::Failed(format!(
                "the GPU returned {} bytes for a {width} x {height} picture, {expected} expected",
                rgba.len()
            )));
        }
        let stats = self.renderer.stats();
        // A picture starts settled and is drawn at the highest quality: the metres as written.
        let forward_triangles = forward_triangles(&stats);
        let models = self.account(proj * view, eye, 1.0);
        Ok(Picture {
            width,
            height,
            rgba,
            stats: PictureStats {
                triangles: stats.triangles,
                draws: u64::from(stats.draw_calls),
                instances: u64::from(stats.instances),
                forward_triangles,
                models,
            },
            problems: self.problems(),
        })
    }

    /// What the models of the synced scene draw from `camera` in a view of width / height
    /// `aspect`: the rows a picture from it carries in [`PictureStats::models`], counted by
    /// the renderer's settled rule on the CPU ([`culling`]) at the live quality's share of
    /// the distances. For a live view that shows its numbers. Exact for a picture, which
    /// starts settled. A live frame may differ where a cell sits within about a metre of a
    /// threshold (1 m divided by the quality's share): the renderer keeps such a cell at the
    /// level of the frame before, by the side the camera came from.
    pub fn count_models(&self, camera: &Camera, aspect: f64) -> Vec<ModelStats> {
        let (view, proj, eye) = camera.matrices(aspect);
        self.account(proj * view, eye, self.lod_scale)
    }

    /// Counts of the last live frame ([`WorldRenderer::draw`]): the instances the view drew
    /// and the triangles its view pass submitted. Zeros before the first frame. A picture
    /// ([`WorldRenderer::render`]) does not change them: what it drew is in the picture.
    pub fn frame_stats(&self) -> FrameStats {
        self.live_stats.unwrap_or_default()
    }

    /// What each model item drew from `eye` through `view_proj`, counted again by the
    /// renderer's rule ([`culling`]).
    fn account(&self, view_proj: Mat4, eye: Vec3, lod_scale: f32) -> Vec<ModelStats> {
        self.built
            .models
            .iter()
            .map(|item| {
                let count = culling::count(
                    &item.instances,
                    &item.triangles,
                    item.bounds,
                    &item.until_m,
                    view_proj,
                    eye,
                    lod_scale,
                );
                ModelStats {
                    asset: item.asset.clone(),
                    instances: item.instances.len(),
                    drawn: count.drawn,
                    culled: count.culled,
                    levels: count
                        .levels
                        .into_iter()
                        .zip(&item.drawn)
                        .map(|((instances, triangles), &level)| LevelCount {
                            level,
                            instances,
                            triangles,
                        })
                        .collect(),
                    until_m: item.until_m.clone(),
                }
            })
            .collect()
    }

    /// How many draw items the renderer holds (terrain chunks, shape groups, characters,
    /// plants, not the impact puffs' own): for tests that a sync leaves nothing behind.
    pub fn item_count(&self) -> usize {
        self.renderer
            .item_count()
            .saturating_sub(usize::from(self.puffs.is_some()))
    }

    /// Sun, sky, clouds, mist, rain, wind, haze and exposure from the environment; `lamps`
    /// when lights shine (the exposure keeps dusk and night darker so their pools read).
    fn apply_environment(&mut self, env: &Environment, lamps: bool) {
        self.environment = Some((env.clone(), lamps));
        let r = &mut self.renderer;
        let sun = env.sun();
        r.set_sun(&vc::Sun::from_angles(
            sun.azimuth_deg as f32,
            sun.elevation_deg as f32,
        ));
        r.set_sky_scale(1.0);
        // The renderer takes the clouds' share of the sun itself (`Clouds::sun_share`).
        r.set_clouds(&vc::Clouds {
            cover: env.cloud_cover.clamp(0.0, 1.0) as f32,
            overcast: light::overcast(env.cloud_cover),
        });
        r.set_mist(env.mist as f32);
        r.set_wet(env.rain as f32, env.rain as f32);
        let [x, y, z] = light::wind_towards(env.wind_direction_deg);
        r.set_wind(&vc::Wind {
            direction: Vec3::new(x as f32, y as f32, z as f32),
            speed_mps: env.wind_speed_mps.max(0.0) as f32,
            gustiness: light::WIND_GUSTINESS as f32,
            gusts: light::WIND_GUSTS as f32,
        });
        let night = light::night(sun.elevation_deg);
        r.set_atmosphere(&vc::Atmosphere {
            haze: light::haze_per_km(sun.elevation_deg) as f32,
            haze_height_m: light::AEROSOL_HEIGHT_M as f32,
            sun_glow: (SUN_GLOW * (1.0 - night)) as f32,
            clear_m: 0.0,
        });
        r.set_exposure(light::exposure_ev(env, lamps) as f32);
        // A changed sky is met at once, not spread over the next frames (a live frame would
        // otherwise show the old sky in its haze and reflections for a while).
        r.finish_environment();
    }

    /// Frees the previous build and adds terrain, ground, shapes, characters and plants.
    fn rebuild(&mut self, world: &World, heights: Option<&HeightField>) {
        free(&mut self.renderer, &mut self.built);
        let mut problems = self.setup_problems.clone();
        let field: Option<Cow<'_, HeightField>> = match (heights, &world.terrain) {
            (Some(h), _) => Some(Cow::Borrowed(h)),
            (None, Some(terrain)) => match terrain.build() {
                Ok(h) => Some(Cow::Owned(h)),
                Err(e) => {
                    problems.push(format!("The terrain is not drawn: {e}"));
                    None
                }
            },
            (None, None) => None,
        };
        let layers = world
            .terrain
            .as_ref()
            .map(|t| t.layers.clone())
            .unwrap_or_default();
        if layers != self.layers {
            let set = if layers.is_empty() {
                default_layers().to_vec()
            } else {
                layers.iter().map(vc_layer).collect()
            };
            match self.renderer.set_terrain_layers(&set) {
                Ok(()) => self.layers = layers.clone(),
                Err(e) => problems.push(format!(
                    "The terrain's painted layers could not be set up: {e}."
                )),
            }
        }
        let ground_color = match layers.first() {
            Some(l) => l.color.map(|c| c as f32 * GROUND_SHADE),
            None => GROUND_COLOR,
        };
        let mut scene = Scene {
            r: &mut self.renderer,
            built: &mut self.built,
            problems: &mut problems,
            colors: BTreeMap::new(),
        };
        scene.roads(world, field.as_deref());
        match field.as_deref() {
            Some(h) => {
                let plane = border_mean(h);
                scene.terrain(h, plane, self.terrain_material);
                scene.r.set_ground(Some(vc::Ground {
                    height: plane as f32,
                    color: ground_color,
                    ..Default::default()
                }));
            }
            None => scene.r.set_ground(None),
        }
        if let Some(units) = self.units {
            scene.shapes(world, units);
        }
        scene.models(
            world,
            &mut self.models,
            &self.asset_files,
            &mut self.model_uploads,
            self.units,
            self.followed,
        );
        scene.characters(world);
        scene.plants(world);
        self.problems = problems;
    }
}

/// The built-in layers of a terrain without painted ones: grass, bare earth and rock.
fn default_layers() -> [vc::TerrainLayer; mesh::TERRAIN_LAYERS] {
    LAYER_COLORS.map(|base_color| vc::TerrainLayer {
        base_color,
        roughness: 0.9,
        variation: LAYER_VARIATION,
        ..Default::default()
    })
}

/// A painted layer for the renderer: its colour and roughness (texture maps when assets
/// exist).
fn vc_layer(l: &TerrainLayer) -> vc::TerrainLayer {
    let [r, g, b] = l.color.map(|c| c as f32);
    vc::TerrainLayer {
        base_color: [r, g, b, 1.0],
        roughness: l.roughness as f32,
        variation: LAYER_VARIATION,
        ..Default::default()
    }
}

/// The triangles the view pass of a frame submitted, of the renderer's statistics.
fn forward_triangles(stats: &vc::RenderStats) -> u64 {
    stats
        .pass_triangles
        .iter()
        .find(|(pass, _)| *pass == "forward")
        .map_or(0, |(_, triangles)| *triangles)
}

/// What the renderer divides the distances of the levels by at a quality: its
/// `lod_distance_percent` over 100 (40 low, 50 medium, 100 high, 200 ultra), as its own
/// `lod_scale` figures it, for a view distance GameCraft never changes from 1.
fn quality_lod_scale(quality: &vc::Quality) -> f32 {
    quality.lod_distance_percent.max(1) as f32 / 100.0
}

/// A light for the renderer.
fn vc_light(s: &light::Spot) -> vc::Light {
    let [x, y, z] = s.position;
    let [r, g, b] = s.color;
    vc::Light {
        position: Vec3::new(x as f32, y as f32, z as f32),
        kind: vc::LightKind::Point,
        color: Vec3::new(r, g, b),
        intensity: s.lumens as f32,
        range_m: s.range_m as f32,
    }
}

/// Removes everything a build added.
fn free(r: &mut vc::Renderer, built: &mut Built) {
    built.characters.clear();
    built.models.clear();
    for id in built.items.drain(..) {
        r.remove_item(id);
    }
    for id in built.meshes.drain(..) {
        r.remove_mesh(id);
    }
    for id in built.materials.drain(..) {
        r.remove_material(id);
    }
    for id in built.textures.drain(..) {
        r.remove_texture(id);
    }
}

/// Uploads a plain mesh.
fn upload_mesh(r: &mut vc::Renderer, m: &MeshData) -> Result<vc::MeshId, String> {
    r.add_mesh(&vc::Mesh {
        positions: &m.positions,
        normals: &m.normals,
        indices: &m.indices,
        face_ids: None,
        uvs: m.uvs.as_deref(),
        wind: None,
    })
}

/// How a texture is used, which decides its colour space and what is done to it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    /// Base colour: sRGB.
    Base,
    /// Normal map: linear.
    Normal,
    /// Metallic-roughness packed as occlusion-roughness-metallic, the occlusion in red kept.
    OrmWithOcclusion,
    /// Metallic-roughness packed as occlusion-roughness-metallic, red set to 1.
    Orm,
    /// Occlusion in red: linear.
    Occlusion,
}

/// Reads the model `path` from the handed-over `files` and uploads it: one mesh and one
/// material per glTF material and set of levels that share them, each image decoded and
/// uploaded once per role; its levels of detail as the file says them. Errors say why it
/// cannot be drawn; a texture that cannot be decoded is left out with a note.
fn upload_model(
    r: &mut vc::Renderer,
    path: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<ResidentModel, FailedModel> {
    let not_handed = |why: String, files: Vec<String>| Err(FailedModel { why, files });
    let bytes = match files.get(path) {
        Some(bytes) => bytes,
        None => {
            return not_handed(
                "its file was not handed to the renderer (the host reads it from the open \
                 project's packs; asset.list shows what they hold)"
                    .to_string(),
                vec![path.to_string()],
            );
        }
    };
    let folder = gamecraft_project::model::folder_of(path);
    let used = std::cell::RefCell::new(vec![path.to_string()]);
    // Why a file the model refers to could not be had, for the error (the first reason wins).
    let missing = std::cell::RefCell::new(None::<String>);
    let data = model::parse(bytes, &|f: &str| {
        // A URI is against the model's folder; a texture of the store is `../textures/...`.
        let key = match resolve_uri(folder, f) {
            Ok(inside) => inside,
            Err(why) => {
                *missing.borrow_mut() = Some(why);
                format!("{folder}{f}")
            }
        };
        used.borrow_mut().push(key.clone());
        let found = files.get(&key).cloned();
        if found.is_none() && missing.borrow().is_none() {
            *missing.borrow_mut() = Some(format!(
                "its file `{key}` was not handed to the renderer (the host reads the files a \
                 model refers to from the open project's packs; asset.list shows what they \
                 hold)"
            ));
        }
        found
    });
    let data = match data {
        Ok(data) => data,
        Err(e) => {
            return not_handed(missing.into_inner().unwrap_or(e), used.into_inner());
        }
    };
    let mut out = ResidentModel {
        levels: Vec::new(),
        bounds: (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        size_m: data.lods.size_m,
        read: data.lods.read.clone(),
        meshes: Vec::new(),
        materials: Vec::new(),
        textures: Vec::new(),
        files: used.into_inner(),
        note: None,
    };
    let mut textures: BTreeMap<(usize, bool, Role), Option<vc::TextureId>> = BTreeMap::new();
    let mut notes: Vec<String> = Vec::new();
    let mut texture = |r: &mut vc::Renderer,
                       out: &mut ResidentModel,
                       slot: Option<TexRef>,
                       role: Role|
     -> Option<vc::TextureId> {
        let slot = slot?;
        let key = (slot.image, slot.repeat, role);
        if let Some(id) = textures.get(&key) {
            return *id;
        }
        let made = data
            .images
            .get(slot.image)
            .ok_or_else(|| format!("image {} does not exist", slot.image))
            .and_then(|b| model::decode_image(b))
            .map(|img| match role {
                Role::Orm => model::orm_from_metal_rough(img, false),
                _ => img,
            })
            .and_then(|img| {
                r.add_texture(&vc::TextureDesc {
                    width: img.width,
                    height: img.height,
                    rgba8: &img.pixels,
                    color_space: if role == Role::Base {
                        vc::ColorSpace::Srgb
                    } else {
                        vc::ColorSpace::Linear
                    },
                    repeat: slot.repeat,
                })
            });
        let id = match made {
            Ok(id) => {
                out.textures.push(id);
                Some(id)
            }
            Err(e) => {
                notes.push(format!("image {}: {e}", slot.image));
                None
            }
        };
        textures.insert(key, id);
        id
    };
    // One mesh and one material per prim (a glTF material and the levels that draw it), as
    // before the levels; each level then lists the prims it draws.
    let mut uploaded: Vec<Option<(vc::MeshId, vc::MaterialId, u64)>> = Vec::new();
    let mut fail = None;
    for prim in &data.prims {
        let Some(m) = data.materials.get(prim.material) else {
            uploaded.push(None);
            continue;
        };
        let orm_role = if m.occlusion_image.is_some()
            && m.occlusion_image.map(|t| t.image) == m.metal_rough_image.map(|t| t.image)
        {
            Role::OrmWithOcclusion
        } else {
            Role::Orm
        };
        let maps = MaterialMaps {
            base: texture(r, &mut out, m.base_image, Role::Base),
            normal: texture(r, &mut out, m.normal_image, Role::Normal),
            orm: texture(r, &mut out, m.metal_rough_image, orm_role),
            occlusion: if orm_role == Role::OrmWithOcclusion {
                None
            } else {
                texture(r, &mut out, m.occlusion_image, Role::Occlusion)
            },
        };
        let material = r.add_material(&model_material(m, maps));
        out.materials.push(material);
        let mesh = r.add_mesh(&vc::Mesh {
            positions: &prim.positions,
            normals: &prim.normals,
            indices: &prim.indices,
            face_ids: None,
            uvs: Some(&prim.uvs),
            wind: None,
        });
        match mesh {
            Ok(mesh) => {
                out.meshes.push(mesh);
                // The box of the mesh as the renderer holds it (its vertices), for the count.
                for p in &prim.positions {
                    let p = Vec3::from(*p);
                    out.bounds.0 = out.bounds.0.min(p);
                    out.bounds.1 = out.bounds.1.max(p);
                }
                uploaded.push(Some((mesh, material, (prim.indices.len() / 3) as u64)));
            }
            Err(e) => {
                fail = Some(format!("a mesh could not be uploaded: {e}"));
                break;
            }
        }
    }
    if let Some(why) = fail {
        let files = out.files.clone();
        out.free(r);
        return Err(FailedModel { why, files });
    }
    out.levels = data
        .lods
        .read
        .drawn
        .iter()
        .map(|&level| {
            let mut parts = Vec::new();
            let mut triangles = 0u64;
            for (part, &mask) in uploaded.iter().zip(&data.lods.drawn_at) {
                if let Some((mesh, material, count)) = part
                    && mask >> level & 1 == 1
                {
                    parts.push((*mesh, *material));
                    triangles += count;
                }
            }
            ResidentLevel { parts, triangles }
        })
        .collect();
    if !notes.is_empty() {
        out.note = Some(format!(
            "some textures could not be read and are left out ({})",
            notes.join("; ")
        ));
    }
    Ok(out)
}

/// The uploaded textures of a model's material.
struct MaterialMaps {
    base: Option<vc::TextureId>,
    normal: Option<vc::TextureId>,
    orm: Option<vc::TextureId>,
    occlusion: Option<vc::TextureId>,
}

/// The renderer material of a glTF material. Ported from `model_material` of VisionCraft's
/// `apps/visioncraft/src/library.rs` (MIT): glass as the renderer's thin glass, a masked
/// double-sided material lit as leaves, blending as blending; here also the normal map's scale,
/// a separate occlusion map, and the emissive colour in cd/m2 ([`GLTF_EMISSIVE_CD_M2`]).
fn model_material(m: &ModelMaterial, maps: MaterialMaps) -> vc::Material {
    let mut out = vc::Material {
        base_color: m.base_color,
        metallic: m.metallic,
        roughness: m.roughness,
        projection: vc::Projection::Uv,
        double_sided: m.double_sided,
        ..Default::default()
    };
    if m.glass {
        // The renderer's glass: `base_color.rgb` is the tint, alpha the opacity.
        let opacity = if m.alpha == Alpha::Blend {
            m.base_color[3]
        } else {
            0.12
        };
        out.base_color[3] = opacity.clamp(0.05, 0.6);
        out.surface = vc::Surface::Glass;
        out.metallic = 0.0;
        out.roughness = m.roughness.min(0.15);
        return out;
    }
    out.base_color_map = maps.base;
    out.normal_map = maps.normal;
    out.relief = m.normal_scale;
    out.orm_map = maps.orm;
    out.occlusion_map = maps.occlusion;
    out.emissive = m.emissive;
    out.emissive_strength = m.emissive_strength * GLTF_EMISSIVE_CD_M2;
    match m.alpha {
        Alpha::Opaque => {}
        Alpha::Mask(t) => {
            out.alpha_mode = vc::AlphaMode::Cutout { threshold: t };
            out.foliage = m.double_sided;
        }
        Alpha::Blend => out.alpha_mode = vc::AlphaMode::Blend,
    }
    out
}

/// The mean height of the height field's border samples: the endless ground's height, so the
/// land runs on at the level the terrain ends.
fn border_mean(h: &HeightField) -> f64 {
    let (cols, rows) = (h.cols(), h.rows());
    let (last_col, last_row) = (cols.saturating_sub(1), rows.saturating_sub(1));
    let mut sum = 0.0;
    let mut n = 0usize;
    let mut add = |c: usize, r: usize| {
        if let Some(v) = h.sample(c, r) {
            sum += f64::from(v);
            n += 1;
        }
    };
    for c in 0..cols {
        add(c, 0);
        add(c, last_row);
    }
    for r in 1..last_row {
        add(0, r);
        add(last_col, r);
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// A model-to-world transform: scale, then a turn of `yaw_deg` about +Y, then a move to the
/// base `position`.
fn placement(position: [f64; 3], yaw_deg: f64, scale: [f64; 3]) -> Mat4 {
    glam::DMat4::from_scale_rotation_translation(
        DVec3::from_array(scale),
        DQuat::from_rotation_y(deg_to_rad(yaw_deg)),
        DVec3::from_array(position),
    )
    .as_mat4()
}

/// A downed character's transform: the body, scaled by `scale`, lies on its side, its feet at
/// `position`, its head to the left of where it faced, and its side resting on the ground.
fn lying(position: [f64; 3], yaw_deg: f64, radius_m: f64, scale: f64) -> Mat4 {
    glam::DMat4::from_scale_rotation_translation(
        DVec3::splat(scale),
        DQuat::from_rotation_y(deg_to_rad(yaw_deg)) * DQuat::from_rotation_z(deg_to_rad(LYING_DEG)),
        DVec3::from_array(position) + DVec3::new(0.0, radius_m, 0.0),
    )
    .as_mat4()
}

/// The two line vertices of a segment, as the renderer wants them.
fn line_pair(s: &Segment) -> [vc::LineVertex; 2] {
    let vertex = |p: [f64; 3]| vc::LineVertex {
        position: [p[0] as f32, p[1] as f32, p[2] as f32],
        color: s.rgba,
        width: s.width_px,
    };
    [vertex(s.from), vertex(s.to)]
}

/// One rebuild in progress.
struct Scene<'a> {
    r: &'a mut vc::Renderer,
    built: &'a mut Built,
    problems: &'a mut Vec<String>,
    /// Materials of plain colours made in this build, by the colour's bits.
    colors: BTreeMap<[u32; 3], vc::MaterialId>,
}

impl Scene<'_> {
    fn add_mesh(&mut self, m: &MeshData, what: &str) -> Option<vc::MeshId> {
        match upload_mesh(self.r, m) {
            Ok(id) => {
                self.built.meshes.push(id);
                Some(id)
            }
            Err(e) => {
                self.problems
                    .push(format!("The mesh of {what} could not be uploaded: {e}."));
                None
            }
        }
    }

    fn add_material(&mut self, material: &vc::Material) -> vc::MaterialId {
        let id = self.r.add_material(material);
        self.built.materials.push(id);
        id
    }

    fn add_item(
        &mut self,
        parts: &[(vc::MeshId, vc::MaterialId)],
        instances: &[Mat4],
    ) -> vc::ItemId {
        let lods: Vec<(vc::MeshId, vc::MaterialId, f32)> = parts
            .iter()
            .map(|&(mesh, material)| (mesh, material, f32::INFINITY))
            .collect();
        let id = self.r.add_lod_item(&lods, instances);
        self.built.items.push(id);
        id
    }

    /// The material of a plain linear colour, made once per build.
    fn color(&mut self, color: [f32; 3]) -> vc::MaterialId {
        let key = color.map(f32::to_bits);
        if let Some(&id) = self.colors.get(&key) {
            return id;
        }
        let [r, g, b] = color;
        let id = self.add_material(&vc::Material {
            base_color: [r, g, b, 1.0],
            roughness: SHAPE_ROUGHNESS,
            ..Default::default()
        });
        self.colors.insert(key, id);
        id
    }

    /// The terrain chunks plus the apron that eases their edge down to the ground plane at
    /// `plane_height_m` (scenery, not walkable).
    fn terrain(&mut self, h: &HeightField, plane_height_m: f64, material: vc::MaterialId) {
        let apron = mesh::terrain_apron(h, plane_height_m);
        let chunks = mesh::terrain_chunks(h)
            .into_iter()
            .chain((!apron.mesh.positions.is_empty()).then_some(apron));
        for chunk in chunks {
            let tc = vc::TerrainChunk {
                positions: &chunk.mesh.positions,
                normals: &chunk.mesh.normals,
                indices: &chunk.mesh.indices,
                weights: &chunk.weights,
            };
            match self.r.add_terrain_chunk(&tc) {
                Ok(id) => {
                    self.built.meshes.push(id);
                    self.add_item(&[(id, material)], &[Mat4::IDENTITY]);
                }
                Err(e) => self.problems.push(format!(
                    "The terrain chunk from sample ({}, {}) could not be uploaded and is \
                     missing: {e}.",
                    chunk.first_col, chunk.first_row
                )),
            }
        }
    }

    /// Roads: one item per terrain chunk and colour ([`crate::roads::road_meshes`]).
    fn roads(&mut self, world: &World, field: Option<&HeightField>) {
        let built = crate::roads::road_meshes(world, field);
        self.problems.extend(built.problems);
        let mut materials: BTreeMap<[u32; 3], vc::MaterialId> = BTreeMap::new();
        for chunk in built.chunks {
            let what = format!("the roads of chunk {:?}", chunk.chunk);
            let Some(mesh) = self.add_mesh(&chunk.mesh, &what) else {
                continue;
            };
            let key = chunk.color.map(f32::to_bits);
            let material = match materials.get(&key) {
                Some(&m) => m,
                None => {
                    let [r, g, b] = chunk.color;
                    let m = self.add_material(&vc::Material {
                        base_color: [r, g, b, 1.0],
                        roughness: ROAD_ROUGHNESS,
                        puddles: true,
                        ..Default::default()
                    });
                    materials.insert(key, m);
                    m
                }
            };
            self.add_item(&[(mesh, material)], &[Mat4::IDENTITY]);
        }
    }

    /// Shapes: one item per (kind, colour), an instance per entity.
    fn shapes(&mut self, world: &World, units: UnitMeshes) {
        let mut groups: BTreeMap<(u8, [u32; 3]), (ShapeKind, Vec<Mat4>)> = BTreeMap::new();
        for e in world.entities() {
            let Some(s) = e.shape() else { continue };
            let order = match s.kind {
                ShapeKind::Box => 0,
                ShapeKind::Sphere => 1,
                ShapeKind::Cylinder => 2,
            };
            groups
                .entry((order, s.color.map(f32::to_bits)))
                .or_insert_with(|| (s.kind, Vec::new()))
                .1
                .push(placement(
                    e.position,
                    e.rotation_y_deg,
                    s.size.map(|v| v * e.scale),
                ));
        }
        for ((_, color), (kind, instances)) in groups {
            let material = self.color(color.map(f32::from_bits));
            self.add_item(&[(units.of(kind), material)], &instances);
        }
    }

    /// Models: one upload per asset (kept from one rebuild to the next), one item per asset
    /// and set of distances (the component's; the scale, in buckets, where a distance follows
    /// it) with an instance per entity, its levels of detail and its draw distance from the
    /// file, the component or the asset's size. Characters with a model get an item of their
    /// own per set, whose instances follow them without a rebuild; the character the camera
    /// follows has an item of its own that is never culled by distance. A model that cannot
    /// be drawn is a small magenta box at each entity, with a problem line that names the
    /// asset, why and the entities.
    fn models(
        &mut self,
        world: &World,
        resident: &mut BTreeMap<String, Result<ResidentModel, FailedModel>>,
        files: &BTreeMap<String, Vec<u8>>,
        uploads: &mut u64,
        units: Option<UnitMeshes>,
        followed: Option<EntityId>,
    ) {
        let mut by_asset: BTreeMap<String, Vec<&Entity>> = BTreeMap::new();
        for e in world.entities() {
            if let Some(m) = e.model() {
                by_asset.entry(m.asset.clone()).or_default().push(e);
            }
        }
        let mut missing: Vec<Mat4> = Vec::new();
        let mut missing_characters: Vec<&Entity> = Vec::new();
        for (asset, entities) in by_asset {
            if !resident.contains_key(&asset) {
                let made = upload_model(self.r, &asset, files);
                if made.is_ok() {
                    *uploads += 1;
                }
                resident.insert(asset.clone(), made);
            }
            let Some(Ok(m)) = resident.get(&asset) else {
                let Some(Err(failed)) = resident.get(&asset) else {
                    continue;
                };
                let why = &failed.why;
                let named: Vec<String> = entities
                    .iter()
                    .take(MAX_NAMED_ENTITIES)
                    .map(|e| format!("{} \"{}\"", e.id, e.name))
                    .collect();
                let more = match entities.len().saturating_sub(MAX_NAMED_ENTITIES) {
                    0 => String::new(),
                    n => format!(" and {n} more"),
                };
                self.problems.push(format!(
                    "Model `{asset}` of entity {}{more} cannot be drawn: {why}; it is \
                     drawn as a small magenta box.",
                    named.join(", ")
                ));
                missing.extend(
                    entities
                        .iter()
                        .filter(|e| e.character().is_none())
                        .map(|e| {
                            let s = e.model().map_or(1.0, |m| m.scale) * e.scale;
                            placement(e.position, e.rotation_y_deg, [MISSING_MODEL_SIZE_M * s; 3])
                        }),
                );
                missing_characters
                    .extend(entities.iter().copied().filter(|e| e.character().is_some()));
                continue;
            };
            if let Some(note) = &m.note {
                self.problems.push(format!("Model `{asset}`: {note}"));
            }
            for warning in &m.read.warnings {
                self.problems.push(format!("Model `{asset}`: {warning}"));
            }
            // By what decides the levels, in a map: a thousand scattered copies, each its
            // own size, are a handful of items, found without a search per entity.
            let mut sets: BTreeMap<ModelKey, Vec<&Entity>> = BTreeMap::new();
            for &e in &entities {
                let Some(model) = e.model() else { continue };
                let component = LodOverride {
                    switch_m: model.lod_distances_m.clone(),
                    draw_m: model.draw_distance_m,
                    coverage: None,
                };
                let scale = model.scale * e.scale;
                let key = ModelKey {
                    lod_distances_m: component
                        .switch_m
                        .iter()
                        .flatten()
                        .map(|d| d.to_bits())
                        .collect(),
                    draw_distance_m: component.draw_m.map(f64::to_bits),
                    scale: follows_scale(m.read.levels, m.read.file.as_ref(), Some(&component))
                        .then(|| scale_bucket(scale).to_bits()),
                    followed: Some(e.id) == followed && e.character().is_some(),
                };
                sets.entry(key).or_default().push(e);
            }
            // In the order of the first entity that shows the set, as the entities come.
            let mut sets: Vec<(ModelKey, Vec<&Entity>)> = sets.into_iter().collect();
            sets.sort_by_key(|(_, entities)| entities.first().map(|e| e.id));
            // What the distances say of this asset is said once, whatever the number of sets.
            let mut said: BTreeSet<String> = BTreeSet::new();
            for (key, entities) in sets {
                // The component's distances, else the file's, else the rule from the size the
                // model has in the world (its bucket's upper scale where they follow it).
                let component = key.distances();
                let (distances, notes) = LodDistances::resolve(
                    m.read.levels,
                    m.size_m * key.scale(),
                    m.read.file.as_ref(),
                    Some(&component),
                );
                for note in notes {
                    if said.insert(note.clone()) {
                        self.problems.push(format!("Model `{asset}`: {note}"));
                    }
                }
                // Where each level that draws ends; the last is the draw distance.
                let mut until_m = m.read.drawn_ends(&distances.ends_m());
                if key.followed
                    && let Some(last) = until_m.last_mut()
                {
                    *last = f64::INFINITY;
                }
                let parts: Vec<(vc::MeshId, vc::MaterialId, f32)> = m
                    .levels
                    .iter()
                    .zip(&until_m)
                    .flat_map(|(level, &end)| {
                        level
                            .parts
                            .iter()
                            .map(move |&(mesh, material)| (mesh, material, end as f32))
                    })
                    .collect();
                let (characters, statics): (Vec<&Entity>, Vec<&Entity>) = entities
                    .iter()
                    .copied()
                    .partition(|e| e.character().is_some());
                if !statics.is_empty() {
                    let instances: Vec<Mat4> = statics
                        .iter()
                        .filter_map(|e| {
                            let s = e.model()?.scale * e.scale;
                            Some(placement(e.position, e.rotation_y_deg, [s; 3]))
                        })
                        .collect();
                    self.add_model_item(&asset, &parts, instances, m, until_m.clone());
                }
                if !characters.is_empty() {
                    let body = Body::Model { size: 1.0 };
                    let instances: Vec<Mat4> =
                        characters.iter().map(|e| character_pose(e, body)).collect();
                    let item = self.add_model_item(&asset, &parts, instances.clone(), m, until_m);
                    self.built.characters.push(CharacterGroup {
                        item,
                        ids: characters.iter().map(|e| e.id).collect(),
                        instances,
                        body,
                        model: Some(self.built.models.len().saturating_sub(1)),
                    });
                }
            }
        }
        let Some(units) = units else { return };
        if !missing.is_empty() {
            let material = self.color(MISSING_MODEL_COLOR);
            self.add_item(&[(units.cube, material)], &missing);
        }
        if !missing_characters.is_empty() {
            let material = self.color(MISSING_MODEL_COLOR);
            let body = Body::Model {
                size: MISSING_MODEL_SIZE_M,
            };
            let instances: Vec<Mat4> = missing_characters
                .iter()
                .map(|e| character_pose(e, body))
                .collect();
            let item = self.add_item(&[(units.cube, material)], &instances);
            self.built.characters.push(CharacterGroup {
                item,
                ids: missing_characters.iter().map(|e| e.id).collect(),
                instances,
                body,
                model: None,
            });
        }
    }

    /// Adds the item of a model with its levels (`parts`: a mesh, its material and where its
    /// level ends), and remembers what is needed to count what it draws.
    fn add_model_item(
        &mut self,
        asset: &str,
        parts: &[(vc::MeshId, vc::MaterialId, f32)],
        instances: Vec<Mat4>,
        m: &ResidentModel,
        until_m: Vec<f64>,
    ) -> vc::ItemId {
        let item = self.r.add_lod_item(parts, &instances);
        self.r.set_item_cull_fade(item, MODEL_CULL_FADE);
        self.built.items.push(item);
        self.built.models.push(ModelItem {
            asset: asset.to_string(),
            instances,
            triangles: m.levels.iter().map(|l| l.triangles).collect(),
            drawn: m.read.drawn.clone(),
            bounds: m.bounds,
            until_m,
        });
        item
    }

    /// Characters without a model: one body and visor mesh per distinct size, an instance per
    /// character. A character with a model is drawn by [`Scene::models`].
    fn characters(&mut self, world: &World) {
        // Per size: the body, the characters and their instances.
        type Group = (Character, Vec<EntityId>, Vec<Mat4>);
        let mut groups: BTreeMap<[u64; 3], Group> = BTreeMap::new();
        for e in world.entities() {
            let Some(c) = e.character() else { continue };
            if e.model().is_some() {
                continue;
            }
            let key = [c.height_m, c.radius_m, c.eye_height_m].map(f64::to_bits);
            let (_, ids, instances) = groups
                .entry(key)
                .or_insert_with(|| (c.clone(), Vec::new(), Vec::new()));
            ids.push(e.id);
            instances.push(character_pose(
                e,
                Body::Capsule {
                    radius_m: c.radius_m,
                },
            ));
        }
        if groups.is_empty() {
            return;
        }
        let body_material = self.color(CHARACTER_COLOR);
        let visor_material = self.color(VISOR_COLOR);
        for (character, ids, instances) in groups.into_values() {
            let meshes = mesh::character(&character);
            let body = self.add_mesh(&meshes.body, "a character's body");
            let visor = self.add_mesh(&meshes.visor, "a character's visor");
            if let (Some(body), Some(visor)) = (body, visor) {
                let item = self.add_item(
                    &[(body, body_material), (visor, visor_material)],
                    &instances,
                );
                self.built.characters.push(CharacterGroup {
                    item,
                    ids,
                    instances,
                    body: Body::Capsule {
                        radius_m: character.radius_m,
                    },
                    model: None,
                });
            }
        }
    }

    /// Plants: one generation and one item per distinct (species, seed, growth); textures
    /// once per species.
    fn plants(&mut self, world: &World) {
        let mut groups: BTreeMap<(String, u64, u64), Vec<Mat4>> = BTreeMap::new();
        for e in world.entities() {
            let Some(p) = e.plant() else { continue };
            if !plants::known(&p.species) {
                let nearest = plants::nearest_species(&p.species).map_or_else(String::new, |id| {
                    format!(" The nearest known id is \"{id}\".")
                });
                self.problems.push(format!(
                    "Entity {} has plant species \"{}\", which the plant library does not \
                     know, so it is not drawn.{nearest}",
                    e.id, p.species
                ));
                continue;
            }
            groups
                .entry((p.species.clone(), p.seed, p.growth.to_bits()))
                .or_default()
                .push(placement(e.position, e.rotation_y_deg, [e.scale; 3]));
        }
        let mut textures: BTreeMap<String, Option<PlantTextures>> = BTreeMap::new();
        let mut materials: BTreeMap<(String, PartKind, [u32; 3]), vc::MaterialId> = BTreeMap::new();
        for ((species, seed, growth_bits), instances) in groups {
            let growth = f64::from_bits(growth_bits);
            let Some(plant) = plants::generate(&species, seed, growth) else {
                continue;
            };
            if plant.parts.is_empty() {
                self.problems.push(format!(
                    "Plant species \"{species}\" with seed {seed} and growth {growth} produced \
                     no geometry, so it is not drawn; try a larger growth."
                ));
                continue;
            }
            let tex = match textures.get(&species) {
                Some(t) => *t,
                None => {
                    let t = self.plant_textures(&species);
                    textures.insert(species.clone(), t);
                    t
                }
            };
            let Some(tex) = tex else { continue };
            let mut parts = Vec::new();
            for part in &plant.parts {
                let tint = match part.kind {
                    PartKind::Foliage => plant.foliage_tint,
                    _ => [1.0; 3],
                };
                let key = (species.clone(), part.kind, tint.map(f32::to_bits));
                let material = match materials.get(&key) {
                    Some(&m) => m,
                    None => {
                        let m = self.add_material(&plant_material(part.kind, tex, tint));
                        materials.insert(key, m);
                        m
                    }
                };
                let uploaded = self.r.add_mesh(&vc::Mesh {
                    positions: &part.positions,
                    normals: &part.normals,
                    indices: &part.indices,
                    face_ids: None,
                    uvs: Some(&part.uvs),
                    wind: Some(&part.wind),
                });
                match uploaded {
                    Ok(id) => {
                        self.built.meshes.push(id);
                        parts.push((id, material));
                    }
                    Err(e) => self.problems.push(format!(
                        "A part of plant species \"{species}\" could not be uploaded: {e}."
                    )),
                }
            }
            if !parts.is_empty() {
                self.add_item(&parts, &instances);
            }
        }
    }

    /// Generates and uploads a species' textures; a problem line when that fails.
    fn plant_textures(&mut self, species: &str) -> Option<PlantTextures> {
        let t = plants::textures(species)?;
        let mut upload = |image: &PlantImage, srgb: bool, repeat: bool| {
            let result = self.r.add_texture(&vc::TextureDesc {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba,
                color_space: if srgb {
                    vc::ColorSpace::Srgb
                } else {
                    vc::ColorSpace::Linear
                },
                repeat,
            });
            match result {
                Ok(id) => {
                    self.built.textures.push(id);
                    Ok(id)
                }
                Err(e) => Err(e),
            }
        };
        let made = (|| {
            Ok::<_, String>(PlantTextures {
                atlas: upload(&t.atlas, true, false)?,
                atlas_normal: match &t.atlas_normal {
                    Some(n) => Some(upload(n, false, false)?),
                    None => None,
                },
                bark: upload(&t.bark, true, true)?,
                bark_normal: upload(&t.bark_normal, false, true)?,
            })
        })();
        match made {
            Ok(t) => Some(t),
            Err(e) => {
                self.problems.push(format!(
                    "The textures of plant species \"{species}\" could not be uploaded, so it \
                     is not drawn: {e}."
                ));
                None
            }
        }
    }
}

/// The renderer material of a plant part: bark opaque with its normal map; foliage, flowers
/// and twigs alpha-cut at 0.5 and double-sided, foliage tinted and lit as leaves. Ported from
/// VisionCraft's `apps/visioncraft/src/scene.rs` (`plant_material_desc`).
fn plant_material(kind: PartKind, tex: PlantTextures, tint: [f32; 3]) -> vc::Material {
    let leaf = |color: [f32; 4], foliage: bool| vc::Material {
        base_color: color,
        base_color_map: Some(tex.atlas),
        normal_map: tex.atlas_normal,
        roughness: 0.6,
        alpha_mode: vc::AlphaMode::Cutout { threshold: 0.5 },
        double_sided: true,
        foliage,
        ..Default::default()
    };
    let [r, g, b] = tint;
    match kind {
        PartKind::Bark => vc::Material {
            base_color_map: Some(tex.bark),
            normal_map: Some(tex.bark_normal),
            roughness: 0.9,
            ..Default::default()
        },
        PartKind::Foliage => leaf([r, g, b, 1.0], true),
        PartKind::Flower => leaf([1.0; 4], true),
        PartKind::Twigs => leaf([1.0; 4], false),
    }
}

/// GPU-free tests of what a sync rebuilds and where character items place their characters.
#[cfg(test)]
mod tests {
    use super::*;
    use gamecraft_world::{Component, EntityDraft, ModelCollider};

    fn crate_model(scale: f64) -> Component {
        Component::Model(Model {
            asset: "models/crate.glb".into(),
            scale,
            solid: false,
            collider: ModelCollider::Box,
            lod_distances_m: None,
            draw_distance_m: None,
        })
    }

    fn world_with(components: Vec<Component>) -> (World, EntityId) {
        let mut world = World::new("key", 1);
        let id = world
            .add_entity(EntityDraft {
                name: "Subject".into(),
                scale: 2.0,
                components,
                ..EntityDraft::default()
            })
            .unwrap();
        (world, id)
    }

    /// Walks and turns entity `id` ten steps; whether the first key still matches each time.
    fn still_matches_while_walking(world: &mut World, id: EntityId) -> bool {
        let key = GeometryKey::of(world, None, 0);
        (1..=10).all(|step| {
            let e = world.entity_mut(id).unwrap();
            e.position = [0.3 * f64::from(step), 0.0, -0.1 * f64::from(step)];
            e.rotation_y_deg = 9.0 * f64::from(step);
            key.matches(world, None, 0)
        })
    }

    #[test]
    fn a_character_with_a_model_walks_without_a_rebuild() {
        let (mut world, id) = world_with(vec![
            Component::Character(Character::default()),
            crate_model(1.5),
        ]);
        assert!(
            still_matches_while_walking(&mut world, id),
            "walking and turning keeps the geometry"
        );
        // Another asset or another scale is other geometry.
        let key = GeometryKey::of(&world, None, 0);
        world
            .entity_mut(id)
            .unwrap()
            .set_component(crate_model(2.5));
        assert!(!key.matches(&world, None, 0), "a new model scale rebuilds");
    }

    #[test]
    fn a_character_without_a_model_still_walks_and_a_static_model_still_rebuilds() {
        let (mut world, id) = world_with(vec![Component::Character(Character::default())]);
        assert!(still_matches_while_walking(&mut world, id));
        let (mut world, id) = world_with(vec![crate_model(1.0)]);
        assert!(
            !still_matches_while_walking(&mut world, id),
            "a model without a character is static geometry"
        );
    }

    #[test]
    fn a_character_with_a_model_is_placed_as_its_model_and_falls_on_its_side() {
        let (mut world, id) = world_with(vec![
            Component::Character(Character::default()),
            crate_model(1.5),
        ]);
        let e = world.entity_mut(id).unwrap();
        e.position = [4.0, 1.0, -2.0];
        e.rotation_y_deg = 30.0;
        let e = world.entity(id).unwrap();
        // The model's scale times the entity's, as a model without a character is placed.
        let drawn = character_pose(e, Body::Model { size: 1.0 });
        assert_eq!(drawn, placement([4.0, 1.0, -2.0], 30.0, [3.0; 3]));
        let stand_in = character_pose(
            e,
            Body::Model {
                size: MISSING_MODEL_SIZE_M,
            },
        );
        assert_eq!(
            stand_in,
            placement([4.0, 1.0, -2.0], 30.0, [MISSING_MODEL_SIZE_M * 3.0; 3])
        );
        // The capsule is not scaled, as before.
        let capsule = character_pose(e, Body::Capsule { radius_m: 0.3 });
        assert_eq!(capsule, placement([4.0, 1.0, -2.0], 30.0, [1.0; 3]));
        // Down: lying on its side, scaled, its side on the ground.
        world
            .entity_mut(id)
            .unwrap()
            .set_component(Component::Health(gamecraft_world::Health {
                max: 100.0,
                current: 0.0,
                armor: 0.0,
            }));
        let e = world.entity(id).unwrap();
        let radius = Character::default().radius_m;
        assert_eq!(
            character_pose(e, Body::Model { size: 1.0 }),
            lying([4.0, 1.0, -2.0], 30.0, radius, 3.0)
        );
        let up = character_pose(e, Body::Model { size: 1.0 }).transform_vector3(Vec3::Y);
        assert!(up.y.abs() < 1e-5, "the model's up lies flat: {up}");
    }
}
