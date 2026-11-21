//! What can be put in a world: `species.list` (the plant library) and `component.list` (the
//! component types with their fields, defaults and an example).

use crate::error::{CommandError, bad, failed};
use crate::example::EXAMPLE_SPECIES;
use crate::params::Params;
use crate::plants::species_list;
use crate::session::{Handler, Registration, Session};
use crate::spec::{CommandSpec, ParamType as T, param, spec};
use gamecraft_world::{Character, Component};
use serde_json::{Value, json};

/// The commands of this module.
pub(crate) const SPECS: &[Registration] = &[
    Registration {
        spec: species_spec,
        handler: Handler::Query(species),
    },
    Registration {
        spec: components_spec,
        handler: Handler::Query(components),
    },
];

/// The categories of the plant library, as `species.list` names them.
const CATEGORIES: [&str; 10] = [
    "tree",
    "conifer",
    "palm",
    "shrub",
    "hedge",
    "perennial",
    "fern",
    "grass",
    "ground_cover",
    "climber",
];

/// Most species one `species.list` lists.
const MAX_SPECIES_LIMIT: u64 = 1000;
/// Species `species.list` lists when `limit` is left out.
const DEFAULT_SPECIES_LIMIT: u64 = 100;

fn species_spec() -> CommandSpec {
    spec(
        "species.list",
        "List Species",
        "Lists the species of the plant library, the ids a `plant` component names, in the \
         library's order; `total` counts all matches, also those beyond `limit`.",
        vec![
            param(
                "filter",
                T::String,
                "Part of the id, the common name or the botanical name, in any case (`oak`).",
            ),
            param("category", T::String, "Only species of this category.").allowed(&CATEGORIES),
            param("limit", T::Integer, "Most species listed, 1 to 1000.").default(json!(100)),
        ],
        "{total, species: [{id, common_name, botanical_name, category, mature_height_m, \
         mature_width_m, evergreen}]}",
        json!({"filter": "oak", "category": "tree"}),
    )
    .query()
}

fn species(_session: &Session, p: Params<'_>) -> Result<Value, CommandError> {
    let filter = p.str("filter")?.map(str::to_lowercase);
    let category = p.str("category")?;
    let limit = p.u64("limit")?.unwrap_or(DEFAULT_SPECIES_LIMIT);
    if !(1..=MAX_SPECIES_LIMIT).contains(&limit) {
        return Err(bad(format!(
            "`limit` is {limit}; it must be from 1 to {MAX_SPECIES_LIMIT}"
        )));
    }
    let matches: Vec<_> = species_list()
        .into_iter()
        .filter(|s| category.is_none_or(|c| s.category == c))
        .filter(|s| {
            filter.as_ref().is_none_or(|f| {
                [&s.id, &s.common_name, &s.botanical_name]
                    .iter()
                    .any(|text| text.to_lowercase().contains(f.as_str()))
            })
        })
        .collect();
    let total = matches.len();
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    let list: Vec<Value> = matches
        .into_iter()
        .take(take)
        .map(|s| {
            json!({
                "id": s.id,
                "common_name": s.common_name,
                "botanical_name": s.botanical_name,
                "category": s.category,
                "mature_height_m": s.mature_height_m,
                "mature_width_m": s.mature_width_m,
                "evergreen": s.evergreen,
            })
        })
        .collect();
    Ok(json!({"total": total, "species": list}))
}

/// One field of a component: name, type as in the spec notation, unit, doc. Its default is
/// read from the component with every field left out.
struct Field {
    name: &'static str,
    ty: &'static str,
    unit: Option<&'static str>,
    doc: &'static str,
}

const fn field(
    name: &'static str,
    ty: &'static str,
    unit: Option<&'static str>,
    doc: &'static str,
) -> Field {
    Field {
        name,
        ty,
        unit,
        doc,
    }
}

/// A component type as `component.list` describes it.
struct Kind {
    kind: &'static str,
    summary: &'static str,
    fields: &'static [Field],
    /// The component with only its required fields given: the others take their defaults.
    minimal: fn() -> Value,
    example: fn() -> Value,
}

const SHAPE_FIELDS: &[Field] = &[
    field(
        "kind",
        "\"box\"|\"sphere\"|\"cylinder\"",
        None,
        "Required. A box (turned with the entity), a sphere, or an upright cylinder.",
    ),
    field(
        "size",
        "[x, y, z]",
        Some("m"),
        "Extent before the entity's scale, 0.001 to 10000 each. A sphere needs all three equal \
         (its diameter); a cylinder needs x == z (its diameter), y is its height.",
    ),
    field(
        "color",
        "[r, g, b]",
        None,
        "Linear RGB, each channel 0 to 1.",
    ),
    field(
        "solid",
        "boolean",
        None,
        "Whether characters collide with it. Rays hit it either way.",
    ),
];

const PLANT_FIELDS: &[Field] = &[
    field(
        "species",
        "string",
        None,
        "Required. A species id of the plant library (see species.list).",
    ),
    field(
        "seed",
        "integer",
        None,
        "Seed of this individual's shape: another seed grows another plant of the species.",
    ),
    field(
        "growth",
        "number",
        None,
        "Growth stage above 0 and at most 1; 1 is mature.",
    ),
];

const CHARACTER_FIELDS: &[Field] = &[
    field("height_m", "number", Some("m"), "Standing height, above 0."),
    field(
        "radius_m",
        "number",
        Some("m"),
        "Capsule radius, above 0 and at most half the height.",
    ),
    field(
        "eye_height_m",
        "number",
        Some("m"),
        "Eye height above the feet, above 0 and at most the height.",
    ),
    field(
        "walk_speed_mps",
        "number",
        Some("mps"),
        "Walking speed, above 0.",
    ),
    field(
        "run_speed_mps",
        "number",
        Some("mps"),
        "Running speed, above 0.",
    ),
    field(
        "jump_speed_mps",
        "number",
        Some("mps"),
        "Upward speed at the start of a jump, above 0.",
    ),
    field(
        "max_slope_deg",
        "number",
        Some("deg"),
        "Steepest walkable slope, 1 to 89.",
    ),
];

const ZONE_FIELDS: &[Field] = &[
    field(
        "shape",
        "\"circle\"|\"box\"",
        None,
        "An upright cylinder, or a box turned with the entity.",
    ),
    field(
        "radius_m",
        "number",
        Some("m"),
        "Radius of a circle (a box ignores it), 0.001 to 10000, times the entity's scale.",
    ),
    field(
        "size",
        "[x, z]",
        Some("m"),
        "Extent of a box (a circle ignores it), 0.001 to 10000 each, times the entity's scale.",
    ),
    field(
        "height_m",
        "number",
        Some("m"),
        "Height above the entity's position, 0.001 to 10000, times the entity's scale. A \
         character whose body overlaps it in height and whose position is in the footprint \
         is inside.",
    ),
];

const INTERACTABLE_FIELDS: &[Field] = &[
    field(
        "reach_m",
        "number",
        Some("m"),
        "How far from a character's eye it can be reached, above 0 and at most 100; measured \
         to the nearest point between its position and the top of its shape.",
    ),
    field(
        "prompt",
        "string",
        None,
        "What a player is told they can do, at most 2000 characters.",
    ),
];

const LIGHT_FIELDS: &[Field] = &[
    field(
        "color",
        "[r, g, b]",
        None,
        "Linear RGB, each channel 0 to 1.",
    ),
    field(
        "intensity_cd",
        "number",
        Some("cd"),
        "Luminous intensity, 0 to 1e7 candela.",
    ),
    field(
        "range_m",
        "number",
        Some("m"),
        "Distance beyond which it lights nothing, above 0 and at most 10000.",
    ),
    field("on", "boolean", None, "Whether it shines."),
];

const TAGS_FIELDS: &[Field] = &[field(
    "tags",
    "[string]",
    None,
    "Free labels (`beacon`, `team:red`), each once, at most 64; matched exactly by \
     \"who\": \"tag:<tag>\" in triggers and by entity.find.",
)];

const MODEL_FIELDS: &[Field] = &[
    field(
        "asset",
        "string",
        None,
        "Required. The glTF asset's path inside the pack's assets/ folder, ending in .glb or \
         .gltf (`models/crate.glb`); asset.list lists them.",
    ),
    field(
        "scale",
        "number",
        None,
        "Uniform scale of the asset on top of the entity's scale, 0.001 to 1000.",
    ),
    field(
        "solid",
        "boolean",
        None,
        "Whether characters are blocked by its collider (the asset's bounding box, turned by \
         the entity's yaw).",
    ),
    field(
        "collider",
        "string",
        None,
        "`box` (the asset's bounding box: rays hit it, and with `solid` characters are \
         blocked) or `none`.",
    ),
];

const HELD_MODEL_FIELDS: &[Field] = &[
    field(
        "asset",
        "string",
        None,
        "Required. The glTF asset's path inside the pack's assets/ folder, ending in .glb or \
         .gltf (`models/rifle.glb`); its forward is -Z, as glTF faces.",
    ),
    field(
        "offset_m",
        "[right, up, forward]",
        Some("m"),
        "Where the model's origin sits from the eye along the aim, each -2 to 2.",
    ),
    field(
        "scale",
        "number",
        None,
        "Uniform scale of the asset, 0.001 to 1000; the entity's scale does not apply.",
    ),
];

const LAUNCHER_FIELDS: &[Field] = &[
    field(
        "muzzle_velocity_mps",
        "number",
        Some("mps"),
        "Speed of the projectile leaving the muzzle, above 0 and at most 3000.",
    ),
    field(
        "mass_kg",
        "number",
        Some("kg"),
        "Mass of one projectile, above 0 and at most 1000.",
    ),
    field(
        "ballistic_coefficient",
        "number",
        Some("kg/m2"),
        "G7 ballistic coefficient in kg/m2 (mass over form factor times pi d^2 / 4): the usual \
         G7 figure in lb/in2 times 895.2. Above 0 and at most 100000.",
    ),
    field(
        "dispersion_mrad",
        "number",
        Some("mrad"),
        "Spread of the shots (radius of a uniform cone), 0 to 500; seeded, so deterministic.",
    ),
    field(
        "rate_s",
        "number",
        Some("s"),
        "Seconds between two shots while firing is held, above 0 and at most 3600.",
    ),
    field(
        "magazine",
        "integer",
        None,
        "Shots before a reload, 1 to 100000.",
    ),
    field(
        "reload_s",
        "number",
        Some("s"),
        "Seconds a reload takes once the magazine is empty, 0 to 3600.",
    ),
    field(
        "damage_per_joule",
        "number",
        None,
        "Damage per joule of kinetic energy at the hit, 0 to 1e6 (a hit takes energy times \
         this, less the target's armor).",
    ),
    field(
        "muzzle_height_m",
        "number",
        Some("m"),
        "Muzzle height above the entity's position, 0 to 10000; a character fires from its \
         eyes instead.",
    ),
    field(
        "max_flight_s",
        "number",
        Some("s"),
        "Longest flight of a projectile, above 0 and at most 600.",
    ),
    field(
        "sound_range_m",
        "number",
        Some("m"),
        "Characters within this distance of the muzzle hear a shot (`heard`), 0 to 100000.",
    ),
];

const HEALTH_FIELDS: &[Field] = &[
    field(
        "max",
        "number",
        None,
        "Full health, above 0 and at most 1e9.",
    ),
    field(
        "current",
        "number",
        None,
        "Health now, 0 to max; at 0 the entity is down (a character stops and takes no input).",
    ),
    field(
        "armor",
        "number",
        None,
        "Damage every hit loses before it counts, 0 to 1e9.",
    ),
];

const BEHAVIOUR_FIELDS: &[Field] = &[
    field(
        "mode",
        "\"idle\"|\"patrol\"|\"guard\"",
        None,
        "What it does when nothing caught its attention: stand, walk its route, or hold its \
         post (where it stood at the start).",
    ),
    field(
        "route",
        "[[x, z], ...]",
        Some("m"),
        "The patrol's points, at most 256; a patrol needs one at least.",
    ),
    field(
        "team",
        "string",
        None,
        "Its side; characters of another non-empty team (a behaviour's team or a tag \
         `team:<name>`) are enemies. Empty: no side.",
    ),
    field(
        "view_range_m",
        "number",
        Some("m"),
        "How far it sees in daylight and clear air, above 0 and at most 10000; night, mist and \
         rain shorten it.",
    ),
    field(
        "fov_deg",
        "number",
        Some("deg"),
        "Field of view, above 0 and at most 360.",
    ),
    field(
        "engage_range_m",
        "number",
        Some("m"),
        "It fires at a seen enemy within this distance and runs towards one farther away, \
         above 0 and at most 10000.",
    ),
    field(
        "aim_error_mrad",
        "number",
        Some("mrad"),
        "Aim error (radius of a uniform cone, drawn per shot), 0 to 1000.",
    ),
    field(
        "reaction_s",
        "number",
        Some("s"),
        "Seconds between first seeing an enemy and firing, 0 to 3600.",
    ),
    field(
        "search_s",
        "number",
        Some("s"),
        "Seconds it searches after losing sight of an enemy or hearing a shot, 0 to 3600.",
    ),
    field(
        "skill",
        "number",
        None,
        "General skill, 0 to 1 (default 1): at 0 its reaction, spot time and cover pauses take \
         twice as long.",
    ),
    field(
        "precision",
        "number",
        None,
        "Aim precision, 0 to 1 (default 1): at 0 its aim errors are four times as large.",
    ),
    field(
        "spot_time_s",
        "number",
        Some("s"),
        "Seconds an enemy in plain view at half the view range takes to be fully known, 0 to \
         3600 (default 0: at first sight); closer, moving or firing sooner. It reacts from \
         knowledge 0.25 and fires from 0.6.",
    ),
    field(
        "aim_settle_s",
        "number",
        Some("s"),
        "Seconds over which the aim error shrinks to aim_error_floor_mrad while the target \
         stays in view, 0 to 3600 (default 0: it does not settle).",
    ),
    field(
        "aim_error_floor_mrad",
        "number",
        Some("mrad"),
        "The aim error of a settled aim, 0 to 1000 (default 0).",
    ),
    field(
        "courage",
        "number",
        None,
        "How much of the stress of near misses it shrugs off, 0 to 1 (default 1: all). Stress \
         widens its aim and, with cover_search_m, sends it into cover.",
    ),
    field(
        "cover_search_m",
        "number",
        Some("m"),
        "How far it looks for cover in combat, 0 to 100 (default 0: it does not take cover).",
    ),
    field(
        "calm_s",
        "number",
        Some("s"),
        "Seconds without contact after which combat calms to aware and aware to safe, 0 to \
         3600 (default 30).",
    ),
    field(
        "alertness",
        "\"safe\"|\"aware\"|\"combat\"",
        None,
        "The alertness it starts in and never falls below (default safe).",
    ),
];

const KINDS: &[Kind] = &[
    Kind {
        kind: "shape",
        summary: "A simple solid standing on the entity's position (its base is the bottom \
                  centre): box, sphere or cylinder.",
        fields: SHAPE_FIELDS,
        minimal: || json!({"type": "shape", "kind": "box"}),
        example: || json!({"type": "shape", "kind": "box", "size": [2, 1, 1], "color": [0.6, 0.4, 0.2], "solid": true}),
    },
    Kind {
        kind: "plant",
        summary: "A plant grown deterministically by the plant library from its species and \
                  seed.",
        fields: PLANT_FIELDS,
        minimal: || json!({"type": "plant", "species": EXAMPLE_SPECIES}),
        example: || json!({"type": "plant", "species": EXAMPLE_SPECIES, "seed": 3, "growth": 0.8}),
    },
    Kind {
        kind: "character",
        summary: "A character: a capsule that walks, runs and jumps, driven by input records \
                  (sim.input) while playing.",
        fields: CHARACTER_FIELDS,
        minimal: || {
            serde_json::to_value(Component::Character(Character::default())).unwrap_or(Value::Null)
        },
        example: || json!({"type": "character", "height_m": 1.7, "walk_speed_mps": 1.4}),
    },
    Kind {
        kind: "zone",
        summary: "A volume standing on the entity's position that produces `enter` and \
                  `leave` events when characters come in and go out, for triggers.",
        fields: ZONE_FIELDS,
        minimal: || json!({"type": "zone"}),
        example: || json!({"type": "zone", "shape": "circle", "radius_m": 4, "height_m": 3}),
    },
    Kind {
        kind: "interactable",
        summary: "Something a character can interact with (sim.input `interact`): the \
                  nearest one within reach that the character faces gets an `interact` event.",
        fields: INTERACTABLE_FIELDS,
        minimal: || json!({"type": "interactable"}),
        example: || json!({"type": "interactable", "reach_m": 2, "prompt": "Open the gate"}),
    },
    Kind {
        kind: "light",
        summary: "A point light at the entity's position (it moves with a character); a \
                  trigger can turn it on and off. It does not change the simulation.",
        fields: LIGHT_FIELDS,
        minimal: || json!({"type": "light"}),
        example: || json!({"type": "light", "color": [1.0, 0.6, 0.2], "intensity_cd": 400, "range_m": 20, "on": false}),
    },
    Kind {
        kind: "tags",
        summary: "Free labels that triggers and entity.find filter by.",
        fields: TAGS_FIELDS,
        minimal: || json!({"type": "tags"}),
        example: || json!({"type": "tags", "tags": ["beacon", "team:red"]}),
    },
    Kind {
        kind: "launcher",
        summary: "Something that fires projectiles (a rifle, a bow, a mortar): on a character \
                  while its input holds `fire`, on anything with sim.fire. Projectiles fly with \
                  gravity, G7 drag, air density and wind, and hit terrain, shapes and characters.",
        fields: LAUNCHER_FIELDS,
        minimal: || json!({"type": "launcher"}),
        example: || json!({"type": "launcher", "muzzle_velocity_mps": 790, "mass_kg": 0.0109, "ballistic_coefficient": 200, "dispersion_mrad": 0.3, "rate_s": 1.2, "magazine": 5, "reload_s": 3}),
    },
    Kind {
        kind: "health",
        summary: "What hits take away; at 0 the entity is down (`down` event) and a character \
                  stops.",
        fields: HEALTH_FIELDS,
        minimal: || json!({"type": "health"}),
        example: || json!({"type": "health", "max": 100, "current": 100, "armor": 5}),
    },
    Kind {
        kind: "behaviour",
        summary: "A small state machine for a character: patrol, guard or idle; on seeing an \
                  enemy it reacts and fires, on hearing a shot it searches, then returns. It \
                  writes the same inputs a player does; sim.input takes it over until \
                  sim.release.",
        fields: BEHAVIOUR_FIELDS,
        minimal: || json!({"type": "behaviour"}),
        example: || json!({"type": "behaviour", "mode": "patrol", "route": [[0, -20], [15, -20]], "team": "red", "reaction_s": 0.8}),
    },
    Kind {
        kind: "model",
        summary: "A glTF model of a pack (`assets/<asset>`), drawn with its PBR materials and \
                  textures, standing on the entity's position. With `solid` its bounding box \
                  (from the asset, turned by the entity's yaw) blocks characters like a box.",
        fields: MODEL_FIELDS,
        minimal: || json!({"type": "model", "asset": "models/crate.glb"}),
        example: || json!({"type": "model", "asset": "models/crate.glb", "scale": 1, "solid": true, "collider": "box"}),
    },
    Kind {
        kind: "held_model",
        summary: "What a character holds as seen from its own eyes (a weapon's view model): a \
                  glTF model of a pack drawn at the first-person camera, pointing where the \
                  character aims (not where a free-looking head turns), only while the \
                  character is seen in first person.",
        fields: HELD_MODEL_FIELDS,
        minimal: || json!({"type": "held_model", "asset": "models/rifle.glb"}),
        example: || json!({"type": "held_model", "asset": "models/rifle.glb", "offset_m": [0.12, -0.14, 0.32], "scale": 1}),
    },
];

/// The component with every optional field at its default, as JSON.
fn defaults(kind: &Kind) -> Result<Value, CommandError> {
    let component: Component = serde_json::from_value((kind.minimal)()).map_err(|e| {
        failed(format!(
            "the defaults of the `{}` component could not be read: {e}",
            kind.kind
        ))
    })?;
    let mut v = serde_json::to_value(&component).map_err(failed)?;
    // A behaviour leaves its skill, detection and cover numbers out of the file at their
    // defaults (older worlds read and write as before); list them anyway.
    if let (Component::Behaviour(b), Value::Object(m)) = (&component, &mut v) {
        for (name, value) in [
            ("skill", json!(b.skill)),
            ("precision", json!(b.precision)),
            ("spot_time_s", json!(b.spot_time_s)),
            ("aim_settle_s", json!(b.aim_settle_s)),
            ("aim_error_floor_mrad", json!(b.aim_error_floor_mrad)),
            ("courage", json!(b.courage)),
            ("cover_search_m", json!(b.cover_search_m)),
            ("calm_s", json!(b.calm_s)),
            ("alertness", json!(b.alertness.name())),
        ] {
            m.entry(name).or_insert(value);
        }
    }
    Ok(v)
}

fn components_spec() -> CommandSpec {
    spec(
        "component.list",
        "List Components",
        "Describes every component type an entity can carry: its fields with their types, \
         units, defaults (null for a required field) and docs, and an example as entity.add \
         and entity.set_component take it.",
        vec![],
        "{components: [{type, summary, fields: [{name, type, unit, default, doc}], example}]}",
        json!({}),
    )
    .query()
}

fn components(_session: &Session, _p: Params<'_>) -> Result<Value, CommandError> {
    let mut list = Vec::new();
    for kind in KINDS {
        let defaults = defaults(kind)?;
        let fields: Vec<Value> = kind
            .fields
            .iter()
            .map(|f| {
                let required = f.doc.starts_with("Required.");
                let default = if required {
                    Value::Null
                } else {
                    defaults.get(f.name).cloned().unwrap_or(Value::Null)
                };
                json!({"name": f.name, "type": f.ty, "unit": f.unit, "default": default, "doc": f.doc})
            })
            .collect();
        list.push(json!({
            "type": kind.kind,
            "summary": kind.summary,
            "fields": fields,
            "example": (kind.example)(),
        }));
    }
    Ok(json!({"components": list}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_fields_are_exactly_the_component_fields() {
        let names: Vec<&str> = KINDS.iter().map(|k| k.kind).collect();
        assert_eq!(names, Component::KINDS);
        for kind in KINDS {
            let d = defaults(kind).unwrap();
            let mut keys: Vec<&str> = d
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "type")
                .collect();
            keys.sort_unstable();
            let mut documented: Vec<&str> = kind.fields.iter().map(|f| f.name).collect();
            documented.sort_unstable();
            assert_eq!(keys, documented, "{}", kind.kind);
            let example: Component = serde_json::from_value((kind.example)()).unwrap();
            assert_eq!(example.kind(), kind.kind);
        }
    }

    #[test]
    fn the_categories_are_the_library_s() {
        for s in species_list() {
            assert!(CATEGORIES.contains(&s.category.as_str()), "{}", s.category);
        }
    }
}
