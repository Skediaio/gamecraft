//! The shared entity map behind a world, and the check of a changed world.

use crate::store::EntityMap;
use crate::{Component, Entity, EntityDraft, EntityId, Prefab, Shape, ShapeKind, World};
use std::collections::BTreeMap;

fn entity(id: u64) -> Entity {
    Entity {
        id: EntityId(id),
        name: format!("e{id}"),
        position: [id as f64, 0.0, 0.0],
        rotation_y_deg: 0.0,
        scale: 1.0,
        prefab: None,
        parent: None,
        layer: None,
        prefab_child: None,
        components: Vec::new(),
    }
}

/// A box at a place of its own.
fn crate_draft(i: u64) -> EntityDraft {
    EntityDraft {
        name: format!("b-{i}"),
        position: [(i % 100) as f64 * 3.0, 0.0, (i / 100) as f64 * 3.0],
        components: vec![Component::Shape(Shape {
            kind: ShapeKind::Box,
            size: [1.0, 1.0, 1.0],
            color: [0.5, 0.5, 0.5],
            solid: true,
        })],
        ..EntityDraft::default()
    }
}

fn world_of(n: u64) -> World {
    let mut w = World::new("Crates", 1);
    for i in 0..n {
        w.add_entity(crate_draft(i)).unwrap();
    }
    w
}

/// A fixed stream of numbers (SplitMix64), so the test is the same on every run.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn assert_same(map: &EntityMap, model: &BTreeMap<EntityId, Entity>) {
    assert_eq!(map.len(), model.len());
    let got: Vec<(EntityId, &Entity)> = map.iter().collect();
    let want: Vec<(EntityId, &Entity)> = model.iter().map(|(k, e)| (*k, e)).collect();
    assert_eq!(got, want);
    assert_eq!(map.iter().size_hint(), (model.len(), Some(model.len())));
}

#[test]
fn the_map_behaves_as_an_ordered_map_for_any_keys() {
    let mut stream = Stream(7);
    let mut map = EntityMap::default();
    let mut model = BTreeMap::new();
    let mut copies = Vec::new();
    for step in 0..4000 {
        let roll = stream.next();
        // Mostly small keys, some far apart, the largest ones too.
        let key = match roll % 10 {
            0 => u64::MAX - (roll >> 60),
            1 => roll >> 20,
            _ => (roll >> 8) % 3000,
        };
        let id = EntityId(key);
        match (roll >> 4) % 4 {
            0 | 1 => {
                let replaced = map.insert(id, entity(key));
                assert_eq!(replaced, model.insert(id, entity(key)).is_some());
            }
            2 => assert_eq!(map.remove(id), model.remove(&id)),
            _ => {
                if let Some(e) = map.get_mut(id) {
                    e.scale = 2.0;
                }
                if let Some(e) = model.get_mut(&id) {
                    e.scale = 2.0;
                }
            }
        }
        assert_eq!(map.get(id), model.get(&id));
        assert_eq!(map.contains_key(id), model.contains_key(&id));
        if step % 500 == 0 {
            copies.push((map.clone(), model.clone()));
        }
    }
    assert_same(&map, &model);
    // Copies taken along the way kept what they held: a change never shows through.
    for (copy, then) in &copies {
        assert_same(copy, then);
        assert_eq!(*copy == map, *then == model);
    }
    // Two maps with the same entries are equal however they were built: here one grew
    // tall for a large key that is gone again.
    let mut a: EntityMap = model.iter().map(|(k, e)| (*k, e.clone())).collect();
    let b = a.clone();
    a.insert(EntityId(u64::MAX), entity(1));
    assert_ne!(a, b);
    a.remove(EntityId(u64::MAX));
    assert_eq!(a, b);
    let small: EntityMap = (1..40).map(|k| (EntityId(k), entity(k))).collect();
    let mut tall = small.clone();
    tall.insert(EntityId(1 << 40), entity(5));
    tall.remove(EntityId(1 << 40));
    assert_eq!(tall, small);
    assert_eq!(small, tall);
    assert_eq!(format!("{small:?}").len(), format!("{tall:?}").len());
}

#[test]
fn a_change_to_a_copy_copies_only_its_path_however_many_entities_there_are() {
    // The work of one change, counted in nodes copied, must not grow with the world.
    let mut copied = Vec::new();
    for n in [1_000, 30_000] {
        let before = world_of(n);
        let mut after = before.clone();
        assert_eq!(after.entities.unshared_nodes(&before.entities), 0);
        after.add_entity(crate_draft(n)).unwrap();
        let added = after.entities.unshared_nodes(&before.entities);
        let mut moved = before.clone();
        moved.entity_mut(EntityId(n / 2)).unwrap().position[1] = 1.0;
        let changed = moved.entities.unshared_nodes(&before.entities);
        copied.push((added, changed));
    }
    // Ids below 1 024 take two levels, below 32 768 three; one node is copied per level.
    assert_eq!(copied, vec![(2, 2), (3, 3)]);
}

#[test]
fn checking_a_change_looks_only_at_what_changed() {
    for n in [100, 20_000] {
        let before = world_of(n);
        before.validate().unwrap();
        let mut after = before.clone();
        assert_eq!(after.validate_change(&before), Ok(0));
        after.add_entity(crate_draft(n)).unwrap();
        assert_eq!(after.validate_change(&before), Ok(1), "{n} entities");
        after.entity_mut(EntityId(3)).unwrap().name = "moved".into();
        assert_eq!(after.validate_change(&before), Ok(2), "{n} entities");
        // A removal can leave a child without its parent: everything is checked.
        let mut removed = before.clone();
        removed.remove_entity(EntityId(5));
        assert_eq!(removed.validate_change(&before), Ok(n as usize - 1));
    }
}

/// The change must give exactly what a whole check gives.
fn assert_checks_agree(before: &World, after: &World) {
    before.validate().unwrap();
    let whole = after.validate();
    let change = after.validate_change(before);
    assert_eq!(
        change.clone().map(drop),
        whole,
        "validate_change and validate disagree"
    );
    if whole.is_ok() {
        assert!(change.is_ok());
    }
}

#[test]
fn checking_a_change_finds_the_same_first_error_as_a_whole_check() {
    let mut before = world_of(50);
    before
        .put_prefab(Prefab {
            id: "crate".into(),
            name: "Crate".into(),
            components: vec![Component::Shape(Shape {
                kind: ShapeKind::Box,
                size: [1.0, 1.0, 1.0],
                color: [0.5, 0.5, 0.5],
                solid: true,
            })],
            children: Vec::new(),
        })
        .unwrap();
    let child = before.add_entity(crate_draft(60)).unwrap();
    before.entity_mut(child).unwrap().parent = Some(EntityId(2));
    let grandchild = before.add_entity(crate_draft(61)).unwrap();
    before.entity_mut(grandchild).unwrap().parent = Some(child);
    before.validate().unwrap();

    // Two bad entities: the first by id is the one reported.
    let mut w = before.clone();
    w.entity_mut(EntityId(40)).unwrap().position[0] = f64::NAN;
    w.entity_mut(EntityId(7)).unwrap().scale = 0.0;
    assert_checks_agree(&before, &w);
    assert!(w.validate().is_err());

    // A parent that does not exist, and a loop through an entity left as it was.
    let mut w = before.clone();
    w.entity_mut(EntityId(9)).unwrap().parent = Some(EntityId(999));
    assert_checks_agree(&before, &w);
    let mut w = before.clone();
    w.entity_mut(EntityId(2)).unwrap().parent = Some(grandchild);
    assert_checks_agree(&before, &w);

    // An entity removed under the map's feet leaves a child without its parent.
    let mut w = before.clone();
    w.entities.remove(child);
    assert_checks_agree(&before, &w);
    assert!(w.validate().is_err());

    // A stored id that differs from its key, a key at the counter, the counter lowered.
    let mut w = before.clone();
    w.entity_mut(EntityId(4)).unwrap().id = EntityId(5);
    assert_checks_agree(&before, &w);
    let mut w = before.clone();
    let next = w.next_entity_id().0;
    w.entities.insert(EntityId(next), entity(next));
    assert_checks_agree(&before, &w);
    let mut w = before.clone();
    w.next_entity_id = EntityId(10);
    assert_checks_agree(&before, &w);

    // An entity placed from a prefab without the prefab's component; a prefab taken away.
    let mut w = before.clone();
    w.entity_mut(EntityId(12)).unwrap().prefab = Some("crate".into());
    w.entity_mut(EntityId(12)).unwrap().components.clear();
    assert_checks_agree(&before, &w);
    let mut w = before.clone();
    w.entity_mut(EntityId(12)).unwrap().prefab = Some("crate".into());
    w.validate().unwrap();
    let placed = w.clone();
    w.prefabs.clear();
    assert_checks_agree(&placed, &w);

    // Changes that stay valid pass both ways.
    let mut w = before.clone();
    w.add_entity(crate_draft(70)).unwrap();
    w.entity_mut(EntityId(30)).unwrap().name = "renamed".into();
    assert_checks_agree(&before, &w);
    assert_eq!(w.validate_change(&before), Ok(2));
}

#[test]
fn changes_are_found_when_the_map_grew_a_level() {
    let before: EntityMap = (1..32).map(|k| (EntityId(k), entity(k))).collect();
    let mut after = before.clone();
    after.insert(EntityId(5000), entity(5000));
    after.insert(EntityId(32), entity(32));
    after.get_mut(EntityId(3)).unwrap().scale = 2.0;
    let changes = after.changes(&before).unwrap();
    assert_eq!(
        changes.changed,
        vec![EntityId(3), EntityId(32), EntityId(5000)]
    );
    assert!(!changes.removed);
    after.remove(EntityId(4));
    assert!(after.changes(&before).unwrap().removed);
    // The other way round the shapes cannot be matched: the caller checks everything.
    assert!(before.changes(&after).is_none());
    // Adding the 32nd entity grows the map, and is still one entity to check.
    let world = world_of(31);
    let mut grown = world.clone();
    grown.add_entity(crate_draft(31)).unwrap();
    assert_eq!(grown.validate_change(&world), Ok(1));
}

#[test]
fn a_copy_of_a_grid_terrain_shares_its_heights_and_writes_a_plain_list() {
    use crate::{GridHeights, Terrain, TerrainSource};
    let heights: Vec<f64> = (0..9).map(|i| f64::from(i) * 0.5).collect();
    let t = Terrain {
        size_m: 64.0,
        cell_m: 4.0,
        source: TerrainSource::Grid {
            cols: 3,
            rows: 3,
            heights: heights.clone().into(),
        },
        ..Default::default()
    };
    let copy = t.clone();
    let (TerrainSource::Grid { heights: a, .. }, TerrainSource::Grid { heights: b, .. }) =
        (&t.source, &copy.source)
    else {
        panic!("not a grid");
    };
    assert!(
        std::ptr::eq(a.as_ptr(), b.as_ptr()),
        "the copy shares the list"
    );
    assert_eq!(copy, t);
    assert_eq!(**a, *heights);
    assert_eq!(format!("{a:?}"), format!("{heights:?}"));
    // The same numbers in another list are equal; other numbers are not.
    assert_eq!(GridHeights::from(heights.clone()), *a);
    assert_ne!(GridHeights::from(vec![0.0; 9]), *a);
    // In JSON exactly what a list of numbers gives, and back.
    assert_eq!(
        serde_json::to_string(a).unwrap(),
        serde_json::to_string(&heights).unwrap()
    );
    let back: Terrain = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
    assert_eq!(back, t);
}
