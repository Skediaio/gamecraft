//! Levels of detail of a model: which geometry of a glTF file is drawn up close and which far
//! away, the distances the levels change at, and the one walk over a file's scene that both
//! readers of a model share ([`crate::inspect`] for the host, the renderer's parse).
//!
//! Two conventions say which nodes are the levels of one object, finest first:
//!
//! - **Names.** Nodes (or their meshes) named `<object>_LOD0`, `<object>_LOD1`, ... anywhere
//!   in the file, as Unity, Unreal and most exporters write them. All the nodes of one object
//!   name are its levels, in the order of their numbers. A number that occurs twice cannot be
//!   told apart, so only the finest level is drawn and the coarser nodes are left out, with a
//!   warning.
//! - **`MSFT_lod`**, the level-of-detail extension of the glTF project's vendor list: the
//!   finest node carries `extensions.MSFT_lod.ids`, the indices of the coarser nodes, which
//!   stand in for it and are usually left out of the scene. `extras.MSFT_screencoverage` on
//!   the same node gives each level's least share of the view (GameCraft reads the share as a
//!   share of the view's height).
//!
//! Every level of a group is drawn where the group's finest node in the scene stands, with
//! that node's transform; a level keeps its own rotation and scale, not its translation (a
//! modelling tool lays the levels out side by side). A level without geometry is left out; as
//! the last level of a file that means "draw nothing beyond here". The other geometry of the
//! scene is drawn at every level. A node of one group may hold groups of its own (a building with a roof that has levels of its own). Whatever is broken
//! (a level that has more triangles than the one before, a missing level, an id that points
//! nowhere) is a warning in [`Plan::warnings`]; the model is drawn all the same.
//!
//! **Distances.** Each level ends at a distance from the camera; the last one ends where the
//! model is no longer drawn. A source that leaves a distance out follows the next: the
//! `model` component (`lod_distances_m`, `draw_distance_m`), then the file
//! (`asset.extras.gamecraft` with the same two fields, else `MSFT_screencoverage`), then
//! [`rule`]: a screen-size rule from the model's size.

use crate::model::{IDENTITY, MAX_NODE_DEPTH, MAX_NODE_VISITS, apply, mul, to_f64};
use gamecraft_math::{DMat4, DQuat, DVec3, Portable, deg_to_rad};
use gamecraft_world::MAX_MODEL_LOD_LEVELS;
use gltf::Document;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The view the screen-size rule is measured in: a picture this many pixels high.
pub const REFERENCE_HEIGHT_PX: f64 = 1080.0;
/// The vertical field of view of that view, degrees.
pub const REFERENCE_FOV_Y_DEG: f64 = 60.0;
/// Where a level gives way to the next: the share of the reference view's height the model's
/// size still spans. A quarter of the height for the first change, then a sixteenth: a quarter
/// of the previous share per level.
pub const SWITCH_COVERAGE: [f64; MAX_MODEL_LOD_LEVELS - 1] = [
    0.25,
    0.0625,
    0.015625,
    0.003_906_25,
    0.000_976_562_5,
    0.000_244_140_625,
    0.000_061_035_156_25,
];
/// How far away a model is drawn: while its size spans this share of the reference view's
/// height (4 of 1080 pixels). A crate vanishes sooner than a house.
pub const DRAW_COVERAGE: f64 = 1.0 / 270.0;
/// A model smaller than this across, metres, counts as this size (a model without extent is
/// drawn some way too).
pub const MIN_MODEL_SIZE_M: f64 = 0.1;
/// The key of the distances in a glTF `asset`'s `extras`.
const SETTINGS_KEY: &str = "gamecraft";
/// The key of the screen coverage in a node's `extras`.
const COVERAGE_KEY: &str = "MSFT_screencoverage";
/// Most digits of the level number in a name.
const MAX_LEVEL_DIGITS: usize = 6;

/// How the levels of a model were found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodSource {
    /// By names ending in `_LOD<n>`.
    Names,
    /// By the `MSFT_lod` extension.
    MsftLod,
    /// Some groups by names, others by the extension.
    Mixed,
}

impl LodSource {
    /// Its name in `asset.list`.
    pub fn name(self) -> &'static str {
        match self {
            LodSource::Names => "names",
            LodSource::MsftLod => "MSFT_lod",
            LodSource::Mixed => "MSFT_lod+names",
        }
    }
}

/// The distances of the levels of one model, metres from the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct LodDistances {
    /// Where level `k` gives way to level `k + 1`: one fewer than the model has levels, each
    /// farther than the one before.
    pub switch_m: Vec<f64>,
    /// How far away the model is drawn.
    pub draw_m: f64,
}

/// What one source says of the distances of a model, each end by the level of the file it
/// ends: what it leaves out is `None` and follows the next source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LodOverride {
    /// Where each level gives way to the next, in metres, as [`LodDistances::switch_m`]; a
    /// shorter list ends where it stops.
    pub switch_m: Option<Vec<f64>>,
    /// How far away the model is drawn, in metres.
    pub draw_m: Option<f64>,
    /// `MSFT_screencoverage`: one share of the reference view's height per level of the
    /// file, the greatest first; level `k` ends where the model's size spans share `k`.
    /// Turned into metres where it is used, for the size the model has in the world.
    pub coverage: Option<Vec<f64>>,
}

/// The distance, metres, at which something `size_m` metres across spans `coverage` of the
/// reference view's height.
pub fn distance_for(size_m: f64, coverage: f64) -> f64 {
    let half_tan = deg_to_rad(REFERENCE_FOV_Y_DEG / 2.0).ptan();
    size_m / (2.0 * half_tan * coverage)
}

/// The extent of a box across, metres: the length of its diagonal, the size the rule follows.
pub fn size_across(min: [f64; 3], max: [f64; 3]) -> f64 {
    min.iter()
        .zip(max)
        .map(|(lo, hi)| (hi - lo) * (hi - lo))
        .sum::<f64>()
        .sqrt()
}

/// Grows the box `lo..hi` to hold the vertices `positions` (read as `f64`) where the
/// model-to-world matrix `world` puts them: the same products in the same order for every
/// reader of a model, so they agree to the bit.
pub fn extend_bounds(
    world: &[f64; 16],
    positions: &[[f32; 3]],
    lo: &mut [f64; 3],
    hi: &mut [f64; 3],
) {
    for q in positions {
        let w = apply(world, q.map(f64::from));
        for (axis, v) in w.iter().enumerate() {
            if let (Some(l), Some(h)) = (lo.get_mut(axis), hi.get_mut(axis)) {
                *l = l.min(*v);
                *h = h.max(*v);
            }
        }
    }
}

/// The size the rule follows: a model's extent across, at least [`MIN_MODEL_SIZE_M`] and a
/// finite number.
pub fn rule_size(size_m: f64) -> f64 {
    if size_m.is_finite() {
        size_m.max(MIN_MODEL_SIZE_M)
    } else {
        MIN_MODEL_SIZE_M
    }
}

/// The screen-size rule for a model `size_m` metres across with `levels` levels (1 to
/// [`MAX_MODEL_LOD_LEVELS`]): level `k` ends where the model's size spans
/// [`SWITCH_COVERAGE`]`[k]` of the reference view's height, and it is drawn while its size
/// spans [`DRAW_COVERAGE`].
pub fn rule(size_m: f64, levels: usize) -> LodDistances {
    let levels = levels.clamp(1, MAX_MODEL_LOD_LEVELS);
    let size = rule_size(size_m);
    LodDistances {
        switch_m: SWITCH_COVERAGE
            .iter()
            .take(levels - 1)
            .map(|&coverage| distance_for(size, coverage))
            .collect(),
        draw_m: distance_for(size, DRAW_COVERAGE),
    }
}

/// The least distance that is farther than `before` by more than a single-precision step
/// (the renderer holds distances as `f32` and merges levels that end at the same distance).
fn farther_than(before: f64) -> f64 {
    before * (1.0 + 1e-5) + 1e-3
}

/// Whether `list` is a usable list of switch distances: fewer than a model may have levels,
/// each finite and above 0, each farther than the one before.
fn usable_distances(list: &[f64]) -> bool {
    list.len() < MAX_MODEL_LOD_LEVELS
        && list.iter().all(|&d| d.is_finite() && d > 0.0)
        && list
            .windows(2)
            .all(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a < b))
}

/// A distance as a sentence shows it: no trailing zeros, at most three decimals.
fn shown(d: f64) -> String {
    if !d.is_finite() {
        return d.to_string();
    }
    let rounded = (d * 1000.0).round() / 1000.0;
    let text = format!("{rounded:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl LodDistances {
    /// Where each level ends, finest first; the last is the draw distance.
    pub fn ends_m(&self) -> Vec<f64> {
        self.switch_m.iter().copied().chain([self.draw_m]).collect()
    }

    /// The end of level `k` of `levels` (the last level's end is the draw distance).
    fn end(&self, k: usize, levels: usize) -> Option<f64> {
        if k + 1 == levels {
            Some(self.draw_m)
        } else {
            self.switch_m.get(k).copied()
        }
    }

    /// The distances of the `levels` levels of the file for a model `size_m` metres across
    /// (its extent times the scales it is drawn with: the rule and the coverage follow it).
    /// The end of each level is what the component says, else what the file says, else the
    /// rule's ([`rule`]). A distance that is not a number above 0 is replaced by the rule's.
    /// The ends must increase: one that would not is moved just past the one before. The
    /// sentences say what was replaced, moved or left unused.
    pub fn resolve(
        levels: usize,
        size_m: f64,
        file: Option<&LodOverride>,
        component: Option<&LodOverride>,
    ) -> (LodDistances, Vec<String>) {
        let levels = levels.clamp(1, MAX_MODEL_LOD_LEVELS);
        let rule = rule(size_m, levels);
        let mut notes = Vec::new();
        for (source, name) in [
            (file, "asset.extras.gamecraft.lod_distances_m"),
            (component, "lod_distances_m"),
        ] {
            if let Some(extra) = source
                .and_then(|s| s.switch_m.as_ref())
                .map(|s| s.len().saturating_sub(levels - 1))
                .filter(|&extra| extra > 0)
            {
                notes.push(format!(
                    "`{name}` gives {extra} switch distance(s) more than the model has changes \
                     of level; they are not used"
                ));
            }
        }
        let mut ends: Vec<f64> = (0..levels)
            .map(|k| {
                let fallback = rule.end(k, levels).unwrap_or(rule.draw_m);
                let given = component
                    .and_then(|c| c.end(k, levels, size_m))
                    .or_else(|| file.and_then(|f| f.end(k, levels, size_m)));
                match given {
                    Some(d) if d.is_finite() && d > 0.0 => d,
                    Some(d) => {
                        notes.push(format!(
                            "the distance {} given for the end of level {k} is not a number \
                             above 0; the rule's {} m is used instead",
                            shown(d),
                            shown(fallback)
                        ));
                        fallback
                    }
                    None => fallback,
                }
            })
            .collect();
        for k in 1..ends.len() {
            let (Some(before), Some(at)) = (ends.get(k - 1).copied(), ends.get_mut(k)) else {
                continue;
            };
            if *at <= before {
                notes.push(format!(
                    "level {k} would end at {} m, no farther than level {} ({} m); it ends at \
                     {} m instead",
                    shown(*at),
                    k - 1,
                    shown(before),
                    shown(farther_than(before))
                ));
                *at = farther_than(before);
            }
        }
        let switch_m = ends.iter().copied().take(levels - 1).collect();
        let draw_m = ends.last().copied().unwrap_or(rule.draw_m);
        (LodDistances { switch_m, draw_m }, notes)
    }
}

impl LodOverride {
    /// Whether this source says nothing.
    pub fn is_empty(&self) -> bool {
        self.switch_m.as_ref().is_none_or(Vec::is_empty)
            && self.draw_m.is_none()
            && self.coverage.as_ref().is_none_or(Vec::is_empty)
    }

    /// What this source says of where level `k` of `levels` ends, in metres as written.
    fn metres(&self, k: usize, levels: usize) -> Option<f64> {
        if k + 1 == levels {
            self.draw_m
        } else {
            self.switch_m.as_ref().and_then(|s| s.get(k).copied())
        }
    }

    /// What this source says of where level `k` of `levels` ends, for a model `size_m`
    /// metres across: its metres, else its coverage turned into metres.
    fn end(&self, k: usize, levels: usize, size_m: f64) -> Option<f64> {
        self.metres(k, levels).or_else(|| {
            self.coverage
                .as_ref()
                .and_then(|c| c.get(k))
                .map(|&share| distance_for(rule_size(size_m), share))
        })
    }
}

/// What `MSFT_screencoverage` means: one share of the reference view's height per level of
/// the file, the greatest first; level `k` ends where the model's size spans share `k`,
/// and the last share is where nothing is drawn (as the extension's example reads:
/// `0.5, 0.2, 0.01` draws the first level down to half the view's height, the second to a
/// fifth, the third to a hundredth).
///
/// [`check_coverage`] says why such a list cannot be used; [`from_coverage`] turns a usable
/// one into metres for a model `size_m` metres across.
pub fn check_coverage(levels: usize, coverage: &[f64]) -> Result<(), String> {
    from_coverage(1.0, levels, coverage).map(|_| ())
}

/// The distances `MSFT_screencoverage` means for a model `size_m` metres across with
/// `levels` levels ([`check_coverage`]); `Err` says why the values cannot be used.
pub fn from_coverage(size_m: f64, levels: usize, coverage: &[f64]) -> Result<LodOverride, String> {
    let bad = |why: String| Err(format!("{COVERAGE_KEY} {why}"));
    if coverage.len() != levels {
        return bad(format!(
            "has {} values where the model has {levels} levels",
            coverage.len()
        ));
    }
    if coverage
        .iter()
        .any(|&c| !c.is_finite() || c <= 0.0 || c > 1.0)
    {
        return bad("is not a list of shares between 0 and 1".into());
    }
    if coverage
        .windows(2)
        .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| b >= a))
    {
        return bad("does not get smaller from level to level".into());
    }
    // The k-th share is where level k ends: the changes of level are the first `levels - 1`,
    // and the last is where nothing is drawn.
    let size = rule_size(size_m);
    Ok(LodOverride {
        switch_m: Some(
            coverage
                .iter()
                .take(levels.saturating_sub(1))
                .map(|&c| distance_for(size, c))
                .collect(),
        ),
        draw_m: coverage.last().map(|&c| distance_for(size, c)),
        coverage: None,
    })
}

/// Sentences about the triangles of the levels, finest first: a level with more triangles
/// than the one before does not save anything where it should.
pub fn coarser_check(level_triangles: &[u64]) -> Vec<String> {
    level_triangles
        .windows(2)
        .enumerate()
        .filter_map(|(k, pair)| {
            let (before, now) = (pair.first().copied()?, pair.get(1).copied()?);
            (now > before).then(|| {
                format!(
                    "level {} has more triangles ({now}) than level {k} ({before}); the levels \
                     should get coarser, check their order (the model is drawn as it is)",
                    k + 1
                )
            })
        })
        .collect()
}

/// The object and level a name ending in `_LOD<n>` names, the letters in any case:
/// `House_LOD1` is (`House`, 1). `None` for any other name.
pub fn lod_name(name: &str) -> Option<(&str, u32)> {
    let bytes = name.as_bytes();
    let mut at: Option<usize> = None;
    for i in 0..=bytes.len().saturating_sub(4) {
        let is_mark = bytes.get(i..i + 4).is_some_and(|w| {
            w.first() == Some(&b'_') && w.get(1..4).is_some_and(|t| t.eq_ignore_ascii_case(b"LOD"))
        });
        if is_mark {
            at = Some(i);
        }
    }
    let digits = name.get(at? + 4..)?;
    let count = digits.len();
    if count == 0 || count > MAX_LEVEL_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let base = name.get(..at?)?;
    let number = digits.parse().ok()?;
    (!base.is_empty()).then_some((base, number))
}

/// One group of nodes that are the levels of the same object.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    /// The member that stands in the scene and stands for the group; every level is drawn
    /// in its place.
    anchor: usize,
    /// The levels by node index, finest first.
    members: Vec<usize>,
}

/// Which nodes of a glTF scene are levels of detail of one object, and what the file says
/// of their distances. Made by [`plan`], walked by [`walk`].
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The levels the model has, finest first; 1 without any.
    pub levels: usize,
    /// The groups, and which group every member belongs to.
    groups: Vec<Group>,
    member: BTreeMap<usize, usize>,
    /// Nodes that are never drawn (a level named twice).
    left_out: BTreeSet<usize>,
    /// How the levels were found; `None` for a model with one level.
    pub source: Option<LodSource>,
    /// `extras.MSFT_screencoverage` of the finest node of the first group that has it.
    coverage: Option<Vec<f64>>,
    /// The nodes named `*_collider` or `*-collider` (or whose mesh is): collision geometry,
    /// never drawn. [`colliders`] walks them for [`crate::inspect`].
    pub colliders: BTreeSet<usize>,
    /// `asset.extras.gamecraft`: what of it could be read.
    pub settings: LodOverride,
    /// What is broken in the file, as sentences.
    pub warnings: Vec<String>,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan {
            levels: 1,
            groups: Vec::new(),
            member: BTreeMap::new(),
            left_out: BTreeSet::new(),
            colliders: BTreeSet::new(),
            source: None,
            coverage: None,
            settings: LodOverride::default(),
            warnings: Vec::new(),
        }
    }
}

/// The numbers of a JSON list, or of one number (`Err` for anything else).
fn numbers(value: &Value) -> Result<Vec<f64>, String> {
    match value {
        Value::Array(list) if list.iter().all(Value::is_number) => {
            Ok(list.iter().filter_map(Value::as_f64).collect())
        }
        Value::Number(_) => Ok(vec![value.as_f64().unwrap_or(f64::NAN)]),
        _ => Err("is not a number".into()),
    }
}

/// A JSON value in a few words.
fn short(value: &Value) -> String {
    let text = value.to_string();
    text.chars().take(40).collect()
}

/// A document and its nodes by index, for the walks over the node tree.
struct Tree<'a> {
    document: &'a Document,
    nodes: Vec<gltf::Node<'a>>,
}

impl<'a> Tree<'a> {
    fn of(document: &'a Document) -> Tree<'a> {
        Tree {
            document,
            nodes: document.nodes().collect(),
        }
    }

    fn get(&self, index: usize) -> Option<gltf::Node<'a>> {
        self.nodes.get(index).cloned()
    }

    fn scene(&self) -> Option<gltf::Scene<'a>> {
        self.document
            .default_scene()
            .or_else(|| self.document.scenes().next())
    }

    /// The nodes the default scene reaches, by a walk over the children.
    fn reachable(&self) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let Some(scene) = self.scene() else {
            return seen;
        };
        let mut stack: Vec<usize> = scene.nodes().map(|n| n.index()).collect();
        let mut visits = 0usize;
        while let Some(index) = stack.pop() {
            visits += 1;
            if visits > MAX_NODE_VISITS {
                break;
            }
            if !seen.insert(index) {
                continue;
            }
            if let Some(node) = self.get(index) {
                stack.extend(node.children().map(|c| c.index()));
            }
        }
        seen
    }

    /// Whether `target` is `from` or lies below it, by a walk over the children.
    fn descends_from(&self, from: usize, target: usize) -> bool {
        let mut stack = vec![from];
        let mut seen = BTreeSet::new();
        let mut visits = 0usize;
        while let Some(index) = stack.pop() {
            if index == target {
                return true;
            }
            visits += 1;
            if visits > MAX_NODE_VISITS || !seen.insert(index) {
                continue;
            }
            if let Some(node) = self.get(index) {
                stack.extend(node.children().map(|c| c.index()));
            }
        }
        false
    }
}

impl Plan {
    fn warn(&mut self, text: String) {
        self.warnings.push(text);
    }

    /// Adds a group of `members` (finest first) whose finest member in the scene is its
    /// anchor, unless a member holds the anchor below it (the walk would never end); whether
    /// the group was added.
    fn add_group(
        &mut self,
        tree: &Tree<'_>,
        reachable: &BTreeSet<usize>,
        mut members: Vec<usize>,
        what: &str,
    ) -> bool {
        if members.len() > MAX_MODEL_LOD_LEVELS {
            self.warn(format!(
                "{what} has {} levels; a model draws at most {MAX_MODEL_LOD_LEVELS}, the finest",
                members.len()
            ));
            members.truncate(MAX_MODEL_LOD_LEVELS);
        }
        let anchor = members
            .iter()
            .copied()
            .find(|m| reachable.contains(m))
            .or_else(|| members.first().copied());
        let Some(anchor) = anchor else {
            return false;
        };
        if members
            .iter()
            .any(|&m| m != anchor && tree.descends_from(m, anchor))
        {
            self.warn(format!(
                "{what}: its finest level lies below one of its own coarser levels, so drawing \
                 the levels would never end; they are left as they stand in the file"
            ));
            return false;
        }
        let id = self.groups.len();
        for &m in &members {
            self.member.entry(m).or_insert(id);
        }
        self.groups.push(Group { anchor, members });
        true
    }

    /// `asset.extras.gamecraft`: `lod_distances_m` and `draw_distance_m`.
    fn read_settings(&mut self, asset: Option<&Value>) {
        let Some(settings) = asset
            .and_then(|a| a.get("extras"))
            .and_then(|e| e.get(SETTINGS_KEY))
        else {
            return;
        };
        let Some(fields) = settings.as_object() else {
            self.warn(format!(
                "asset.extras.{SETTINGS_KEY} is not an object with lod_distances_m and \
                 draw_distance_m; it is ignored"
            ));
            return;
        };
        for (key, value) in fields {
            let field = format!("asset.extras.{SETTINGS_KEY}.{key}");
            let list = numbers(value);
            let as_text = |list: &[f64]| {
                list.iter()
                    .map(|d| shown(*d))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            match key.as_str() {
                "lod_distances_m" => match list {
                    Ok(list) if usable_distances(&list) => self.settings.switch_m = Some(list),
                    Ok(list) => self.warn(format!(
                        "`{field}` ({}) is not a list of fewer than {MAX_MODEL_LOD_LEVELS} \
                         distances in metres above 0 that increase; it is ignored",
                        as_text(&list)
                    )),
                    Err(why) => self.warn(format!("`{field}` {why}; it is ignored")),
                },
                "draw_distance_m" => match list {
                    Ok(list)
                        if list.len() == 1
                            && list.first().is_some_and(|d| d.is_finite() && *d > 0.0) =>
                    {
                        self.settings.draw_m = list.first().copied();
                    }
                    Ok(list) => self.warn(format!(
                        "`{field}` ({}) is not one distance in metres above 0; it is ignored",
                        as_text(&list)
                    )),
                    Err(why) => self.warn(format!("`{field}` {why}; it is ignored")),
                },
                other => self.warn(format!(
                    "`asset.extras.{SETTINGS_KEY}.{other}` is not known; lod_distances_m and \
                     draw_distance_m are read"
                )),
            }
        }
    }

    /// The `MSFT_lod` extension on the scene's nodes.
    fn read_extension(&mut self, tree: &Tree<'_>, nodes: &[&Value], reachable: &BTreeSet<usize>) {
        let count = tree.nodes.len();
        for node in &tree.nodes {
            let index = node.index();
            let Some(extension) = nodes
                .get(index)
                .and_then(|n| n.get("extensions"))
                .and_then(|e| e.get("MSFT_lod"))
            else {
                continue;
            };
            let name = node.name().unwrap_or("").to_string();
            let what = format!("MSFT_lod on node {index} ({name})");
            let Some(ids) = extension.get("ids").and_then(Value::as_array) else {
                self.warn(format!("{what} has no list of ids; it is ignored"));
                continue;
            };
            let mut members = vec![index];
            for id in ids {
                let named = id
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|&i| i < count);
                match named {
                    Some(i) if i == index => self.warn(format!(
                        "{what} names the node itself among its lower levels; that entry is \
                         ignored"
                    )),
                    Some(i) if members.contains(&i) => self.warn(format!(
                        "{what} names node {i} twice; the second entry is ignored"
                    )),
                    Some(i) => members.push(i),
                    None => self.warn(format!(
                        "{what} names {} where the file has {count} nodes; that entry is ignored",
                        short(id)
                    )),
                }
            }
            if members.len() < 2 {
                self.warn(format!(
                    "{what} names no other node of the file; it is ignored"
                ));
                continue;
            }
            if !reachable.contains(&index) {
                self.warn(format!(
                    "{what}: its finest node is not in the scene, so its levels are never \
                     drawn; it is ignored"
                ));
                continue;
            }
            if members.iter().any(|m| self.member.contains_key(m)) {
                self.warn(format!(
                    "{what} names a node that is already a level of another group; it is \
                     ignored"
                ));
                continue;
            }
            if self.coverage.is_none()
                && let Some(value) = nodes
                    .get(index)
                    .and_then(|n| n.get("extras"))
                    .and_then(|e| e.get(COVERAGE_KEY))
            {
                match numbers(value) {
                    Ok(list) => self.coverage = Some(list),
                    Err(why) => self.warn(format!(
                        "{COVERAGE_KEY} on node {index} {why}; it is ignored"
                    )),
                }
            }
            if self.add_group(tree, reachable, members, &what) {
                self.source = Some(match self.source {
                    Some(LodSource::Names) | Some(LodSource::Mixed) => LodSource::Mixed,
                    _ => LodSource::MsftLod,
                });
            }
        }
    }

    /// Names ending in `_LOD<n>`, grouped by the object name.
    fn read_names(&mut self, tree: &Tree<'_>, reachable: &BTreeSet<usize>) {
        // Object name -> level number -> the nodes so named.
        let mut by_object: BTreeMap<String, BTreeMap<u32, Vec<usize>>> = BTreeMap::new();
        for node in &tree.nodes {
            let named = lod_name(node.name().unwrap_or(""))
                .or_else(|| node.mesh().and_then(|m| lod_name(m.name().unwrap_or(""))));
            let Some((base, number)) = named else {
                continue;
            };
            if self.member.contains_key(&node.index()) {
                continue;
            }
            by_object
                .entry(base.to_string())
                .or_default()
                .entry(number)
                .or_default()
                .push(node.index());
        }
        for (base, numbers) in by_object {
            // One level only: ordinary geometry that happens to be named with a level.
            if numbers.len() < 2 {
                continue;
            }
            let Some((&finest, _)) = numbers.first_key_value() else {
                continue;
            };
            let twice: Vec<String> = numbers
                .iter()
                .filter(|(_, list)| list.len() > 1)
                .map(|(number, _)| format!("{base}_LOD{number}"))
                .collect();
            if !twice.is_empty() {
                // The levels cannot be told apart: the finest is drawn alone, the rest never.
                let mut left_out = 0usize;
                for (number, list) in &numbers {
                    if *number == finest {
                        continue;
                    }
                    for &index in list {
                        self.left_out.insert(index);
                        left_out += 1;
                    }
                }
                self.warn(format!(
                    "`{}` {} named more than once in the file, so the levels cannot \
                     be told apart: _LOD{finest} is drawn alone and {left_out} coarser node(s) \
                     are left out; give each object its own name",
                    twice.join("`, `"),
                    if twice.len() == 1 { "is" } else { "are" }
                ));
                continue;
            }
            let counts: Vec<u32> = numbers.keys().copied().collect();
            for pair in counts.windows(2) {
                if let (Some(a), Some(b)) = (pair.first().copied(), pair.get(1).copied())
                    && b != a + 1
                {
                    self.warn(format!(
                        "`{base}` has no level between _LOD{a} and _LOD{b}; the levels are \
                         drawn in the order of their numbers"
                    ));
                }
            }
            if finest != 0 {
                self.warn(format!(
                    "`{base}` has no _LOD0: its finest level is _LOD{finest}"
                ));
            }
            let members: Vec<usize> = numbers.values().flatten().copied().collect();
            if self.add_group(tree, reachable, members, &format!("the levels of `{base}`")) {
                self.source = Some(match self.source {
                    Some(LodSource::MsftLod) | Some(LodSource::Mixed) => LodSource::Mixed,
                    _ => LodSource::Names,
                });
            }
        }
    }

    /// What the levels of the file are once its triangles are counted (`level_triangles`, by
    /// level of the file): the levels that draw, what the file says of the distances, and
    /// every warning about the file.
    pub fn finish(&self, level_triangles: &[u64]) -> Levels {
        let mut warnings = self.warnings.clone();
        let coverage = self
            .coverage
            .as_deref()
            .filter(|c| match check_coverage(self.levels, c) {
                Ok(()) => true,
                Err(why) => {
                    warnings.push(format!("{why}; the rule from the model's size is used"));
                    false
                }
            });
        // A level without geometry is left out; as the last one it ends the model there.
        let drawn: Vec<usize> = (0..self.levels)
            .filter(|&k| level_triangles.get(k).copied().unwrap_or(0) > 0)
            .collect();
        for k in 0..self.levels {
            if drawn.contains(&k) {
                continue;
            }
            if k + 1 == self.levels && !drawn.is_empty() {
                warnings.push(format!(
                    "level {k} has no geometry and is left out: nothing of the model is drawn \
                     beyond where that level would have started"
                ));
            } else {
                warnings.push(format!(
                    "level {k} has no geometry and is left out; the level after it starts where \
                     it would have"
                ));
            }
        }
        let triangles: Vec<u64> = drawn
            .iter()
            .map(|&k| level_triangles.get(k).copied().unwrap_or(0))
            .collect();
        warnings.extend(coarser_check(&triangles));
        let file = LodOverride {
            switch_m: self.settings.switch_m.clone(),
            draw_m: self.settings.draw_m,
            coverage: coverage.map(<[f64]>::to_vec),
        };
        Levels {
            levels: self.levels,
            drawn,
            triangles,
            file: (!file.is_empty()).then_some(file),
            warnings,
        }
    }
}

/// The levels of a model as both of its readers hold them: the ones that draw, what the file
/// says of the distances and what is broken in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Levels {
    /// The levels the file has, finest first; 1 without any. The distances of the file are
    /// by this level ([`LodDistances::resolve`], [`LodOverride`]).
    pub levels: usize,
    /// The levels of the file that draw (that have geometry), finest first: a level without
    /// geometry is left out.
    pub drawn: Vec<usize>,
    /// Triangles of each drawn level, in the order of [`Levels::drawn`].
    pub triangles: Vec<u64>,
    /// What the file itself says of the distances of its levels; `None` when it says
    /// nothing.
    pub file: Option<LodOverride>,
    /// What is broken in the file, as sentences; the model is drawn all the same.
    pub warnings: Vec<String>,
}

impl Default for Levels {
    /// Nothing read yet: one level, without geometry.
    fn default() -> Levels {
        Levels {
            levels: 1,
            drawn: Vec::new(),
            triangles: Vec::new(),
            file: None,
            warnings: Vec::new(),
        }
    }
}

impl Levels {
    /// Where each drawn level ends, from the ends `ends` of every level of the file, finest
    /// first ([`LodDistances::ends_m`]): the last is the draw distance.
    pub fn drawn_ends(&self, ends: &[f64]) -> Vec<f64> {
        self.drawn
            .iter()
            .map(|&k| ends.get(k).copied().unwrap_or(f64::INFINITY))
            .collect()
    }
}

/// Whether a distance of the levels of a model follows its size (the rule's, or the file's
/// coverage), so that entities of different scales cannot share one item: false when every
/// level's end is given in metres by the component or by the file.
pub fn follows_scale(
    levels: usize,
    file: Option<&LodOverride>,
    component: Option<&LodOverride>,
) -> bool {
    (0..levels).any(|k| {
        component
            .and_then(|c| c.metres(k, levels))
            .or_else(|| file.and_then(|f| f.metres(k, levels)))
            .is_none()
    })
}

/// Which nodes of `document` (whose JSON chunk is `json`) are levels of detail of one
/// object, and what the file says of their distances. Anything broken is a warning in
/// [`Plan::warnings`]; this never fails.
pub fn plan(document: &Document, json: &[u8]) -> Plan {
    let mut plan = Plan::default();
    let root: Value = match serde_json::from_slice(json) {
        Ok(root) => root,
        Err(e) => {
            plan.warn(format!(
                "the file's extensions and extras could not be read ({e}); the model is drawn \
                 as one level"
            ));
            return plan;
        }
    };
    plan.read_settings(root.get("asset"));
    let nodes: Vec<&Value> = root
        .get("nodes")
        .and_then(Value::as_array)
        .map(|n| n.iter().collect())
        .unwrap_or_default();
    let tree = Tree::of(document);
    let reachable = tree.reachable();
    plan.read_extension(&tree, &nodes, &reachable);
    plan.read_names(&tree, &reachable);
    for node in &tree.nodes {
        let collider = is_collider_name(node.name().unwrap_or(""))
            || node
                .mesh()
                .is_some_and(|m| is_collider_name(m.name().unwrap_or("")));
        if collider {
            plan.colliders.insert(node.index());
        }
    }
    plan.levels = plan
        .groups
        .iter()
        .map(|g| g.members.len())
        .max()
        .unwrap_or(1)
        .max(1);
    plan
}

/// One primitive of the scene: where it is in the file (the node that shows it), its
/// model-to-world matrix and the levels that draw it.
#[derive(Clone, Debug)]
pub struct Visit<'a> {
    /// The primitive.
    pub primitive: gltf::Primitive<'a>,
    /// The index of the node that shows the primitive in the scene.
    pub node: usize,
    /// Model-to-world, column-major, as [`crate::inspect`] measures it (the same products
    /// in the same order, so the two readers agree to the bit).
    pub world: [f64; 16],
    /// Bit `k` is set when the primitive is drawn at level `k`.
    pub drawn_at: u8,
}

/// The ends of the name of a collision node: a node (or a mesh) named `<something>_collider` or
/// `<something>-collider`, in any case, is collision geometry ([`is_collider_name`]). Both
/// spellings are read because packs use both.
pub const COLLIDER_SUFFIXES: [&str; 2] = ["_collider", "-collider"];

/// Whether a node's or a mesh's name marks collision geometry: it ends in one of
/// [`COLLIDER_SUFFIXES`], the letters in any case, with something in front of it.
pub fn is_collider_name(name: &str) -> bool {
    COLLIDER_SUFFIXES.iter().any(|suffix| {
        name.len() > suffix.len()
            && name
                .get(name.len() - suffix.len()..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
    })
}

/// The levels `k` for which bit `k` of `mask` is set, lowest first.
pub fn levels_of(mask: u8) -> impl Iterator<Item = usize> {
    (0..8).filter(move |k| mask >> k & 1 == 1)
}

/// A node's transform without its translation, column-major: its rotation and scale as
/// glTF gives them (a node given as a matrix is taken apart).
fn unshifted(t: &gltf::scene::Transform) -> [f64; 16] {
    let (scale, rotation) = match *t {
        gltf::scene::Transform::Decomposed {
            rotation, scale, ..
        } => (DVec3::from_array(scale.map(f64::from)), {
            let [x, y, z, w] = rotation.map(f64::from);
            DQuat::from_xyzw(x, y, z, w)
        }),
        gltf::scene::Transform::Matrix { matrix } => {
            let f64_matrix = matrix.map(|column| column.map(f64::from));
            let (scale, rotation, _) =
                DMat4::from_cols_array_2d(&f64_matrix).to_scale_rotation_translation();
            (scale, rotation)
        }
    };
    DMat4::from_scale_rotation_translation(scale, rotation, DVec3::ZERO).to_cols_array()
}

/// The mask of levels `lo..hi`: bits `lo` to `hi - 1` set (none for an empty range).
fn mask(lo: u8, hi: u8) -> u8 {
    if lo >= hi || hi > 8 {
        return 0;
    }
    (((1u16 << (hi - lo)) - 1) << lo) as u8
}

/// Walks the default scene of `document` by [`Plan`] and gives every primitive to draw with
/// its model-to-world matrix and its levels. The levels of a group are drawn in the place
/// and the transform of its anchor (the finest node of the group that is in the scene), each
/// over its own range of the levels, keeping its own rotation and scale; a coarser level met
/// where it stands in the file stands in there instead, and a node the plan leaves out is
/// not drawn. Geometry of no group is drawn at every level. An error, never a panic, for a file without a scene
/// or a node tree that is too deep or loops.
pub fn walk<'a>(document: &'a Document, plan: &Plan) -> Result<Vec<Visit<'a>>, String> {
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or("the file has no scene")?;
    let nodes: Vec<gltf::Node<'a>> = document.nodes().collect();
    let top = u8::try_from(plan.levels).unwrap_or(u8::MAX);
    // A node, its parent's transform, its depth, the first level and the end of the levels
    // it is drawn at, and whether it stands in for another node.
    let mut stack: Vec<(usize, [f64; 16], usize, u8, u8, bool)> = scene
        .nodes()
        .map(|n| (n.index(), IDENTITY, 0, 0, top, false))
        .collect();
    let mut visits = 0usize;
    let mut out = Vec::new();
    // Draws the primitives of `node`'s mesh over the levels `lo..hi` with `world`.
    let draw =
        |node: &gltf::Node<'a>, world: [f64; 16], lo: u8, hi: u8, out: &mut Vec<Visit<'a>>| {
            let drawn_at = mask(lo, hi);
            if drawn_at == 0 {
                return;
            }
            if let Some(mesh) = node.mesh() {
                for primitive in mesh.primitives() {
                    out.push(Visit {
                        primitive,
                        node: node.index(),
                        world,
                        drawn_at,
                    });
                }
            }
        };
    while let Some((index, parent, depth, lo, hi, stands_in)) = stack.pop() {
        visits += 1;
        if visits > MAX_NODE_VISITS || depth > MAX_NODE_DEPTH {
            return Err("the scene's node tree is too deep or loops".into());
        }
        let Some(node) = nodes.get(index) else {
            continue;
        };
        if stands_in {
            // A coarser level in the place of the finest node in the scene: it keeps its own
            // rotation and scale, not its translation.
            let world = mul(&parent, &unshifted(&node.transform()));
            stack.extend(
                node.children()
                    .map(|c| (c.index(), world, depth + 1, lo, hi, false)),
            );
            draw(node, world, lo, hi, &mut out);
            continue;
        }
        // A level named twice, or collision geometry (and everything below it): never drawn.
        if plan.left_out.contains(&index) || plan.colliders.contains(&index) {
            continue;
        }
        let world = mul(&parent, &to_f64(&node.transform().matrix()));
        if let Some(&g) = plan.member.get(&index) {
            let Some(group) = plan.groups.get(g) else {
                continue;
            };
            if group.anchor != index {
                continue;
            }
            // Every level of the group stands here, each over its own range of the levels.
            for (j, &member) in group.members.iter().enumerate() {
                let start = lo.saturating_add(u8::try_from(j).unwrap_or(u8::MAX));
                let end = if j + 1 < group.members.len() {
                    hi.min(start.saturating_add(1))
                } else {
                    hi
                };
                if start >= end {
                    break;
                }
                if member == index {
                    stack.extend(
                        node.children()
                            .map(|c| (c.index(), world, depth + 1, start, end, false)),
                    );
                    draw(node, world, start, end, &mut out);
                } else {
                    stack.push((member, world, depth + 1, start, end, true));
                }
            }
            continue;
        }
        stack.extend(
            node.children()
                .map(|c| (c.index(), world, depth + 1, lo, hi, false)),
        );
        draw(node, world, lo, hi, &mut out);
    }
    Ok(out)
}

/// Walks the default scene of `document` and gives every primitive of its **collision nodes**
/// (those of [`Plan::colliders`], [`is_collider_name`]) with its model-to-world matrix: what
/// [`crate::inspect`] measures as a model's collision parts. Every node is met where it stands
/// in the file, whatever level-of-detail group it belongs to. An error, never a panic, for a
/// file without a scene or a node tree that is too deep or loops.
pub fn colliders<'a>(document: &'a Document, plan: &Plan) -> Result<Vec<Visit<'a>>, String> {
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or("the file has no scene")?;
    let nodes: Vec<gltf::Node<'a>> = document.nodes().collect();
    let mut stack: Vec<(usize, [f64; 16], usize)> =
        scene.nodes().map(|n| (n.index(), IDENTITY, 0)).collect();
    let mut visits = 0usize;
    let mut out = Vec::new();
    while let Some((index, parent, depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_NODE_VISITS || depth > MAX_NODE_DEPTH {
            return Err("the scene's node tree is too deep or loops".into());
        }
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let world = mul(&parent, &to_f64(&node.transform().matrix()));
        if plan.colliders.contains(&index)
            && let Some(mesh) = node.mesh()
        {
            for primitive in mesh.primitives() {
                out.push(Visit {
                    primitive,
                    node: index,
                    world,
                    drawn_at: 1,
                });
            }
        }
        stack.extend(node.children().map(|c| (c.index(), world, depth + 1)));
    }
    Ok(out)
}

/// The sockets of a model: every node of the default scene named `socket_<name>`
/// ([`gamecraft_world::attachments::socket_name_of`]) with its position and full rotation in
/// the asset's own axes, by name. A name used twice keeps the first; a node outside the scene
/// has no place and is no socket. An error, never a panic, for a file without a scene or a node
/// tree that is too deep or loops.
pub fn sockets(document: &Document) -> Result<BTreeMap<String, gamecraft_world::Socket>, String> {
    use gamecraft_world::attachments::socket_name_of;
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or("the file has no scene")?;
    let nodes: Vec<gltf::Node<'_>> = document.nodes().collect();
    let mut stack: Vec<(usize, [f64; 16], DQuat, usize)> = scene
        .nodes()
        .map(|n| (n.index(), IDENTITY, DQuat::IDENTITY, 0))
        .collect();
    let mut visits = 0usize;
    // Every socket with its node, then the first node of a name in the file wins.
    let mut found: Vec<(usize, String, gamecraft_world::Socket)> = Vec::new();
    while let Some((index, parent, parent_turn, depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_NODE_VISITS || depth > MAX_NODE_DEPTH {
            return Err("the scene's node tree is too deep or loops".into());
        }
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let world = mul(&parent, &to_f64(&node.transform().matrix()));
        // The turn is the turns of the chain, multiplied: read back from the matrix, a node
        // scaled per axis would turn the socket by something that is no turn at all.
        let own = DQuat::from_array(node.transform().decomposed().1.map(f64::from));
        let own_sq = own.length_squared();
        if !own_sq.is_finite() || own_sq <= 1e-24 {
            return Err(format!(
                "node {} has a rotation that is not a turn (a quaternion of length zero or                  not a number)",
                node.name().unwrap_or(&index.to_string())
            ));
        }
        let turn = parent_turn * (own * (1.0 / own_sq.sqrt()));
        let name = node.name().unwrap_or("");
        if let Some(socket) = socket_name_of(name) {
            let position = DMat4::from_cols_array(&world).w_axis.truncate();
            found.push((
                index,
                socket.to_string(),
                gamecraft_world::Socket {
                    position: position.to_array(),
                    rotation: turn.to_array(),
                },
            ));
        }
        stack.extend(node.children().map(|c| (c.index(), world, turn, depth + 1)));
    }
    found.sort_by_key(|(index, _, _)| *index);
    let mut out: BTreeMap<String, gamecraft_world::Socket> = BTreeMap::new();
    for (_, name, socket) in found {
        out.entry(name).or_insert(socket);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn names_ending_in_a_level_number_name_an_object_and_a_level() {
        assert_eq!(lod_name("House_LOD0"), Some(("House", 0)));
        assert_eq!(lod_name("House_LOD12"), Some(("House", 12)));
        assert_eq!(lod_name("house_lod1"), Some(("house", 1)));
        assert_eq!(lod_name("A_B_LOD2_LOD3"), Some(("A_B_LOD2", 3)));
        for other in [
            "House",
            "House_LOD",
            "House_LODone",
            "House_LOD1x",
            "_LOD1",
            "HouseLOD1",
            "House_LOD1234567",
            "House_LOD1.001",
        ] {
            assert_eq!(lod_name(other), None, "{other}");
        }
    }

    #[test]
    fn the_rule_scales_with_the_size_and_the_levels() {
        let half_tan = 0.5773502691896257;
        // A metre across: 4 of 1080 pixels at 60 degrees.
        let one_m = rule(1.0, 1);
        assert!(one_m.switch_m.is_empty());
        assert!(close(one_m.draw_m, 1.0 / (2.0 * half_tan * DRAW_COVERAGE)));
        let three = rule(2.0, 3);
        assert_eq!(three.switch_m.len(), 2);
        assert!(close(three.switch_m[0], 2.0 / (2.0 * half_tan * 0.25)));
        assert!(close(three.switch_m[1], 2.0 / (2.0 * half_tan * 0.0625)));
        assert!(close(three.draw_m, 2.0 * one_m.draw_m));
        // Eight levels at most; a model without extent counts as a decimetre.
        assert_eq!(rule(1.0, 99).switch_m.len(), MAX_MODEL_LOD_LEVELS - 1);
        assert!(close(rule(0.0, 1).draw_m, rule(MIN_MODEL_SIZE_M, 1).draw_m));
        assert!(close(
            rule(f64::NAN, 2).switch_m[0],
            rule(MIN_MODEL_SIZE_M, 2).switch_m[0]
        ));
    }

    #[test]
    fn the_component_wins_over_the_file_which_wins_over_the_rule() {
        let rule = rule(10.0, 3);
        let file = LodOverride {
            switch_m: Some(vec![50.0]),
            draw_m: Some(900.0),
            coverage: None,
        };
        let component = LodOverride {
            switch_m: Some(vec![80.0, 300.0, 400.0]),
            draw_m: None,
            coverage: None,
        };
        let (d, notes) = LodDistances::resolve(3, 10.0, Some(&file), Some(&component));
        assert_eq!(d.switch_m, vec![80.0, 300.0]);
        assert_eq!(
            d.draw_m, 900.0,
            "the file's draw distance, the component gives none"
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("lod_distances_m") && notes[0].contains("1 switch distance"),
            "{notes:?}"
        );
        // Only the file: its first switch distance, then the rule's second.
        let (d, notes) = LodDistances::resolve(3, 10.0, Some(&file), None);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(d.switch_m[0], 50.0);
        assert_eq!(d.switch_m[1], rule.switch_m[1]);
        assert_eq!(d.draw_m, 900.0);
        // Neither: the rule.
        let (d, notes) = LodDistances::resolve(3, 10.0, None, None);
        assert!(notes.is_empty() && d == rule);
        // A bad distance falls back to the rule's, and the ends must increase.
        let bad = LodOverride {
            switch_m: Some(vec![0.0, 20.0]),
            draw_m: Some(f64::NAN),
            coverage: None,
        };
        let (d, notes) = LodDistances::resolve(3, 10.0, Some(&bad), None);
        assert_eq!(d.switch_m[0], rule.switch_m[0]);
        assert_eq!(d.draw_m, rule.draw_m);
        assert!(d.switch_m[1] > d.switch_m[0], "{d:?}");
        assert!(
            notes.iter().any(|n| n.contains("not a number above 0")),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("no farther than level 0")),
            "{notes:?}"
        );
        assert_eq!(d.ends_m().len(), 3);
    }

    #[test]
    fn coverage_follows_the_size_and_metres_do_not() {
        let file = LodOverride {
            switch_m: None,
            draw_m: None,
            coverage: Some(vec![1.0, 0.25, 0.02]),
        };
        assert!(
            follows_scale(3, Some(&file), None),
            "the coverage follows the size"
        );
        let half_tan = 0.5773502691896257;
        let (small, notes) = LodDistances::resolve(3, 2.0, Some(&file), None);
        assert!(notes.is_empty(), "{notes:?}");
        let (big, _) = LodDistances::resolve(3, 20.0, Some(&file), None);
        for k in 0..3 {
            assert!(close(big.ends_m()[k], 10.0 * small.ends_m()[k]), "{k}");
        }
        assert!(close(small.ends_m()[0], 2.0 / (2.0 * half_tan)));
        // Metres as written do not follow the size, wherever they come from.
        let metres = LodOverride {
            switch_m: Some(vec![40.0, 150.0]),
            draw_m: Some(800.0),
            coverage: None,
        };
        assert!(!follows_scale(3, Some(&metres), None));
        let file = LodOverride {
            switch_m: Some(vec![40.0]),
            draw_m: None,
            coverage: Some(vec![1.0, 0.25, 0.02]),
        };
        assert!(
            follows_scale(3, Some(&file), None),
            "the second end follows the coverage"
        );
        let (d, _) = LodDistances::resolve(3, 2.0, Some(&file), Some(&metres));
        assert_eq!(
            (d.switch_m, Some(d.draw_m)),
            (vec![40.0, 150.0], Some(800.0))
        );
        assert!(!follows_scale(3, Some(&file), Some(&metres)));
        assert!(LodOverride::default().is_empty());
    }

    #[test]
    fn screen_coverage_becomes_distances_per_level() {
        let half_tan = 0.5773502691896257;
        let d = from_coverage(1.0, 3, &[0.5, 0.2, 0.01]).unwrap();
        assert!(close(
            d.switch_m.as_deref().unwrap()[0],
            1.0 / (2.0 * half_tan * 0.5)
        ));
        assert!(close(
            d.switch_m.as_deref().unwrap()[1],
            1.0 / (2.0 * half_tan * 0.2)
        ));
        assert!(
            d.draw_m
                .is_some_and(|m| close(m, 1.0 / (2.0 * half_tan * 0.01)))
        );
        assert!(
            from_coverage(1.0, 2, &[0.5, 0.2, 0.01]).is_err(),
            "one value per level"
        );
        assert!(
            from_coverage(1.0, 3, &[0.5, 0.5, 0.1]).is_err(),
            "they must shrink"
        );
        assert!(
            from_coverage(1.0, 3, &[0.5, 0.2, 2.0]).is_err(),
            "shares are at most 1"
        );
    }

    #[test]
    fn a_level_with_more_triangles_than_the_one_before_is_said() {
        assert!(coarser_check(&[300, 120, 12]).is_empty());
        assert!(coarser_check(&[300]).is_empty());
        let notes = coarser_check(&[300, 400, 12, 40]);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].contains("level 1 has more triangles (400) than level 0 (300)"),
            "{notes:?}"
        );
        assert!(
            notes[1].contains("level 3 has more triangles (40) than level 2 (12)"),
            "{notes:?}"
        );
    }

    #[test]
    fn the_size_of_a_box_is_its_diagonal() {
        assert!(close(size_across([-0.5; 3], [0.5; 3]), 3f64.sqrt()));
        assert!(close(size_across([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), 0.0));
    }

    #[test]
    fn distances_are_shown_without_trailing_zeros() {
        assert_eq!(shown(40.0), "40");
        assert_eq!(shown(150.5), "150.5");
        assert_eq!(shown(3.464_101_615_137_754), "3.464");
        assert_eq!(shown(f64::NAN), "NaN");
    }
}
