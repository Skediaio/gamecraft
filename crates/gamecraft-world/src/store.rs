//! The entities of a world, by id, shared between copies of the world.
//!
//! A persistent radix tree keyed by the id's bits, 32 ways per level, in the style of the
//! persistent vectors of Bagwell's "Ideal Hash Trees" (2001) and Okasaki's "Purely
//! Functional Data Structures" (1998): nodes sit behind `Arc`s, so a copy of the map is one
//! reference count and a change copies only the nodes on the path to the changed id
//! (O(log n)). A command works on a copy of the world and the session keeps earlier copies as
//! undo steps; with this map neither costs time or memory in proportion to the number of
//! entities. Comparing two maps skips every subtree they share, and [`EntityMap::changes`]
//! lists the ids where one map differs from another, which is what checking a changed world
//! needs (`World::validate_change`).
//!
//! The map holds no empty nodes (a removal prunes them), so two maps with the same entries
//! have the same shape up to their height, which only grows.

use crate::{Entity, EntityId};
use std::sync::Arc;

/// Bits of the id per level.
const BITS: u32 = 5;
/// Children of a node.
const WIDTH: usize = 1 << BITS;
/// The bits of one level.
const MASK: u64 = (1 << BITS) - 1;
/// Most branch levels above the leaves: 13 levels of 5 bits cover all 64 bits of an id.
const MAX_BRANCH_LEVELS: u32 = 12;

#[derive(Clone)]
enum Node {
    /// Level 1 and up: the nodes one level down.
    Branch([Option<Arc<Node>>; WIDTH]),
    /// Level 0: the entities.
    Leaf([Option<Arc<Entity>>; WIDTH]),
}

impl Node {
    fn empty(level: u32) -> Node {
        if level == 0 {
            Node::Leaf(std::array::from_fn(|_| None))
        } else {
            Node::Branch(std::array::from_fn(|_| None))
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Node::Branch(children) => children
                .iter()
                .all(|c| c.as_ref().is_none_or(|n| n.is_empty())),
            Node::Leaf(items) => items.iter().all(Option::is_none),
        }
    }
}

/// The slot of `key` in a node at `level`.
fn slot(key: u64, level: u32) -> usize {
    ((key >> (BITS * level)) & MASK) as usize
}

/// The lowest key below slot `index` of a node at `level` whose own lowest key is `base`.
fn key_of(base: u64, index: usize, level: u32) -> u64 {
    base | ((index as u64) << (BITS * level))
}

/// The entities of a world by id, in id order; cheap to copy (see the module).
#[derive(Clone, Default)]
pub(crate) struct EntityMap {
    root: Option<Arc<Node>>,
    /// Branch levels above the leaves: the map holds keys below `32^(levels + 1)`.
    levels: u32,
    len: usize,
}

impl EntityMap {
    /// How many entities the map holds.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Whether `key` fits below the root as it is.
    fn fits(&self, key: u64) -> bool {
        key.checked_shr(BITS * (self.levels + 1)).unwrap_or(0) == 0
    }

    /// The entity stored under `key`.
    pub(crate) fn get(&self, key: EntityId) -> Option<&Entity> {
        let key = key.0;
        if !self.fits(key) {
            return None;
        }
        let mut node = self.root.as_deref()?;
        let mut level = self.levels;
        loop {
            match node {
                Node::Branch(children) => {
                    node = children.get(slot(key, level))?.as_deref()?;
                    level = level.checked_sub(1)?;
                }
                Node::Leaf(items) => return items.get(slot(key, 0))?.as_deref(),
            }
        }
    }

    /// Whether an entity is stored under `key`.
    pub(crate) fn contains_key(&self, key: EntityId) -> bool {
        self.get(key).is_some()
    }

    /// The entity stored under `key`, to change; copies the nodes on its path that are shared.
    pub(crate) fn get_mut(&mut self, key: EntityId) -> Option<&mut Entity> {
        // Copy nothing for a key that is not there.
        self.get(key)?;
        get_mut_in(self.root.as_mut()?, key.0, self.levels)
    }

    /// Stores `entity` under `key`; true when it replaced one.
    pub(crate) fn insert(&mut self, key: EntityId, entity: Entity) -> bool {
        let key = key.0;
        while !self.fits(key) && self.levels < MAX_BRANCH_LEVELS {
            if let Some(root) = self.root.take() {
                let mut children: [Option<Arc<Node>>; WIDTH] = std::array::from_fn(|_| None);
                if let Some(first) = children.first_mut() {
                    *first = Some(root);
                }
                self.root = Some(Arc::new(Node::Branch(children)));
            }
            self.levels += 1;
        }
        let replaced = insert_in(&mut self.root, key, self.levels, Arc::new(entity));
        if !replaced {
            self.len = self.len.saturating_add(1);
        }
        replaced
    }

    /// Removes the entity stored under `key` and returns it.
    pub(crate) fn remove(&mut self, key: EntityId) -> Option<Entity> {
        // Copy nothing for a key that is not there.
        self.get(key)?;
        let removed = remove_in(&mut self.root, key.0, self.levels)?;
        self.len = self.len.saturating_sub(1);
        Some(Arc::try_unwrap(removed).unwrap_or_else(|shared| (*shared).clone()))
    }

    /// Every entry, in key order.
    pub(crate) fn iter(&self) -> Iter<'_> {
        let mut stack = Vec::new();
        if let Some(root) = self.root.as_deref() {
            stack.push((root, 0, 0, self.levels));
        }
        Iter {
            stack,
            remaining: Some(self.len),
        }
    }

    /// Every entity, in key order.
    pub(crate) fn values(&self) -> impl Iterator<Item = &Entity> {
        self.iter().map(|(_, e)| e)
    }

    /// Where this map differs from `before`: the keys whose entity is new or not the very
    /// same as in `before` (a shared entity is the same; an equal copy counts as changed), in
    /// key order, and whether a key of `before` is gone. `None` when the two cannot be
    /// compared node by node (`before` is the taller, which a map made from it by these
    /// functions never is); the caller then looks at every entry.
    pub(crate) fn changes(&self, before: &EntityMap) -> Option<Changes> {
        let mut changes = Changes::default();
        lift(
            self.root.as_ref(),
            self.levels,
            before.root.as_ref(),
            before.levels,
            &mut changes,
        )
        .then_some(changes)
    }

    /// How many nodes of this map are not shared with `other` (a measure of what a change
    /// copied).
    #[cfg(test)]
    pub(crate) fn unshared_nodes(&self, other: &EntityMap) -> usize {
        fn collect(node: &Arc<Node>, out: &mut Vec<*const Node>) {
            out.push(Arc::as_ptr(node));
            if let Node::Branch(children) = &**node {
                for c in children.iter().flatten() {
                    collect(c, out);
                }
            }
        }
        let (mut mine, mut theirs) = (Vec::new(), Vec::new());
        if let Some(r) = &self.root {
            collect(r, &mut mine);
        }
        if let Some(r) = &other.root {
            collect(r, &mut theirs);
        }
        let theirs: std::collections::BTreeSet<*const Node> = theirs.into_iter().collect();
        mine.iter().filter(|p| !theirs.contains(p)).count()
    }
}

/// What [`EntityMap::changes`] found.
#[derive(Default)]
pub(crate) struct Changes {
    /// Keys whose entity is new or changed, in key order.
    pub(crate) changed: Vec<EntityId>,
    /// A key of the earlier map is gone.
    pub(crate) removed: bool,
}

fn get_mut_in(node: &mut Arc<Node>, key: u64, level: u32) -> Option<&mut Entity> {
    match Arc::make_mut(node) {
        Node::Branch(children) => {
            let child = children.get_mut(slot(key, level))?.as_mut()?;
            get_mut_in(child, key, level.checked_sub(1)?)
        }
        Node::Leaf(items) => items.get_mut(slot(key, 0))?.as_mut().map(Arc::make_mut),
    }
}

/// Stores `entity` under `key` below `node` (a node at `level`, made when missing); true when
/// it replaced an entity.
fn insert_in(node: &mut Option<Arc<Node>>, key: u64, level: u32, entity: Arc<Entity>) -> bool {
    let node = Arc::make_mut(node.get_or_insert_with(|| Arc::new(Node::empty(level))));
    match node {
        Node::Branch(children) => {
            match (children.get_mut(slot(key, level)), level.checked_sub(1)) {
                (Some(child), Some(below)) => insert_in(child, key, below, entity),
                // Unreachable for a map built by these functions: a branch sits above level 0.
                _ => false,
            }
        }
        Node::Leaf(items) => match items.get_mut(slot(key, 0)) {
            Some(item) => item.replace(entity).is_some(),
            None => false,
        },
    }
}

/// Removes the entity under `key` below `node` (at `level`) and drops nodes left empty.
fn remove_in(node: &mut Option<Arc<Node>>, key: u64, level: u32) -> Option<Arc<Entity>> {
    let inner = Arc::make_mut(node.as_mut()?);
    let removed = match inner {
        Node::Branch(children) => {
            let child = children.get_mut(slot(key, level))?;
            remove_in(child, key, level.checked_sub(1)?)
        }
        Node::Leaf(items) => items.get_mut(slot(key, 0))?.take(),
    };
    if inner.is_empty_shallow() {
        *node = None;
    }
    removed
}

impl Node {
    /// No child at all (children are never empty themselves).
    fn is_empty_shallow(&self) -> bool {
        match self {
            Node::Branch(children) => children.iter().all(Option::is_none),
            Node::Leaf(items) => items.iter().all(Option::is_none),
        }
    }
}

/// [`diff`] of a root `now` at `now_level` and a root `before` at `before_level`: the
/// lowest slots of the taller `now` hold what `before` holds, the other slots only new keys.
/// False when `before` is the taller or the shapes do not match.
fn lift(
    now: Option<&Arc<Node>>,
    now_level: u32,
    before: Option<&Arc<Node>>,
    before_level: u32,
    out: &mut Changes,
) -> bool {
    if now_level == before_level {
        return diff(now, before, now_level, 0, out);
    }
    let Some(below) = now_level
        .checked_sub(1)
        .filter(|_| now_level > before_level)
    else {
        return false;
    };
    let Some(node) = now else {
        out.removed |= before.is_some_and(|b| !b.is_empty());
        return true;
    };
    let Node::Branch(children) = &**node else {
        return false;
    };
    // Slot 0 holds the lower keys, so it comes first in key order.
    if !lift(
        children.first().and_then(Option::as_ref),
        below,
        before,
        before_level,
        out,
    ) {
        return false;
    }
    for (i, child) in children.iter().enumerate().skip(1) {
        if let Some(child) = child {
            let added = Iter {
                stack: vec![(&**child, 0, key_of(0, i, now_level), below)],
                remaining: None,
            };
            out.changed.extend(added.map(|(k, _)| k));
        }
    }
    true
}

/// Collects into `out` where `now` differs from `before`, both at `level` with lowest key
/// `base`; false when their shapes do not match.
fn diff(
    now: Option<&Arc<Node>>,
    before: Option<&Arc<Node>>,
    level: u32,
    base: u64,
    out: &mut Changes,
) -> bool {
    match (now, before) {
        (None, None) => true,
        (Some(a), Some(b)) if Arc::ptr_eq(a, b) => true,
        (None, Some(b)) => {
            out.removed |= !b.is_empty();
            true
        }
        (Some(a), None) => {
            let added = Iter {
                stack: vec![(&**a, 0, base, level)],
                remaining: None,
            };
            out.changed.extend(added.map(|(k, _)| k));
            true
        }
        (Some(a), Some(b)) => match (&**a, &**b) {
            (Node::Branch(x), Node::Branch(y)) => {
                let Some(below) = level.checked_sub(1) else {
                    return false;
                };
                x.iter().zip(y.iter()).enumerate().all(|(i, (c, d))| {
                    diff(c.as_ref(), d.as_ref(), below, key_of(base, i, level), out)
                })
            }
            (Node::Leaf(x), Node::Leaf(y)) => {
                for (i, (c, d)) in x.iter().zip(y.iter()).enumerate() {
                    match (c, d) {
                        (Some(e), Some(f)) if Arc::ptr_eq(e, f) => {}
                        (Some(_), _) => out.changed.push(EntityId(key_of(base, i, 0))),
                        (None, Some(_)) => out.removed = true,
                        (None, None) => {}
                    }
                }
                true
            }
            _ => false,
        },
    }
}

/// Whether the subtrees `a` and `b` (at the same level) hold the same entries.
fn same(a: Option<&Arc<Node>>, b: Option<&Arc<Node>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) if Arc::ptr_eq(x, y) => true,
        (Some(x), None) | (None, Some(x)) => x.is_empty(),
        (Some(x), Some(y)) => match (&**x, &**y) {
            (Node::Branch(p), Node::Branch(q)) => p
                .iter()
                .zip(q.iter())
                .all(|(c, d)| same(c.as_ref(), d.as_ref())),
            (Node::Leaf(p), Node::Leaf(q)) => p.iter().zip(q.iter()).all(|(c, d)| match (c, d) {
                (None, None) => true,
                (Some(e), Some(f)) => Arc::ptr_eq(e, f) || **e == **f,
                _ => false,
            }),
            _ => false,
        },
    }
}

impl PartialEq for EntityMap {
    /// The same entries: compares only the subtrees the two maps do not share.
    fn eq(&self, other: &EntityMap) -> bool {
        if self.len != other.len {
            return false;
        }
        // Bring the taller root down to the other's height: what lies outside its first
        // slot must be empty.
        let (mut tall, mut tall_levels, short, short_levels) = if self.levels >= other.levels {
            (
                self.root.as_ref(),
                self.levels,
                other.root.as_ref(),
                other.levels,
            )
        } else {
            (
                other.root.as_ref(),
                other.levels,
                self.root.as_ref(),
                self.levels,
            )
        };
        while tall_levels > short_levels {
            let Some(node) = tall else {
                break;
            };
            let Node::Branch(children) = &**node else {
                return false;
            };
            if children
                .iter()
                .skip(1)
                .any(|c| c.as_ref().is_some_and(|n| !n.is_empty()))
            {
                return false;
            }
            tall = children.first().and_then(Option::as_ref);
            tall_levels -= 1;
        }
        same(tall, short)
    }
}

impl std::fmt::Debug for EntityMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl FromIterator<(EntityId, Entity)> for EntityMap {
    fn from_iter<I: IntoIterator<Item = (EntityId, Entity)>>(entries: I) -> EntityMap {
        let mut map = EntityMap::default();
        for (key, entity) in entries {
            map.insert(key, entity);
        }
        map
    }
}

/// The entries of an [`EntityMap`] in key order.
pub(crate) struct Iter<'a> {
    /// The nodes being walked: node, next slot, lowest key, level.
    stack: Vec<(&'a Node, usize, u64, u32)>,
    /// Entries still to come, when known.
    remaining: Option<usize>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (EntityId, &'a Entity);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (node, next, base, level) = self.stack.last_mut()?;
            let index = *next;
            if index >= WIDTH {
                self.stack.pop();
                continue;
            }
            *next += 1;
            let (node, base, level) = (*node, *base, *level);
            match node {
                Node::Branch(children) => {
                    if let (Some(Some(child)), Some(below)) =
                        (children.get(index), level.checked_sub(1))
                    {
                        self.stack
                            .push((&**child, 0, key_of(base, index, level), below));
                    }
                }
                Node::Leaf(items) => {
                    if let Some(Some(entity)) = items.get(index) {
                        self.remaining = self.remaining.map(|n| n.saturating_sub(1));
                        return Some((EntityId(key_of(base, index, 0)), &**entity));
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.remaining {
            Some(n) => (n, Some(n)),
            None => (0, None),
        }
    }
}
