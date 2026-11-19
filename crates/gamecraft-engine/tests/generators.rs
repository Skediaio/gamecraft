//! Generators through the commands: add, set, remove, list, preview, bake; undo; a running
//! world; and the queries that see the instances.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{example, playing, refuse};
use gamecraft_engine::Session;
use gamecraft_world::{Entity, Instance};
use serde_json::{Value, json};

fn birches() -> Value {
    json!({
        "id": "birches",
        "kind": "forest",
        "area": {"circle": {"center": [25, -25], "radius_m": 25}},
        "seed": 11,
        "density_per_ha": 200,
        "min_distance_m": 2.5,
        "items": [
            {"weight": 2, "components": [{"type": "plant", "species": "betula-pendula"}],
             "growth_range": [0.5, 1.0]},
            {"weight": 1, "components": [{"type": "plant", "species": "lavandula-angustifolia"}]}
        ]
    })
}

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run(command, &params)
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn instances(s: &Session, id: &str) -> Vec<Instance> {
    s.current_world()
        .generated()
        .set(id)
        .map(|set| set.instances().to_vec())
        .unwrap_or_default()
}

#[test]
fn add_places_a_forest_that_undo_takes_away_and_redo_brings_back() {
    let mut s = example();
    let r = run(&mut s, "generator.add", birches());
    let placed = r["placed"].as_u64().unwrap();
    assert!(placed > 20, "{r}");
    assert_eq!(r["requested"], json!(39));
    assert_eq!(instances(&s, "birches").len() as u64, placed);
    // Trunks came from the species: a birch gets one, a lavender none.
    let g = s.document().generator("birches").unwrap();
    let birch = g.items[0].trunk.unwrap();
    assert!(birch.radius_m > 0.1 && birch.height_m > 5.0, "{birch:?}");
    assert_eq!(g.items[1].trunk.unwrap().radius_m, 0.0);
    // The document holds the recipe, not the trees.
    let text = s.document().to_json().unwrap();
    assert!(text.contains("\"birches\"") && !text.contains("birches#"));
    s.undo().unwrap();
    assert!(instances(&s, "birches").is_empty());
    assert!(s.document().generator("birches").is_none());
    s.redo().unwrap();
    assert_eq!(instances(&s, "birches").len() as u64, placed);
    // The preview of the same entry gives the same counts without placing anything.
    let mut fresh = example();
    let p = run(
        &mut fresh,
        "generator.preview",
        json!({"recipe": birches()}),
    );
    assert_eq!(p, r);
    assert!(fresh.document().generator("birches").is_none());
    let p = run(&mut s, "generator.preview", json!({"id": "birches"}));
    assert_eq!(p, r);
}

#[test]
fn set_changes_fields_and_keeps_the_rest_and_remove_takes_it_away() {
    let mut s = example();
    run(&mut s, "generator.add", birches());
    let before = instances(&s, "birches");
    // Keeping a clearing changes only the trees in it.
    let r = run(
        &mut s,
        "generator.set",
        json!({"id": "birches", "exclude": [{"circle": {"center": [25, -25], "radius_m": 8}}]}),
    );
    let after = instances(&s, "birches");
    assert_eq!(r["placed"].as_u64().unwrap() as usize, after.len());
    let kept: Vec<&Instance> = before
        .iter()
        .filter(|i| {
            let (dx, dz) = (i.position[0] - 25.0, i.position[2] + 25.0);
            dx * dx + dz * dz >= 64.0
        })
        .collect();
    assert_eq!(after.iter().collect::<Vec<_>>(), kept);
    assert_eq!(s.document().generator("birches").unwrap().seed, 11);
    // Null clears an optional field.
    run(
        &mut s,
        "generator.set",
        json!({"id": "birches", "exclude": null}),
    );
    assert_eq!(instances(&s, "birches"), before);
    let r = run(&mut s, "generator.remove", json!({"id": "birches"}));
    assert_eq!(r["placed"].as_u64().unwrap() as usize, before.len());
    assert!(s.document().generator("birches").is_none());
    let l = run(&mut s, "generator.list", json!({}));
    assert_eq!(l["generators"].as_array().unwrap().len(), 1, "{l}");
    assert_eq!(l["generators"][0]["id"], json!("pines"));
    assert!(l["generators"][0]["recipe"]["items"].is_array());
}

#[test]
fn bake_turns_a_spot_into_entities_and_the_rest_keeps_its_numbers() {
    let mut s = example();
    run(&mut s, "generator.add", birches());
    let set = s.document().generated().set("birches").unwrap();
    let all: Vec<Entity> = set.instances().iter().map(|i| set.entity(i)).collect();
    let before = s.document().entity_count();
    let spot = ([30.0, -20.0], 10.0);
    let r = run(
        &mut s,
        "generator.bake",
        json!({"id": "birches", "center": spot.0, "radius_m": spot.1, "layer": "grove"}),
    );
    let baked = r["baked"].as_u64().unwrap() as usize;
    assert!(baked > 0, "{r}");
    assert_eq!(s.document().entity_count(), before + baked);
    assert_eq!(r["removed"], json!(false));
    // Baked entities and the instances left are the forest as it was, each once.
    let inside = |e: &Entity| {
        let (dx, dz) = (e.position[0] - spot.0[0], e.position[2] - spot.0[1]);
        dx * dx + dz * dz < spot.1 * spot.1
    };
    let mut now: Vec<Entity> = s
        .document()
        .entities()
        .filter(|e| e.name.starts_with("birches#"))
        .map(|e| {
            assert_eq!(e.layer.as_deref(), Some("grove"));
            Entity {
                id: gamecraft_world::EntityId(0),
                layer: None,
                ..e.clone()
            }
        })
        .collect();
    assert_eq!(now.len(), baked);
    assert!(now.iter().all(inside));
    let left = s.document().generated().set("birches").unwrap();
    now.extend(left.instances().iter().map(|i| left.entity(i)));
    now.sort_by_key(|e| e.name.split('#').nth(1).unwrap().parse::<u32>().unwrap());
    assert_eq!(now, all);
    // Baking the rest removes the generator; undo brings it back.
    let r = run(&mut s, "generator.bake", json!({"id": "birches"}));
    assert_eq!(r["removed"], json!(true));
    assert_eq!(r["baked"].as_u64().unwrap() as usize, all.len() - baked);
    assert!(s.document().generator("birches").is_none());
    s.undo().unwrap();
    assert!(s.document().generator("birches").is_some());
}

#[test]
fn the_game_master_grows_a_wood_in_a_running_world() {
    let mut s = playing();
    run(&mut s, "sim.step", json!({"ticks": 10}));
    let r = run(&mut s, "generator.add", birches());
    assert!(r["placed"].as_u64().unwrap() > 0);
    assert!(!instances(&s, "birches").is_empty());
    assert!(
        s.document().generator("birches").is_none(),
        "the document is untouched"
    );
    run(&mut s, "sim.step", json!({"ticks": 10}));
    assert_eq!(s.live().unwrap().tick(), 20);
    // The running world's queries see the new trees.
    let first = instances(&s, "birches")[0];
    let id = format!("birches#{}", first.number);
    let g = run(&mut s, "entity.get", json!({"id": id}));
    assert_eq!(g["generated"], json!(true));
    assert_eq!(g["position"], json!(first.position));
    run(&mut s, "sim.stop", json!({"keep": true}));
    assert!(s.document().generator("birches").is_some());
    assert_eq!(instances(&s, "birches")[0], first);
}

#[test]
fn queries_see_the_generated_instances() {
    let mut s = example();
    let pines = instances(&s, "pines");
    assert!(pines.len() > 20, "{}", pines.len());
    let tree = pines[0];
    let [x, y, z] = tree.position;
    // entity.find: by name, near a point, and not at all with generated false.
    let f = run(
        &mut s,
        "entity.find",
        json!({"near": [x, z], "radius_m": 0.5, "component": "plant"}),
    );
    assert_eq!(
        f["entities"][0]["id"],
        json!(format!("pines#{}", tree.number))
    );
    assert_eq!(f["entities"][0]["generated"], json!(true));
    let f = run(
        &mut s,
        "entity.find",
        json!({"name": "pines#", "limit": 1000}),
    );
    assert_eq!(f["total"].as_u64().unwrap() as usize, pines.len());
    let f = run(
        &mut s,
        "entity.find",
        json!({"generated": false, "limit": 1000}),
    );
    assert_eq!(
        f["total"].as_u64().unwrap() as usize,
        s.document().entity_count()
    );
    // entity.get of an instance; a wrong number is an error.
    let g = run(
        &mut s,
        "entity.get",
        json!({"id": format!("pines#{}", tree.number)}),
    );
    assert_eq!(g["components"][0]["species"], json!("pinus-sylvestris"));
    assert!(g["trunk"]["radius_m"].as_f64().unwrap() > 0.0);
    refuse(
        &mut s,
        "entity.get",
        json!({"id": "pines#99999"}),
        &["pines#99999"],
    );
    // query.raycast hits the trunk.
    let r = run(
        &mut s,
        "query.raycast",
        json!({"origin": [x, y + 1.0, z + 20.0], "direction": [0, 0, -1], "max_m": 40}),
    );
    assert_eq!(r["kind"], json!("generated"), "{r}");
    assert_eq!(r["generator"], json!("pines"));
    // gm.overview counts them, also near a point.
    let o = run(
        &mut s,
        "gm.overview",
        json!({"near": [-30, 30], "radius_m": 30}),
    );
    assert_eq!(o["generated"][0]["id"], json!("pines"));
    assert_eq!(
        o["generated"][0]["placed"].as_u64().unwrap() as usize,
        pines.len()
    );
    assert_eq!(
        o["generated"][0]["near"].as_u64().unwrap() as usize,
        pines.len()
    );
    let i = run(&mut s, "world.inspect", json!({"entities": false}));
    assert_eq!(i["generated_count"].as_u64().unwrap() as usize, pines.len());
}

#[test]
fn bad_parameters_are_errors_that_name_them() {
    let mut s = example();
    let with = |key: &str, value: Value| {
        let mut p = birches();
        p[key] = value;
        p
    };
    refuse(
        &mut s,
        "generator.add",
        with("kind", json!("jungle")),
        &["kind"],
    );
    refuse(
        &mut s,
        "generator.add",
        with("id", json!("a b")),
        &["generators.id"],
    );
    refuse(
        &mut s,
        "generator.add",
        with("id", json!("pines")),
        &["exists already"],
    );
    refuse(
        &mut s,
        "generator.add",
        with("area", json!({"square": 3})),
        &["square"],
    );
    refuse(
        &mut s,
        "generator.add",
        with("density_per_ha", json!(1e9)),
        &["density_per_ha"],
    );
    refuse(
        &mut s,
        "generator.add",
        with(
            "items",
            json!([{"weight": 1, "components": [{"type": "plant", "species": "oak"}]}]),
        ),
        &["items[0]", "plant library"],
    );
    refuse(
        &mut s,
        "generator.add",
        with(
            "items",
            json!([{"weight": 1, "components": [{"type": "shape", "kind": "box"}]}]),
        ),
        &["items[0]", "plant"],
    );
    refuse(
        &mut s,
        "generator.set",
        json!({"id": "nope", "seed": 1}),
        &["nope", "pines"],
    );
    refuse(
        &mut s,
        "generator.set",
        json!({"id": "pines"}),
        &["at least one field"],
    );
    refuse(
        &mut s,
        "generator.set",
        json!({"id": "pines", "kind": null}),
        &["`kind`"],
    );
    refuse(&mut s, "generator.remove", json!({"id": "nope"}), &["nope"]);
    refuse(
        &mut s,
        "generator.bake",
        json!({"id": "pines", "radius_m": 3}),
        &["`center`"],
    );
    refuse(
        &mut s,
        "generator.bake",
        json!({"id": "pines", "center": [0, 0], "radius_m": -1}),
        &["`radius_m`"],
    );
    refuse(
        &mut s,
        "generator.bake",
        json!({"id": "pines", "layer": "a/b"}),
        &["`layer`"],
    );
    refuse(
        &mut s,
        "generator.preview",
        json!({}),
        &["`id`", "`recipe`"],
    );
}
