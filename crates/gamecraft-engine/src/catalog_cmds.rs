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
];

/// The component with every optional field at its default, as JSON.
fn defaults(kind: &Kind) -> Result<Value, CommandError> {
    let component: Component = serde_json::from_value((kind.minimal)()).map_err(|e| {
        failed(format!(
            "the defaults of the `{}` component could not be read: {e}",
            kind.kind
        ))
    })?;
    serde_json::to_value(component).map_err(failed)
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
