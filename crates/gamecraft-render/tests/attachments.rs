//! Attachments on a real GPU: a chain of three models (a base, an adapter on its socket, an
//! optic on the adapter's) is drawn as three items, each in its own colour, where the engine
//! says they land (the composed transforms, with a socket rolled 90 degrees about the length
//! axis: each colour where its model's centre is, read from the pixels); on a walking
//! character the attachments follow without a rebuild; a removed attachment is gone; a model
//! stretched along one axis is drawn stretched, its attachment with it. The picture is written
//! to `out/render-attachments.png`. The GPU tests skip without an adapter (and fail with
//! `GAMECRAFT_REQUIRE_GPU=1`). The models are built in the test: cubes, scaled, moved and
//! turned by their nodes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use gamecraft_render::{Camera, Picture, PictureRequest, WorldRenderer, encode_png, headless_gpu};
use gamecraft_world::attachments::{Attachment, Socket, place};
use gamecraft_world::{Component, EntityDraft, Model, Scale, Terrain, TerrainSource, World};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

static SERIAL: Mutex<()> = Mutex::new(());

fn renderer() -> Option<(MutexGuard<'static, ()>, WorldRenderer)> {
    static GPU: OnceLock<Result<(wgpu::Device, wgpu::Queue), String>> = OnceLock::new();
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match GPU.get_or_init(|| headless_gpu().map_err(|e| e.to_string())) {
        Ok((device, queue)) => Some((guard, WorldRenderer::new(device.clone(), queue.clone()))),
        Err(e) => {
            if std::env::var("GAMECRAFT_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("GAMECRAFT_REQUIRE_GPU=1 but there is no GPU: {e}");
            }
            println!("skipping GPU test: {e}");
            None
        }
    }
}

fn flat() -> (World, gamecraft_world::HeightField) {
    let mut world = World::new("range", 1);
    let terrain = Terrain {
        size_m: 64.0,
        cell_m: 1.0,
        source: TerrainSource::Flat { height_m: 0.0 },
        ..Default::default()
    };
    let hf = terrain.build().unwrap();
    world.terrain = Some(terrain);
    world.environment.time_of_day_h = 14.0;
    (world, hf)
}

/// One node of a test model: a unit cube scaled, moved and turned, in a colour; without a
/// colour it is an empty node (a socket).
struct Part {
    name: &'static str,
    at: [f64; 3],
    size: Option<[f64; 3]>,
    color: Option<[f64; 3]>,
    /// A turn as a quaternion `[x, y, z, w]`.
    rotation: Option<[f64; 4]>,
}

impl Part {
    fn cube(name: &'static str, at: [f64; 3], size: [f64; 3], color: [f64; 3]) -> Part {
        Part {
            name,
            at,
            size: Some(size),
            color: Some(color),
            rotation: None,
        }
    }

    fn socket(name: &'static str, at: [f64; 3], rotation: [f64; 4]) -> Part {
        Part {
            name,
            at,
            size: None,
            color: None,
            rotation: Some(rotation),
        }
    }
}

/// The 36 corners of a box standing on the origin, 1 m on each side and centred on x and z
/// (x and z from -0.5 to 0.5, y from 0 to 1): two triangles per side, counter-clockwise from
/// outside, as a triangle list.
fn cube() -> Vec<[f32; 3]> {
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
        let (c00, c10, c11, c01) = (
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        );
        let quad = if sign == 1.0 {
            [c00, c10, c11, c00, c11, c01]
        } else {
            [c00, c11, c10, c00, c01, c11]
        };
        out.extend(quad);
    }
    out.iter_mut().for_each(|p| {
        p[0] -= 0.5;
        p[2] -= 0.5;
    });
    out
}

/// The GLB of the parts: one mesh and one material per cube (its POSITION data only).
fn glb(parts: &[Part]) -> Vec<u8> {
    let corners = cube();
    let mut bin: Vec<u8> = Vec::new();
    for p in &corners {
        for v in p {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let count = corners.len();
    let mut materials = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for p in parts {
        let mut node = serde_json::json!({"name": p.name, "translation": p.at});
        if let Some(r) = p.rotation {
            node["rotation"] = serde_json::json!(r);
        }
        if let (Some(size), Some(color)) = (p.size, p.color) {
            node["scale"] = serde_json::json!(size);
            let material = materials.len();
            materials.push(serde_json::json!({
                "name": format!("m{material}"),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [color[0], color[1], color[2], 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.9,
                },
            }));
            meshes.push(serde_json::json!({"primitives": [{
                "attributes": {"POSITION": 0}, "material": material}]}));
            node["mesh"] = serde_json::json!(meshes.len() - 1);
        }
        nodes.push(node);
    }
    let doc = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": (0..nodes.len()).collect::<Vec<_>>()}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                       "min": [-0.5, 0.0, -0.5], "max": [0.5, 1.0, 0.5]}],
    });
    gamecraft_project::textures::pack_glb(&serde_json::to_vec(&doc).unwrap(), Some(&bin)).unwrap()
}

const RED: [f64; 3] = [0.8, 0.05, 0.05];
const GREEN: [f64; 3] = [0.05, 0.8, 0.05];
const BLUE: [f64; 3] = [0.05, 0.05, 0.8];
/// A turn of 90 degrees about +Z.
const ROLL_Z: [f64; 4] = [
    0.0,
    0.0,
    std::f64::consts::FRAC_1_SQRT_2,
    std::f64::consts::FRAC_1_SQRT_2,
];
const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// A red post 0.4 wide and 1 m tall with `socket_top` on top, rolled 90 degrees about z.
fn base() -> Vec<u8> {
    glb(&[
        Part::cube("Post", [0.0; 3], [0.4, 1.0, 0.4], RED),
        Part::socket("socket_top", [0.0, 1.0, 0.0], ROLL_Z),
    ])
}

/// A green bar 0.2 x 0.2 x 0.6 standing on its socket's origin (outwards along its +Y, along
/// its +Z), with `socket_tip` on its far upper corner; in the base's frame the roll turns the
/// bar sideways, to -X.
fn adapter() -> Vec<u8> {
    glb(&[
        Part::cube("Bar", [-0.1, 0.0, -0.3], [0.2, 0.2, 0.6], GREEN),
        Part::socket("socket_tip", [0.0, 0.2, 0.3], IDENTITY),
    ])
}

/// A blue cube of 0.15 m sitting on the tip.
fn optic() -> Vec<u8> {
    glb(&[Part::cube(
        "Cube",
        [-0.075, 0.0, -0.075],
        [0.15, 0.15, 0.15],
        BLUE,
    )])
}

/// The sockets the host measures in the three models.
fn sockets() -> BTreeMap<String, BTreeMap<String, Socket>> {
    [
        (
            "models/base.glb".to_string(),
            [(
                "top".to_string(),
                Socket {
                    position: [0.0, 1.0, 0.0],
                    rotation: ROLL_Z,
                },
            )]
            .into_iter()
            .collect(),
        ),
        (
            "models/adapter.glb".to_string(),
            [(
                "tip".to_string(),
                Socket {
                    position: [0.0, 0.2, 0.3],
                    rotation: IDENTITY,
                },
            )]
            .into_iter()
            .collect(),
        ),
    ]
    .into_iter()
    .collect()
}

/// The model component with the chain: the adapter on `top`, the optic on the adapter's `tip`.
fn mounted() -> Model {
    Model {
        attachments: vec![Attachment {
            attachments: vec![Attachment::new("tip", "models/optic.glb")],
            ..Attachment::new("top", "models/adapter.glb")
        }],
        ..Model::of("models/base.glb")
    }
}

fn world_with_chain(at: [f64; 3], character: bool) -> (World, gamecraft_world::HeightField) {
    let (mut world, hf) = flat();
    world.set_model_sockets(sockets());
    let mut components = vec![Component::Model(mounted())];
    if character {
        components.push(Component::Character(Default::default()));
    }
    world
        .add_entity(EntityDraft {
            name: if character { "Walker" } else { "Rig" }.into(),
            position: at,
            components,
            ..EntityDraft::default()
        })
        .unwrap();
    (world, hf)
}

fn provide(r: &mut WorldRenderer) {
    r.provide_asset("models/base.glb", base());
    r.provide_asset("models/adapter.glb", adapter());
    r.provide_asset("models/optic.glb", optic());
}

const CAMERA: Camera = Camera {
    position: [2.2, 1.8, 2.8],
    target: [-0.15, 1.0, 0.0],
    fov_y_deg: 50.0,
};

fn picture(r: &mut WorldRenderer, camera: Camera) -> Picture {
    r.render(&PictureRequest {
        width: 320,
        height: 240,
        camera,
        time_s: 0.0,
    })
    .unwrap()
}

/// The mean position of the pixels of a colour, in pixels.
fn centroid(p: &Picture, of: fn([f64; 3]) -> bool) -> Option<(f64, f64)> {
    let w = p.width as usize;
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, px) in p.rgba.as_chunks::<4>().0.iter().enumerate() {
        let c = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2])];
        if of(c) {
            sx += (i % w) as f64;
            sy += (i / w) as f64;
            n += 1.0;
        }
    }
    (n > 0.0).then_some((sx / n, sy / n))
}

fn is_red(c: [f64; 3]) -> bool {
    c[0] > 1.8 * c[1] + 25.0 && c[0] > 1.8 * c[2] + 25.0
}

/// The models' green, sunlit or shaded; the grass (117, 134, 78) is yellowish next to it
/// (a sunlit face reads 107, 185, 81).
fn is_green(c: [f64; 3]) -> bool {
    c[1] > 1.4 * c[0] + 20.0 && c[1] > 1.2 * c[2]
}

fn is_blue(c: [f64; 3]) -> bool {
    c[2] > 1.8 * c[0] + 20.0 && c[2] > 1.3 * c[1]
}

#[test]
fn a_chain_is_drawn_as_three_items_where_the_engine_says() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    provide(&mut r);
    let (world, hf) = world_with_chain([0.0; 3], false);
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    // One item per model of the chain.
    let items = r.item_count();
    let p = picture(&mut r, CAMERA);
    // Where the engine says each model lands.
    let entity = world.entities().next().unwrap();
    let model = entity.model().unwrap();
    let sockets = world.all_model_sockets();
    let bar = place(entity, model, sockets, &["top".to_string()]).unwrap();
    let cube = place(
        entity,
        model,
        sockets,
        &["top".to_string(), "tip".to_string()],
    )
    .unwrap();
    for k in 0..3 {
        assert!(
            (bar.position[k] - [0.0, 1.0, 0.0][k]).abs() < 1e-9,
            "{bar:?}"
        );
        assert!(
            (cube.position[k] - [-0.2, 1.0, 0.3][k]).abs() < 1e-9,
            "{cube:?}"
        );
    }
    // The three colours are in the picture, each where the engine's placement puts it: the
    // green bar above the red post (it sits on the post's top), the blue cube beside the bar
    // at the same height (the socket's roll turned the adapter's up to the base's side), and
    // not on top of it.
    let red = centroid(&p, is_red).expect("the red post is in the picture");
    let green = centroid(&p, is_green).expect("the green bar is in the picture");
    let blue = centroid(&p, is_blue).expect("the blue cube is in the picture");
    assert!(
        green.1 < red.1 - 15.0,
        "the bar is above the post: {red:?} {green:?}"
    );
    assert!(
        (blue.1 - green.1).abs() < 30.0 && (blue.0 - green.0).abs() > 15.0,
        "the cube is beside the bar, at its height: {green:?} {blue:?}"
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("render-attachments.png"),
        encode_png(&p.rgba, p.width, p.height).unwrap(),
    )
    .unwrap();
    // Without the optic: one item fewer.
    let mut bare = Model::of("models/base.glb");
    bare.attachments = vec![Attachment::new("top", "models/adapter.glb")];
    let mut world2 = world.clone();
    world2
        .entity_mut(entity.id)
        .unwrap()
        .set_component(Component::Model(bare));
    r.sync(&world2, Some(&hf));
    assert_eq!(r.item_count(), items - 1);
    let p2 = picture(&mut r, CAMERA);
    assert!(centroid(&p2, is_blue).is_none(), "the optic is gone");
    assert!(centroid(&p2, is_green).is_some(), "the bar stays");
}

#[test]
fn a_walker_s_attachments_follow_it_without_a_rebuild() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    provide(&mut r);
    let (mut world, hf) = world_with_chain([4.0, 0.0, 4.0], true);
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let camera = Camera {
        position: [6.0, 1.8, 7.0],
        target: [4.0, 1.0, 4.0],
        fov_y_deg: 50.0,
    };
    let rebuilds = r.rebuild_count();
    let id = world.entities().next().unwrap().id;
    let before = centroid(&picture(&mut r, camera), is_blue).expect("the optic is in the picture");
    for step in 1..=6 {
        let e = world.entity_mut(id).unwrap();
        e.position = [4.0 - 0.3 * f64::from(step), 0.0, 4.0];
        e.rotation_y_deg = 5.0 * f64::from(step);
        r.sync(&world, Some(&hf));
    }
    assert_eq!(r.rebuild_count(), rebuilds, "walking is no rebuild");
    // The engine says where the bar landed (the socket on the walker, turned 30 degrees), and
    // the picture shows the optic moved with it, without a rebuild.
    let entity = world.entity(id).unwrap();
    let bar = place(
        entity,
        entity.model().unwrap(),
        world.all_model_sockets(),
        &["top".to_string()],
    )
    .unwrap();
    assert!(
        (bar.position[0] - 2.2).abs() < 1e-9 && (bar.position[2] - 4.0).abs() < 1e-9,
        "{bar:?}"
    );
    let after = centroid(&picture(&mut r, camera), is_blue).expect("the optic is still drawn");
    assert!(
        (after.0 - before.0).abs() > 20.0 || (after.1 - before.1).abs() > 20.0,
        "the attachments followed the walker: {before:?} -> {after:?}"
    );
}

/// The extent of the pixels of a colour between the columns `columns` (pixel x, from, to):
/// left, right, top and bottom, in pixels.
fn extent(
    p: &Picture,
    of: fn([f64; 3]) -> bool,
    columns: std::ops::Range<usize>,
) -> Option<(f64, f64, f64, f64)> {
    let w = p.width as usize;
    let mut found: Option<(f64, f64, f64, f64)> = None;
    for (i, px) in p.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        if !columns.contains(&x) {
            continue;
        }
        let c = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2])];
        if !of(c) {
            continue;
        }
        let (x, y) = (x as f64, y as f64);
        found = Some(match found {
            None => (x, x, y, y),
            Some((l, r, t, b)) => (l.min(x), r.max(x), t.min(y), b.max(y)),
        });
    }
    found
}

/// A red post 1 m on each side with `socket_side` on its right face, unturned.
fn post() -> Vec<u8> {
    glb(&[
        Part::cube("Post", [0.0; 3], [1.0; 3], RED),
        Part::socket("socket_side", [0.5, 0.0, 0.0], IDENTITY),
    ])
}

/// A green slab 0.5 m wide, 1 m tall and as deep as the post, standing on its socket's
/// origin: its front is the post's front.
fn slab() -> Vec<u8> {
    glb(&[Part::cube("Slab", [0.0; 3], [0.5, 1.0, 1.0], GREEN)])
}

#[test]
fn a_model_stretched_along_one_axis_is_drawn_stretched_its_attachment_with_it() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    r.provide_asset("models/post.glb", post());
    r.provide_asset("models/slab.glb", slab());
    // Two rigs with the same mount, 16 m apart: one as the models are, the other stretched to
    // twice its width along the entity's x.
    let (mut world, hf) = flat();
    world.set_model_sockets(
        [(
            "models/post.glb".to_string(),
            [(
                "side".to_string(),
                Socket {
                    position: [0.5, 0.0, 0.0],
                    rotation: IDENTITY,
                },
            )]
            .into_iter()
            .collect(),
        )]
        .into_iter()
        .collect(),
    );
    for (name, x, scale) in [("Plain", -8.0, [1.0; 3]), ("Wide", 8.0, [2.0, 1.0, 1.0])] {
        let mut model = Model::of("models/post.glb");
        model.attachments = vec![Attachment::new("side", "models/slab.glb")];
        world
            .add_entity(EntityDraft {
                name: name.into(),
                position: [x, 0.0, 0.0],
                scale: Scale::per_axis(scale[0], scale[1], scale[2]),
                components: vec![Component::Model(model)],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    // A picture straight in front of each rig, from the same distance: no side face shows,
    // so the widths are the models' own.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    let mut shots = Vec::new();
    for (name, x) in [("plain", -8.0), ("wide", 8.0)] {
        let p = picture(
            &mut r,
            Camera {
                position: [x, 0.5, 10.0],
                target: [x, 0.5, 0.0],
                fov_y_deg: 50.0,
            },
        );
        std::fs::write(
            dir.join(format!("render-attachments-stretched-{name}.png")),
            encode_png(&p.rgba, p.width, p.height).unwrap(),
        )
        .unwrap();
        shots.push(p);
    }
    let width = |e: (f64, f64, f64, f64)| e.1 - e.0 + 1.0;
    let (plain, wide) = (
        extent(&shots[0], is_red, 0..320).expect("the plain post is in the picture"),
        extent(&shots[1], is_red, 0..320).expect("the wide post is in the picture"),
    );
    // The post is twice as wide stretched, and as tall as before.
    assert!(
        width(wide) > 1.7 * width(plain),
        "the wide post is twice as wide: {plain:?} {wide:?}"
    );
    assert!(
        (wide.2 - plain.2).abs() <= 1.5 && (wide.3 - plain.3).abs() <= 2.5,
        "and as tall: {plain:?} {wide:?}"
    );
    // The slab on its right face is stretched with the rig, and as tall as on the plain one.
    // The engine says where: the socket moved out with the stretch.
    let entity = world.entities().nth(1).unwrap();
    let placed = place(
        entity,
        entity.model().unwrap(),
        world.all_model_sockets(),
        &["side".to_string()],
    )
    .unwrap();
    assert!((placed.position[0] - 9.0).abs() < 1e-9, "{placed:?}");
    let (plain, wide) = (
        extent(&shots[0], is_green, 0..320).expect("the plain slab is in the picture"),
        extent(&shots[1], is_green, 0..320).expect("the wide slab is in the picture"),
    );
    assert!(
        width(wide) > 1.7 * width(plain),
        "the slab on the wide rig is twice as wide: {plain:?} {wide:?}"
    );
    assert!(
        (wide.2 - plain.2).abs() <= 1.5 && (wide.3 - plain.3).abs() <= 2.5,
        "and as tall: {plain:?} {wide:?}"
    );
}

/// A green slab 4 m on each side standing on its socket's origin: large enough that the rule
/// alone would draw it from a kilometre away.
fn slab_big() -> Vec<u8> {
    glb(&[Part::cube("Slab", [0.0; 3], [4.0; 3], GREEN)])
}

/// A tiny blue cube of 0.1 m with `socket_side`: the rule alone stops drawing it at some
/// twenty metres.
fn tiny() -> Vec<u8> {
    glb(&[
        Part::cube("Cube", [-0.05, 0.0, -0.05], [0.1, 0.1, 0.1], BLUE),
        Part::socket("socket_side", [0.0, 0.05, 0.0], IDENTITY),
    ])
}

/// The stand-in's magenta in sunlight (208, 104, 200): red and blue high, green well under.
fn is_magenta(c: [f64; 3]) -> bool {
    c[0] > 150.0 && c[2] > 150.0 && c[1] < 140.0 && c[0] > 1.5 * c[1] && c[2] > 1.5 * c[1]
}

/// The pixels per metre at `distance` m straight ahead of a camera of 50 degrees over 240
/// pixels.
fn px_per_m(distance: f64) -> f64 {
    120.0 / (distance * (25.0f64).to_radians().tan())
}

#[test]
fn a_missing_attachment_s_stand_in_sits_at_its_socket_not_near_the_origin() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    provide(&mut r);
    let (mut world, hf) = flat();
    world.set_model_sockets(sockets());
    // A rig far from the origin whose mount's model was never given: a stand-in on its
    // socket `top`, a metre up.
    let mut model = Model::of("models/base.glb");
    model.attachments = vec![Attachment::new("top", "models/absent.glb")];
    world
        .add_entity(EntityDraft {
            name: "Rig".into(),
            position: [500.0, 0.0, 500.0],
            components: vec![Component::Model(model)],
            ..EntityDraft::default()
        })
        .unwrap();
    // The one problem line: the model of the mount, drawn as a stand-in.
    let problems = r.sync(&world, Some(&hf));
    assert!(
        problems.len() == 1
            && problems[0].contains("models/absent.glb")
            && problems[0].contains("magenta"),
        "{problems:?}"
    );
    let looking = |at: [f64; 3]| Camera {
        position: [at[0], at[1], at[2] + 10.0],
        target: at,
        fov_y_deg: 50.0,
    };
    let near = picture(&mut r, looking([500.0, 0.6, 500.0]));
    let far = picture(&mut r, looking([0.0, 0.6, 0.0]));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, p) in [("near", &near), ("far", &far)] {
        std::fs::write(
            dir.join(format!("render-attachments-stand-in-{name}.png")),
            encode_png(&p.rgba, p.width, p.height).unwrap(),
        )
        .unwrap();
    }
    let found = centroid(&near, is_magenta).expect("the stand-in is at the socket, by the rig");
    assert!(
        centroid(&far, is_magenta).is_none(),
        "nothing stands in for it near the origin"
    );
    // At 10 m the socket is where the camera looks, 0.4 m above its height. The stand-in is
    // the cube of half a metre in the socket's frame, which `top` rolls 90 degrees about z:
    // it lies to the left of the socket and is centred on its height. Its centre is a
    // quarter of a metre to the left of the middle, and 0.4 m above it.
    assert!(
        (found.0 - (160.0 - 0.25 * px_per_m(10.0))).abs() < 3.0,
        "{found:?}"
    );
    assert!(
        (found.1 - (120.0 - 0.4 * px_per_m(10.0))).abs() < 3.0,
        "{found:?}"
    );
}

#[test]
fn an_attachment_is_never_drawn_beyond_where_its_root_s_model_stops_drawing() {
    let Some((_gpu, mut r)) = renderer() else {
        return;
    };
    provide(&mut r);
    r.provide_asset("models/slab.glb", slab_big());
    r.provide_asset("models/tiny.glb", tiny());
    let (mut world, hf) = flat();
    let mut sockets = sockets();
    sockets.insert(
        "models/tiny.glb".to_string(),
        [(
            "side".to_string(),
            Socket {
                position: [0.0, 0.05, 0.0],
                rotation: IDENTITY,
            },
        )]
        .into_iter()
        .collect(),
    );
    world.set_model_sockets(sockets);
    // Left: a post that stops at 40 m (its component says so) with a slab on its socket.
    // Right: a cube of 0.1 m, which the rule stops at some twenty metres, with a slab on it.
    // The slabs alone would be drawn from a kilometre away.
    let mut post = Model {
        draw_distance_m: Some(40.0),
        ..Model::of("models/base.glb")
    };
    post.attachments = vec![Attachment::new("top", "models/slab.glb")];
    let mut cube = Model::of("models/tiny.glb");
    cube.attachments = vec![Attachment::new("side", "models/slab.glb")];
    for (name, x, model) in [("Post", -5.0, post), ("Cube", 5.0, cube)] {
        world
            .add_entity(EntityDraft {
                name: name.into(),
                position: [x, 0.0, 0.0],
                components: vec![Component::Model(model)],
                ..EntityDraft::default()
            })
            .unwrap();
    }
    assert_eq!(r.sync(&world, Some(&hf)), Vec::<String>::new());
    let looking_from = |z: f64| Camera {
        position: [0.0, 3.0, z],
        target: [0.0, 3.0, 0.0],
        fov_y_deg: 50.0,
    };
    // A hundred metres away: the roots are not drawn, and neither is what sits on them.
    let far = picture(&mut r, looking_from(100.0));
    assert!(centroid(&far, is_red).is_none(), "the post ends at 40 m");
    assert!(
        centroid(&far, is_green).is_none(),
        "the slabs end where their roots do"
    );
    // Fourteen metres away: both draw.
    let near = picture(&mut r, looking_from(14.0));
    assert!(centroid(&near, is_green).is_some(), "near, the slabs draw");
}
