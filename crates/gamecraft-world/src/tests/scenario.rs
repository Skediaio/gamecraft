//! Scenario data: the components `zone`, `interactable`, `light` and `tags`, and the world's
//! `vars`, `objectives` and `triggers`.

use super::{EXAMPLE_PRETTY, full_world};
use crate::{
    CompareOp, Component, Condition, EntityDraft, EntityId, EntityRef, Interactable, Light,
    Objective, ObjectiveStatus, Step, Tags, Trigger, VarValue, When, World, Zone, ZoneShape,
};
use std::collections::BTreeMap;

/// A small scenario: a beacon with every new component, a variable, an objective and a
/// trigger with a condition.
fn scenario_world() -> World {
    let mut w = World::new("Beacons", 3);
    w.add_entity(EntityDraft {
        name: "Beacon".into(),
        position: [1.0, 0.0, -2.0],
        components: vec![
            Component::Zone(Zone {
                shape: ZoneShape::Circle,
                radius_m: 3.0,
                size: [4.0, 4.0],
                height_m: 3.0,
            }),
            Component::Interactable(Interactable {
                reach_m: 2.0,
                prompt: "Light the beacon".into(),
            }),
            Component::Light(Light {
                color: [1.0, 0.7, 0.4],
                intensity_cd: 200.0,
                range_m: 12.0,
                on: false,
            }),
            Component::Tags(Tags {
                tags: vec!["beacon".into()],
            }),
        ],
        ..EntityDraft::default()
    })
    .unwrap();
    w.vars.insert("lit".into(), VarValue::Number(0.0));
    w.vars.insert("alarm".into(), VarValue::Bool(false));
    w.objectives.push(Objective {
        id: "light-all".into(),
        title: "Light every beacon".into(),
        status: ObjectiveStatus::Open,
        hidden: false,
    });
    w.triggers.push(Trigger {
        id: "light".into(),
        when: When::Interact {
            target: Some(EntityRef("Beacon".into())),
            who: Some(EntityRef("tag:player".into())),
        },
        condition: Some(Condition::Compare {
            var: "lit".into(),
            op: CompareOp::Lt,
            value: VarValue::Number(3.0),
        }),
        actions: vec![Step {
            command: "var.set".into(),
            params: serde_json::json!({"name": "lit", "add": 1}),
        }],
        once: true,
        enabled: true,
    });
    w
}

const SCENARIO_PRETTY_TAIL: &str = r#"  "vars": {
    "alarm": false,
    "lit": 0.0
  },
  "objectives": [
    {
      "id": "light-all",
      "title": "Light every beacon",
      "status": "open",
      "hidden": false
    }
  ],
  "triggers": [
    {
      "id": "light",
      "when": {
        "event": "interact",
        "target": "Beacon",
        "who": "tag:player"
      },
      "if": {
        "var": "lit",
        "op": "<",
        "value": 3.0
      },
      "do": [
        {
          "command": "var.set",
          "params": {
            "add": 1,
            "name": "lit"
          }
        }
      ],
      "once": true,
      "enabled": true
    }
  ]
}"#;

#[test]
fn a_scenario_round_trips_and_prints_in_a_stable_order() {
    let w = scenario_world();
    w.validate().unwrap();
    let text = w.to_json().unwrap();
    assert!(text.ends_with(SCENARIO_PRETTY_TAIL), "{text}");
    let back = World::from_json(&text).unwrap();
    assert_eq!(back, w);
    assert_eq!(back.to_json().unwrap(), text);
}

#[test]
fn a_document_without_scenario_data_is_still_version_1_and_prints_unchanged() {
    let w = World::from_json(EXAMPLE_PRETTY).unwrap();
    assert!(w.vars.is_empty() && w.objectives.is_empty() && w.triggers.is_empty());
    let text = w.to_json().unwrap();
    assert_eq!(text, EXAMPLE_PRETTY);
    for key in ["vars", "objectives", "triggers"] {
        assert!(!text.contains(key), "{key}");
    }
}

#[test]
fn the_new_components_take_their_defaults() {
    let w = World::from_json(
        r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [
            {"type": "zone"}, {"type": "interactable"}, {"type": "light"}, {"type": "tags"}]}]}"#,
    )
    .unwrap();
    let e = w.entity(EntityId(1)).unwrap();
    assert_eq!(
        e.zone(),
        Some(&Zone {
            shape: ZoneShape::Circle,
            radius_m: 2.0,
            size: [4.0, 4.0],
            height_m: 3.0
        })
    );
    assert_eq!(
        e.interactable(),
        Some(&Interactable {
            reach_m: 2.0,
            prompt: String::new()
        })
    );
    let light = e.light().unwrap();
    assert!(light.on);
    assert_eq!((light.intensity_cd, light.range_m), (100.0, 10.0));
    assert!(e.tags().is_empty());
    assert!(!e.has_tag("x"));
    assert_eq!(
        Component::KINDS[..7],
        [
            "shape",
            "plant",
            "character",
            "zone",
            "interactable",
            "light",
            "tags"
        ]
    );
}

#[test]
fn tags_are_matched_exactly() {
    let w = scenario_world();
    let e = w.entity(EntityId(1)).unwrap();
    assert!(e.has_tag("beacon"));
    assert!(!e.has_tag("Beacon") && !e.has_tag("beac"));
    assert_eq!(e.tags(), ["beacon".to_string()]);
}

#[test]
fn entity_references_accept_an_id_or_a_name() {
    let w = World::from_json(
        r#"{"gamecraft": 1, "triggers": [
            {"id": "a", "when": {"event": "enter", "zone": 4}, "do": []},
            {"id": "b", "when": {"event": "leave", "zone": "Gate", "who": 7}, "do": []}]}"#,
    )
    .unwrap();
    assert_eq!(
        w.triggers[0].when,
        When::Enter {
            zone: EntityRef("4".into()),
            who: None
        }
    );
    assert_eq!(
        w.triggers[1].when,
        When::Leave {
            zone: EntityRef("Gate".into()),
            who: Some(EntityRef("7".into()))
        }
    );
    // Defaults: once and enabled.
    assert!(w.triggers[0].once && w.triggers[0].enabled);
    let e = err(
        r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "enter", "zone": [1]}, "do": []}]}"#,
    );
    assert!(e.contains("id or name"), "{e}");
}

#[test]
fn every_event_and_condition_form_reads_and_writes() {
    let text = r#"{"gamecraft": 1, "vars": {"n": 2, "s": "x", "b": true}, "triggers": [
        {"id": "t1", "when": {"event": "time", "at_s": 120}, "do": [{"command": "environment.set", "params": {"mist": 0.8}}]},
        {"id": "t2", "when": {"event": "var", "name": "n"}, "if": {"all": [{"var": "n", "op": ">=", "value": 3}, {"any": [{"var": "s", "op": "==", "value": "x"}, {"var": "b", "op": "!=", "value": false}]}]}, "do": [{"command": "message.show"}], "once": false},
        {"id": "t3", "when": {"event": "start"}, "do": [], "enabled": false},
        {"id": "t4", "when": {"event": "interact"}, "do": []}
    ]}"#;
    let w = World::from_json(text).unwrap();
    assert_eq!(w.triggers.len(), 4);
    assert_eq!(w.triggers[0].when, When::Time { at_s: 120.0 });
    assert_eq!(w.triggers[2].when, When::Start);
    assert_eq!(
        w.triggers[3].when,
        When::Interact {
            target: None,
            who: None
        }
    );
    assert!(matches!(&w.triggers[1].condition, Some(Condition::All(items)) if items.len() == 2));
    assert_eq!(w.vars.get("s"), Some(&VarValue::Text("x".into())));
    let back = World::from_json(&w.to_json().unwrap()).unwrap();
    assert_eq!(back, w);
}

fn err(text: &str) -> String {
    match World::from_json(text) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn unknown_fields_and_bad_shapes_in_scenario_data_are_refused() {
    let cases = [
        (
            r#"{"gamecraft": 1, "objectives": [{"id": "a", "title": "A", "colour": 1}]}"#,
            "colour",
        ),
        (
            r#"{"gamecraft": 1, "objectives": [{"id": "a", "title": "A", "status": "won"}]}"#,
            "won",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "do": [], "then": 1}]}"#,
            "then",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "explode"}, "do": []}]}"#,
            "explode",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "time", "at_s": 1, "zone": "x"}, "do": []}]}"#,
            "zone",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "do": [{"command": "x", "args": {}}]}]}"#,
            "args",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "if": {"var": "x", "op": "=", "value": 1}, "do": []}]}"#,
            "op",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "if": {"var": "x", "op": "==", "value": 1, "extra": 2}, "do": []}]}"#,
            "extra",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "if": {"none": []}, "do": []}]}"#,
            "all",
        ),
        (
            r#"{"gamecraft": 1, "triggers": [{"id": "a", "when": {"event": "start"}, "if": {"var": "x", "op": "==", "value": [1]}, "do": []}]}"#,
            "value",
        ),
        (r#"{"gamecraft": 1, "vars": {"x": [1]}}"#, "x"),
        (r#"{"gamecraft": 1, "vars": {"x": null}}"#, "x"),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "zone", "shape": "ring"}]}]}"#,
            "ring",
        ),
        (
            r#"{"gamecraft": 1, "entities": [{"id": 1, "components": [{"type": "tags", "tags": "a"}]}]}"#,
            "sequence",
        ),
    ];
    for (text, needle) in cases {
        let e = err(text);
        assert!(e.contains(needle), "{text}\n -> {e}");
    }
}

type Mutation = fn(&mut World);

fn beacon(w: &mut World) -> &mut crate::Entity {
    w.entity_mut(EntityId(1)).unwrap()
}

fn zone(w: &mut World) -> &mut Zone {
    match beacon(w).components.get_mut(0) {
        Some(Component::Zone(z)) => z,
        _ => panic!("no zone"),
    }
}

fn interactable(w: &mut World) -> &mut Interactable {
    match beacon(w).components.get_mut(1) {
        Some(Component::Interactable(i)) => i,
        _ => panic!("no interactable"),
    }
}

fn light(w: &mut World) -> &mut Light {
    match beacon(w).components.get_mut(2) {
        Some(Component::Light(l)) => l,
        _ => panic!("no light"),
    }
}

fn tags(w: &mut World) -> &mut Vec<String> {
    match beacon(w).components.get_mut(3) {
        Some(Component::Tags(t)) => &mut t.tags,
        _ => panic!("no tags"),
    }
}

fn nested(depth: usize) -> Condition {
    let mut c = Condition::Compare {
        var: "lit".into(),
        op: CompareOp::Eq,
        value: VarValue::Number(1.0),
    };
    for _ in 0..depth {
        c = Condition::All(vec![c]);
    }
    c
}

/// (description, mutation, texts the error must contain)
fn cases() -> Vec<(&'static str, Mutation, &'static [&'static str])> {
    vec![
        (
            "zone radius",
            |w| zone(w).radius_m = 0.0,
            &["entity 1", "zone.radius_m"],
        ),
        (
            "zone size",
            |w| zone(w).size[1] = f64::NAN,
            &["entity 1", "zone.size[1]"],
        ),
        (
            "zone height",
            |w| zone(w).height_m = -1.0,
            &["entity 1", "zone.height_m"],
        ),
        (
            "reach",
            |w| interactable(w).reach_m = 0.0,
            &["entity 1", "interactable.reach_m"],
        ),
        (
            "reach large",
            |w| interactable(w).reach_m = 1000.0,
            &["entity 1", "interactable.reach_m", "100"],
        ),
        (
            "prompt",
            |w| interactable(w).prompt = "p".repeat(3000),
            &["entity 1", "interactable.prompt", "3000 characters"],
        ),
        (
            "light colour",
            |w| light(w).color[0] = 2.0,
            &["entity 1", "light.color[0]"],
        ),
        (
            "light intensity",
            |w| light(w).intensity_cd = -1.0,
            &["entity 1", "light.intensity_cd"],
        ),
        (
            "light range",
            |w| light(w).range_m = 0.0,
            &["entity 1", "light.range_m"],
        ),
        (
            "empty tag",
            |w| tags(w).push(" ".into()),
            &["entity 1", "tags.tags[1]", "empty"],
        ),
        (
            "duplicate tag",
            |w| tags(w).push("beacon".into()),
            &["entity 1", "tags.tags[1]", "beacon", "more than once"],
        ),
        (
            "too many tags",
            |w| *tags(w) = (0..100).map(|i| format!("t{i}")).collect(),
            &["entity 1", "tags.tags", "64"],
        ),
        (
            "empty var name",
            |w| {
                w.vars.insert(String::new(), VarValue::Bool(true));
            },
            &["vars", "empty"],
        ),
        (
            "var not finite",
            |w| {
                w.vars.insert("lit".into(), VarValue::Number(f64::INFINITY));
            },
            &["vars.lit", "finite"],
        ),
        (
            "var text long",
            |w| {
                w.vars.insert("s".into(), VarValue::Text("x".repeat(3000)));
            },
            &["vars.s", "3000 characters"],
        ),
        (
            "objective id empty",
            |w| w.objectives[0].id = String::new(),
            &["objectives[0].id", "empty"],
        ),
        (
            "objective id twice",
            |w| {
                let again = w.objectives[0].clone();
                w.objectives.push(again);
            },
            &["objectives[1].id", "light-all", "more than once"],
        ),
        (
            "objective title long",
            |w| w.objectives[0].title = "t".repeat(3000),
            &["objectives[0].title"],
        ),
        (
            "trigger id twice",
            |w| {
                let again = w.triggers[0].clone();
                w.triggers.push(again);
            },
            &["triggers[1].id", "light", "more than once"],
        ),
        (
            "trigger time negative",
            |w| w.triggers[0].when = When::Time { at_s: -1.0 },
            &["triggers[0] (\"light\")", "when.at_s", "-1"],
        ),
        (
            "trigger zone empty",
            |w| {
                w.triggers[0].when = When::Enter {
                    zone: EntityRef(String::new()),
                    who: None,
                }
            },
            &["when.zone", "empty"],
        ),
        (
            "trigger tag empty",
            |w| {
                w.triggers[0].when = When::Enter {
                    zone: EntityRef("Beacon".into()),
                    who: Some(EntityRef("tag:".into())),
                }
            },
            &["when.who", "tag:"],
        ),
        (
            "trigger var name empty",
            |w| {
                w.triggers[0].when = When::Var {
                    name: String::new(),
                }
            },
            &["when.name", "empty"],
        ),
        (
            "condition too deep",
            |w| w.triggers[0].condition = Some(nested(20)),
            &["triggers[0]", "if", "deep", "8"],
        ),
        (
            "condition value not finite",
            |w| {
                w.triggers[0].condition = Some(Condition::Compare {
                    var: "lit".into(),
                    op: CompareOp::Gt,
                    value: VarValue::Number(f64::NAN),
                })
            },
            &["triggers[0]", "if", "value", "finite"],
        ),
        (
            "step command empty",
            |w| w.triggers[0].actions[0].command = String::new(),
            &["triggers[0]", "do[0].command", "empty"],
        ),
        (
            "step params not an object",
            |w| w.triggers[0].actions[0].params = serde_json::json!([1]),
            &["do[0].params", "object"],
        ),
        (
            "too many steps",
            |w| {
                let step = w.triggers[0].actions[0].clone();
                w.triggers[0].actions = vec![step; 100];
            },
            &["triggers[0]", "do", "64"],
        ),
    ]
}

#[test]
fn every_scenario_rule_names_its_field() {
    for (what, mutate, needles) in cases() {
        let mut w = scenario_world();
        mutate(&mut w);
        let e = match w.validate() {
            Ok(()) => panic!("{what}: accepted"),
            Err(e) => e.to_string(),
        };
        for n in needles {
            assert!(e.contains(n), "{what}: {e:?} does not contain {n:?}");
        }
    }
}

#[test]
fn too_many_objectives_and_triggers_are_refused() {
    let mut w = World::new("many", 0);
    for i in 0..=crate::limits::MAX_OBJECTIVES {
        w.objectives.push(Objective {
            id: format!("o{i}"),
            title: String::new(),
            status: ObjectiveStatus::Open,
            hidden: false,
        });
    }
    let e = w.validate().unwrap_err().to_string();
    assert!(e.contains("objectives") && e.contains("1000"), "{e}");
    let mut w = World::new("many", 0);
    for i in 0..=crate::limits::MAX_TRIGGERS {
        w.triggers.push(Trigger {
            id: format!("t{i}"),
            when: When::Start,
            condition: None,
            actions: vec![],
            once: true,
            enabled: true,
        });
    }
    let e = w.validate().unwrap_err().to_string();
    assert!(e.contains("triggers") && e.contains("1000"), "{e}");
}

fn vars(pairs: &[(&str, VarValue)]) -> BTreeMap<String, VarValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn cmp(var: &str, op: CompareOp, value: VarValue) -> Condition {
    Condition::Compare {
        var: var.into(),
        op,
        value,
    }
}

#[test]
fn conditions_compare_numbers_strings_and_booleans() {
    use CompareOp::*;
    use VarValue::{Bool, Number as N, Text};
    let v = vars(&[("n", N(3.0)), ("s", Text("red".into())), ("b", Bool(true))]);
    let holds = |c: Condition| c.holds(&v);
    assert!(holds(cmp("n", Eq, N(3.0))));
    assert!(holds(cmp("n", Ge, N(3.0))) && holds(cmp("n", Le, N(3.0))));
    assert!(holds(cmp("n", Gt, N(2.5))) && !holds(cmp("n", Gt, N(3.0))));
    assert!(holds(cmp("n", Lt, N(4.0))) && !holds(cmp("n", Lt, N(3.0))));
    assert!(holds(cmp("n", Ne, N(1.0))) && !holds(cmp("n", Ne, N(3.0))));
    assert!(holds(cmp("s", Eq, Text("red".into()))));
    assert!(holds(cmp("s", Ne, Text("blue".into()))));
    assert!(holds(cmp("b", Eq, Bool(true))) && holds(cmp("b", Ne, Bool(false))));
    // Orderings need two numbers; another type is not equal.
    assert!(!holds(cmp("s", Lt, Text("z".into()))));
    assert!(!holds(cmp("n", Eq, Text("3".into()))) && holds(cmp("n", Ne, Text("3".into()))));
    // A missing variable equals nothing.
    assert!(!holds(cmp("gone", Eq, N(0.0))) && holds(cmp("gone", Ne, N(0.0))));
    assert!(!holds(cmp("gone", Ge, N(0.0))));
    // all / any, and their empty forms.
    assert!(holds(Condition::All(vec![])));
    assert!(!holds(Condition::Any(vec![])));
    assert!(holds(Condition::All(vec![
        cmp("n", Eq, N(3.0)),
        Condition::Any(vec![cmp("n", Eq, N(0.0)), cmp("b", Eq, Bool(true))]),
    ])));
    assert!(!holds(Condition::All(vec![
        cmp("n", Eq, N(3.0)),
        cmp("b", Eq, Bool(false))
    ])));
    // A condition deeper than the limit never holds (and never overflows the stack).
    let lit = vars(&[("lit", N(1.0))]);
    assert!(nested(crate::limits::MAX_CONDITION_DEPTH - 1).holds(&lit));
    assert!(!nested(1000).holds(&lit));
}

#[test]
fn full_world_with_a_scenario_stays_valid() {
    let mut w = full_world();
    let s = scenario_world();
    w.vars = s.vars.clone();
    w.objectives = s.objectives.clone();
    w.triggers = s.triggers.clone();
    w.validate().unwrap();
    assert_eq!(World::from_json(&w.to_json().unwrap()).unwrap(), w);
}
