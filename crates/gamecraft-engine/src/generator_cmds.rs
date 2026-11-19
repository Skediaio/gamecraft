//! Generators (`docs/design/generators.md`): `generator.add`, `generator.set`,
//! `generator.remove`, `generator.list`, `generator.preview` and `generator.bake`.
//!
//! A generator is one entry of the world document (a forest of tens of thousands of trees, a
//! scatter of rocks): an area, a seed, a density and the items it places. What it places is
//! derived ([`gamecraft_world::Generated`]), never stored: the instances are called
//! `<id>#<n>`, the simulation and the queries see them, and `generator.bake` turns those of a
//! spot into ordinary entities to edit by hand. The changing commands are World commands: one
//! undo step each, and they work on a running world (the game master grows a wood in the fog).

use crate::error::{CommandError, bad, failed};
use crate::params::Params;
use crate::session::{Edit, Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, excerpt, param, spec};
use gamecraft_world::generator::MAX_GENERATORS;
use gamecraft_world::{
    Area, Circle, Component, EntityDraft, Generated, GeneratedSet, Generator, GeneratorKind, Trunk,
    World,
};
use serde_json::{Map, Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: add_spec,
        handler: Handler::World(add),
    },
    Registration {
        spec: set_spec,
        handler: Handler::World(set),
    },
    Registration {
        spec: remove_spec,
        handler: Handler::World(remove),
    },
    Registration {
        spec: list_spec,
        handler: Handler::Query(list),
    },
    Registration {
        spec: preview_spec,
        handler: Handler::Query(preview),
    },
    Registration {
        spec: bake_spec,
        handler: Handler::World(bake),
    },
];

/// Most instances one `generator.bake` turns into entities.
pub const MAX_BAKE: usize = 20_000;

/// The fields of a generator entry, in the file's order, after `id`.
const RECIPE_FIELDS: [&str; 9] = [
    "kind",
    "area",
    "seed",
    "density_per_ha",
    "min_distance_m",
    "slope_max_deg",
    "height_range_m",
    "exclude",
    "items",
];

const AREA_DOC: &str = "Where instances stand: {\"circle\": {\"center\": [x, z], \"radius_m\": \
    r}} or {\"polygon\": [[x, z], ...]} (3 to 1024 corners, metres).";

const ITEMS_DOC: &str = "What it places, 1 to 64 items: [{\"weight\": w (its share), \
    \"components\": [...] (as in entity.add) | \"prefab\": id, \"scale_range\": [min, max] \
    (default [1, 1]), \"random_rotation\": true, \"growth_range\": [min, max] (a plant's growth, \
    in (0, 1], cut into 3 classes), \"variants\": 3 (plant seeds the instances share, 1 to 16), \
    \"trunk\": {\"radius_m\", \"height_m\"} (what blocks; a forest item without one gets its \
    species' trunk)}]. Every item of a forest has a `plant`.";

const SUMMARY_DOC: &str = "{id, kind, requested (density times area), candidates (after the \
    spacing), placed (after the terrain and the excludes), per_item: [counts], bounds: null | \
    {min: [x, z], max: [x, z]}, note? (why nothing stands)}";

fn recipe_params(required: bool) -> Vec<crate::spec::ParamSpec> {
    let req = |p: crate::spec::ParamSpec| if required { p.required() } else { p };
    vec![
        param(
            "id",
            T::String,
            "The generator's id: 1 to 64 ASCII letters, digits, `-` and `_`; its instances are \
             `<id>#<n>`.",
        )
        .required(),
        req(param(
            "kind",
            T::String,
            "`forest` (plants, with trunks that block) or `scatter` (anything).",
        )
        .allowed(&["forest", "scatter"])),
        req(param("area", T::Object, AREA_DOC)),
        param(
            "seed",
            T::Integer,
            "Seed of the placement; the world's seed when left out.",
        ),
        req(param(
            "density_per_ha",
            T::Number,
            "Instances per hectare (10 000 m2) of the area, above 0, at most 10 000; one \
             generator places at most 250 000.",
        )),
        param(
            "min_distance_m",
            T::Number,
            "Least horizontal distance between two instances, 0 to 1000.",
        )
        .unit("m"),
        param(
            "slope_max_deg",
            T::Number,
            "Steepest ground an instance stands on, 0 to 90.",
        )
        .unit("deg"),
        param(
            "height_range_m",
            T::Array,
            "[min, max] of the ground height an instance stands on.",
        )
        .unit("m"),
        param(
            "exclude",
            T::Array,
            "Areas kept clear (roads, clearings, camps), each as `area`, at most 256.",
        ),
        req(param("items", T::Array, ITEMS_DOC)),
    ]
}

fn add_spec() -> CommandSpec {
    spec(
        "generator.add",
        "Add Generator",
        "Adds a generator: a forest or a scatter as one entry of the world (an area, a seed, a \
         density and the items it places), whose instances are derived, never stored. The \
         same entry on the same terrain gives the same instances in the same order. Forest \
         items without a `trunk` get their species' (from the plant library), written into \
         the entry. One undo step; works on a running world. Needs a terrain for anything to \
         stand.",
        recipe_params(true),
        SUMMARY_DOC,
        json!({
            "id": "oakwood",
            "kind": "forest",
            "area": {"circle": {"center": [20, -20], "radius_m": 30}},
            "seed": 5,
            "density_per_ha": 150,
            "min_distance_m": 3,
            "slope_max_deg": 30,
            "items": [
                {"weight": 3, "components": [{"type": "plant", "species": "quercus-robur"}],
                 "growth_range": [0.6, 1.0]},
                {"weight": 1, "components": [{"type": "plant", "species": "betula-pendula"}]}
            ]
        }),
    )
}

fn set_spec() -> CommandSpec {
    spec(
        "generator.set",
        "Set Generator",
        "Changes a generator: the fields given replace its own (null clears an optional one), \
         the rest stay. A changed area, seed, density, spacing or items places it anew; \
         changed excludes or ground limits keep every other instance with its number. One undo \
         step; works on a running world.",
        recipe_params(false),
        SUMMARY_DOC,
        json!({"id": "pines", "density_per_ha": 80}),
    )
}

fn remove_spec() -> CommandSpec {
    spec(
        "generator.remove",
        "Remove Generator",
        "Removes a generator and with it everything it placed (entities baked from it stay). \
         One undo step; works on a running world.",
        vec![param("id", T::String, "The generator's id.").required()],
        "{removed: id, placed (how many instances went with it)}",
        json!({"id": "pines"}),
    )
}

fn list_spec() -> CommandSpec {
    spec(
        "generator.list",
        "List Generators",
        "Every generator of the current world (the running world while playing), in order, \
         with its entry and what it placed.",
        vec![
            param(
                "recipes",
                T::Boolean,
                "Include each generator's full entry; false gives the counts only.",
            )
            .default(json!(true)),
        ],
        &format!("{{generators: [{SUMMARY_DOC} + recipe?: {{the entry}}], total_placed}}"),
        json!({"recipes": false}),
    )
    .query()
}

fn preview_spec() -> CommandSpec {
    spec(
        "generator.preview",
        "Preview Generator",
        "Counts and bounds of what a generator places, without placing it: an existing one by \
         `id`, or an entry not yet added (`recipe`, the parameters of generator.add) on the \
         current terrain. Changes nothing.",
        vec![
            param("id", T::String, "An existing generator."),
            param(
                "recipe",
                T::Object,
                "An entry as the parameters of generator.add, previewed on the current world.",
            ),
        ],
        SUMMARY_DOC,
        json!({"id": "pines"}),
    )
    .query()
}

fn bake_spec() -> CommandSpec {
    spec(
        "generator.bake",
        "Bake Generator",
        "Turns a generator's instances into ordinary entities, for a spot to edit by hand: \
         those inside the circle `center`, `radius_m` (and the circle joins the generator's \
         `exclude`, so the rest keep their numbers), or all of them (and the generator is \
         removed). An entity is named like its instance (`<id>#<n>`) and has its place, turn, \
         scale and components; an instance of a prefab item stays an instance of the prefab \
         (without its children). At most 20 000 at once. One undo step; works on a running \
         world.",
        vec![
            param("id", T::String, "The generator's id.").required(),
            param(
                "center",
                T::Point2,
                "Centre [x, z] of the spot, with `radius_m`; all instances when left out.",
            )
            .unit("m"),
            param("radius_m", T::Number, "Radius of the spot, above 0.").unit("m"),
            param(
                "layer",
                T::String,
                "The layer the entities go to (a plain name); the world's default layer when \
                 left out.",
            ),
        ],
        "{baked, first_id (null when none), last_id, removed (true when the generator went \
         with it), generator: summary | null}",
        json!({"id": "pines", "center": [-30, 30], "radius_m": 10}),
    )
}

/// The generator of the world with `id`, or an error listing the ids.
fn existing<'w>(world: &'w World, id: &str) -> Result<&'w Generator, CommandError> {
    world.generator(id).ok_or_else(|| {
        let known: Vec<&str> = world
            .generators
            .iter()
            .take(20)
            .map(|g| g.id.as_str())
            .collect();
        failed(format!(
            "no generator has the id \"{}\"; {}",
            excerpt(id),
            if known.is_empty() {
                "the world has none (add one with generator.add)".to_string()
            } else {
                format!("the generators are: {}", known.join(", "))
            }
        ))
    })
}

/// The entry of the JSON object `map` (the parameters of generator.add), checked: it reads,
/// its plants are in the library, forest trunks are filled in from the species, and it keeps
/// the rules of a generator in `world`.
fn recipe_of(
    world: &World,
    map: &Map<String, Value>,
    default_seed: u64,
) -> Result<Generator, CommandError> {
    let mut map = map.clone();
    map.retain(|k, v| !v.is_null() && (k == "id" || RECIPE_FIELDS.contains(&k.as_str())));
    map.entry("seed").or_insert(json!(default_seed));
    let mut g: Generator = serde_json::from_value(Value::Object(map))
        .map_err(|e| bad(format!("the generator entry does not read: {e}")))?;
    for (i, item) in g.items.iter().enumerate() {
        for c in &item.components {
            if let Component::Plant(p) = c
                && !crate::plants::species_exists(&p.species)
            {
                let hint = crate::plants::nearest_species(&p.species)
                    .map(|n| format!("; the nearest is \"{n}\""))
                    .unwrap_or_default();
                return Err(bad(format!(
                    "`items[{i}]`: plant species {:?} is not in the plant library{hint}",
                    excerpt(&p.species)
                )));
            }
        }
    }
    if g.kind == GeneratorKind::Forest {
        let prefabs = world.prefabs.clone();
        for item in &mut g.items {
            if item.trunk.is_some() {
                continue;
            }
            let species = item
                .base_components(&prefabs)
                .into_iter()
                .find_map(|c| match c {
                    Component::Plant(p) => Some(p.species),
                    _ => None,
                });
            let (radius_m, height_m) = species
                .as_deref()
                .and_then(crate::plants::trunk_of)
                .unwrap_or((0.0, 1.0));
            item.trunk = Some(Trunk { radius_m, height_m });
        }
    }
    g.validate(&world.prefabs).map_err(|e| bad(e.to_string()))?;
    Ok(g)
}

/// The summary of what `set` placed.
fn summary(world: &World, set: &GeneratedSet) -> Value {
    let g = set.generator();
    let mut v = Map::new();
    v.insert("id".into(), json!(g.id));
    v.insert("kind".into(), json!(g.kind.name()));
    v.insert("requested".into(), json!(g.requested()));
    v.insert("candidates".into(), json!(set.candidate_count()));
    v.insert("placed".into(), json!(set.instances().len()));
    v.insert("per_item".into(), json!(set.per_item()));
    v.insert(
        "bounds".into(),
        set.footprint()
            .map_or(Value::Null, |f| json!({"min": f.min, "max": f.max})),
    );
    if world.terrain.is_none() {
        v.insert(
            "note".into(),
            json!("the world has no terrain, so nothing stands; add one with terrain.set"),
        );
    } else if set.instances().is_empty() && set.candidate_count() > 0 {
        v.insert(
            "note".into(),
            json!(
                "every candidate was turned away: outside the terrain, in an exclude, too \
                 steep or outside height_range_m"
            ),
        );
    }
    Value::Object(v)
}

/// The summary of generator `id` of the edited world, derived again.
fn edited_summary(edit: &mut Edit<'_>, id: &str) -> Result<Value, CommandError> {
    edit.regenerate();
    let set = edit
        .world
        .generated()
        .set(id)
        .ok_or_else(|| failed(format!("generator \"{id}\" placed nothing")))?;
    Ok(summary(&edit.world, set))
}

fn checked_id(p: Params<'_>) -> Result<String, CommandError> {
    let id = p.str("id")?.ok_or_else(|| bad("missing `id`"))?;
    Ok(id.to_string())
}

fn add(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = checked_id(p)?;
    if edit.world.generator(&id).is_some() {
        return Err(failed(format!(
            "a generator \"{}\" exists already; change it with generator.set or pick another id",
            excerpt(&id)
        )));
    }
    if edit.world.generators.len() >= MAX_GENERATORS {
        return Err(failed(format!(
            "the world holds {MAX_GENERATORS} generators, the most it can; remove one first"
        )));
    }
    let map = params_map(p);
    let g = recipe_of(&edit.world, &map, edit.world.seed)?;
    edit.world.generators.push(g);
    edit.world.validate().map_err(|e| bad(e.to_string()))?;
    edited_summary(edit, &id)
}

/// The parameters as a JSON object.
fn params_map(p: Params<'_>) -> Map<String, Value> {
    let mut map = Map::new();
    for key in std::iter::once("id").chain(RECIPE_FIELDS) {
        if let Some(v) = p.value(key) {
            map.insert(key.to_string(), v.clone());
        }
    }
    map
}

fn set(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = checked_id(p)?;
    let old = existing(&edit.world, &id)?.clone();
    let Value::Object(mut map) = serde_json::to_value(&old).map_err(failed)? else {
        return Err(failed("the generator entry is not an object"));
    };
    let mut changed = false;
    for key in RECIPE_FIELDS {
        if p.is_null(key) {
            if matches!(key, "kind" | "area" | "density_per_ha" | "items") {
                return Err(bad(format!(
                    "`{key}` cannot be cleared; every generator has one"
                )));
            }
            map.remove(key);
            changed = true;
        } else if let Some(v) = p.value(key) {
            map.insert(key.to_string(), v.clone());
            changed = true;
        }
    }
    if !changed {
        return Err(bad(format!(
            "give at least one field to change: {}",
            RECIPE_FIELDS.join(", ")
        )));
    }
    // A forest item whose trunk was filled in keeps it; a new item gets its species'.
    let g = recipe_of(&edit.world, &map, old.seed)?;
    let Some(slot) = edit.world.generators.iter_mut().find(|g| g.id == id) else {
        return Err(failed(format!("generator \"{id}\" went away")));
    };
    *slot = g;
    edit.world.validate().map_err(|e| bad(e.to_string()))?;
    edited_summary(edit, &id)
}

fn remove(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = checked_id(p)?;
    existing(&edit.world, &id)?;
    let placed = edit
        .world
        .generated()
        .set(&id)
        .map_or(0, |s| s.instances().len());
    edit.world.generators.retain(|g| g.id != id);
    Ok(json!({"removed": id, "placed": placed}))
}

fn list(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let recipes = p.bool("recipes")?.unwrap_or(true);
    let world = session.current_world();
    let mut out = Vec::new();
    for set in world.generated().sets() {
        let mut v = summary(world, set);
        if recipes && let Value::Object(m) = &mut v {
            m.insert(
                "recipe".into(),
                serde_json::to_value(set.generator()).map_err(failed)?,
            );
        }
        out.push(v);
    }
    Ok(json!({"generators": out, "total_placed": world.generated().count()}))
}

fn preview(session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let world = session.current_world();
    match (p.str("id")?, p.value("recipe")) {
        (Some(id), None) => {
            existing(world, id)?;
            let set = world
                .generated()
                .set(id)
                .ok_or_else(|| failed(format!("generator \"{id}\" has not been placed yet")))?;
            Ok(summary(world, set))
        }
        (None, Some(Value::Object(map))) => {
            let g = recipe_of(world, map, world.seed)?;
            let derived = Generated::default().derive(
                std::slice::from_ref(&g),
                &world.prefabs,
                world.terrain.as_ref(),
                session.current_heights(),
                world.all_model_bounds(),
            );
            let set = derived
                .set(&g.id)
                .ok_or_else(|| failed("the entry placed nothing"))?;
            Ok(summary(world, set))
        }
        (None, Some(_)) => Err(bad(
            "`recipe` must be an object, the parameters of generator.add",
        )),
        (Some(_), Some(_)) => Err(bad("give `id` or `recipe`, not both")),
        (None, None) => Err(bad(
            "give `id` (an existing generator) or `recipe` (an entry to try)",
        )),
    }
}

fn bake(edit: &mut Edit<'_>, p: Params<'_>) -> Result<Value, CommandError> {
    let id = checked_id(p)?;
    existing(&edit.world, &id)?;
    let spot = match (p.point2("center")?, p.f64("radius_m")?) {
        (Some(center), Some(radius_m)) => {
            if radius_m.is_nan() || radius_m <= 0.0 {
                return Err(bad(format!(
                    "`radius_m` is {radius_m}; it must be above 0 metres"
                )));
            }
            Some(Circle { center, radius_m })
        }
        (None, None) => None,
        (Some(_), None) => return Err(bad("`center` needs `radius_m`, the spot's radius")),
        (None, Some(_)) => return Err(bad("`radius_m` needs `center` [x, z]")),
    };
    let layer = p.str("layer")?.map(str::to_string);
    if let Some(l) = &layer
        && !gamecraft_world::is_layer_name(l)
    {
        return Err(bad(format!(
            "`layer` is {:?}; a layer is 1 to 64 ASCII letters, digits, `-` and `_`",
            excerpt(l)
        )));
    }
    edit.regenerate();
    let area = spot.clone().map(Area::Circle);
    let drafts: Vec<(EntityDraft, Option<String>)> = {
        let set = edit
            .world
            .generated()
            .set(&id)
            .ok_or_else(|| failed(format!("generator \"{id}\" placed nothing")))?;
        let chosen: Vec<_> = set
            .instances()
            .iter()
            .filter(|i| {
                area.as_ref()
                    .is_none_or(|a| a.contains([i.position[0], i.position[2]]))
            })
            .collect();
        if chosen.len() > MAX_BAKE {
            return Err(failed(format!(
                "{} instances lie in the spot; one generator.bake turns at most {MAX_BAKE} into \
                 entities: bake a smaller circle",
                chosen.len()
            )));
        }
        chosen
            .into_iter()
            .map(|i| {
                let e = set.entity(i);
                let prefab = set
                    .generator()
                    .items
                    .get(usize::from(i.item))
                    .and_then(|item| item.prefab.clone());
                (
                    EntityDraft {
                        name: e.name,
                        position: e.position,
                        rotation_y_deg: e.rotation_y_deg,
                        scale: e.scale,
                        components: e.components,
                        layer: layer.clone(),
                    },
                    prefab,
                )
            })
            .collect()
    };
    let (mut first, mut last) = (None, None);
    let baked = drafts.len();
    for (draft, prefab) in drafts {
        let eid = edit
            .world
            .add_entity(draft)
            .map_err(|e| failed(e.to_string()))?;
        if let Some(prefab) = prefab
            && let Some(e) = edit.world.entity_mut(eid)
        {
            e.prefab = Some(prefab);
        }
        first.get_or_insert(eid.0);
        last = Some(eid.0);
    }
    let removed = spot.is_none();
    match spot {
        None => edit.world.generators.retain(|g| g.id != id),
        Some(circle) => {
            if let Some(g) = edit.world.generators.iter_mut().find(|g| g.id == id) {
                g.exclude.push(Area::Circle(circle));
            }
        }
    }
    edit.world.validate().map_err(|e| bad(e.to_string()))?;
    let generator = if removed {
        Value::Null
    } else {
        edited_summary(edit, &id)?
    };
    Ok(json!({
        "baked": baked,
        "first_id": first,
        "last_id": last,
        "removed": removed,
        "generator": generator,
    }))
}
