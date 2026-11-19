//! The session: the document, the running simulation, the command registry and the history.
//!
//! Modelled on VisionCraft's `crates/visioncraft-core/src/session.rs` (MIT): commands are
//! registered with their spec and checked against it before the handler runs, a World command
//! runs on a copy and the previous document is one undo step (snapshot undo, limit 100), and a
//! transaction makes several commands one step or none. Adapted: handlers are `fn` pointers of
//! three kinds (by [`Scope`]), and a session has two states, editing and playing.

use crate::error::{CommandError, failed};
use crate::journal::{Journal, JournalEnd, JournalEntry, MAX_JOURNAL_ENTRIES, hash_text};
use crate::params::Params;
use crate::spec::{CommandSpec, Scope, nearest};
use crate::triggers::PlayLog;
use gamecraft_sim::Sim;
use gamecraft_world::{HeightField, ModelBounds, Terrain, World};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

/// Most undo steps kept.
pub const UNDO_LIMIT: usize = 100;

/// The world a World command edits: a copy of the target world, and the ground of the world
/// it was copied from.
pub struct Edit<'a> {
    /// The copy; the handler changes it. The session validates it afterwards.
    pub world: World,
    /// The height field of the world before the command (`None` without terrain).
    heights: Option<&'a HeightField>,
    /// A height field the handler built for the new terrain, so the session need not build
    /// it again.
    pub(crate) built: Option<HeightField>,
}

impl Edit<'_> {
    /// The ground height at `x`, `z` of the world before the command; an error that says why
    /// when there is no ground there (no terrain, or outside it).
    pub fn ground(&self, x: f64, z: f64) -> Result<f64, CommandError> {
        if self.world.terrain.is_none() {
            return Err(failed(format!(
                "there is no ground at [{x}, {z}]: the world has no terrain; add one with \
                 terrain.set"
            )));
        }
        let Some(field) = self.heights else {
            return Err(failed(format!(
                "there is no ground at [{x}, {z}]: the terrain could not be built"
            )));
        };
        field.height_at(x, z).ok_or_else(|| {
            let [x0, z0] = field.min();
            let [x1, z1] = field.max();
            failed(format!(
                "there is no ground at [{x}, {z}]: it is outside the terrain, which spans x \
                 from {x0} to {x1} and z from {z0} to {z1}"
            ))
        })
    }

    /// The ground height at `x`, `z`, `None` where there is none.
    pub fn ground_or_none(&self, x: f64, z: f64) -> Option<f64> {
        self.world.terrain.as_ref()?;
        self.heights?.height_at(x, z)
    }

    /// The height field of the world before the command (`None` without terrain).
    pub fn heights(&self) -> Option<&HeightField> {
        self.heights
    }
}

/// A World handler: changes the copy in `edit`.
pub type WorldFn = fn(&mut Edit<'_>, Params<'_>) -> Result<Value, CommandError>;
/// A Query handler: reads the session.
pub type QueryFn = fn(&Session, Params<'_>) -> Result<Value, CommandError>;
/// A Play handler: works on the session's play state.
pub type PlayFn = fn(&mut Session, Params<'_>) -> Result<Value, CommandError>;

/// A command's handler, by scope.
#[derive(Clone, Copy)]
pub enum Handler {
    /// Changes the document, or the running world while playing.
    World(WorldFn),
    /// Reads only.
    Query(QueryFn),
    /// Starts, stops, steps or feeds the simulation.
    Play(PlayFn),
}

impl Handler {
    /// The scope this kind of handler belongs to.
    pub fn scope(self) -> Scope {
        match self {
            Handler::World(_) => Scope::World,
            Handler::Query(_) => Scope::Query,
            Handler::Play(_) => Scope::Play,
        }
    }
}

/// One command of a module's `SPECS`: how to build its spec, and its handler.
pub struct Registration {
    /// Builds the spec.
    pub spec: fn() -> CommandSpec,
    /// Runs the command.
    pub handler: Handler,
}

/// Every command module's list. A new module of commands is one more line here.
const MODULES: &[&[Registration]] = &[
    crate::world_cmds::SPECS,
    crate::entity_cmds::SPECS,
    crate::play_cmds::SPECS,
    crate::combat_cmds::SPECS,
    crate::inspect_cmds::SPECS,
    crate::find_cmds::SPECS,
    crate::geometry_cmds::SPECS,
    crate::aim_cmds::SPECS,
    crate::catalog_cmds::SPECS,
    crate::scatter_cmds::SPECS,
    crate::scenario_cmds::SPECS,
    crate::prefab_cmds::SPECS,
    crate::journal_cmds::SPECS,
    crate::gm_cmds::SPECS,
    crate::terrain_cmds::SPECS,
];

/// The spec and scope of the built-in command `name`, without a session (for checking the
/// commands inside a trigger's `do`).
pub(crate) fn registered(name: &str) -> Option<(CommandSpec, Scope)> {
    MODULES.iter().flat_map(|m| m.iter()).find_map(|r| {
        let spec = (r.spec)();
        (spec.name == name).then(|| (spec, r.handler.scope()))
    })
}

/// The ids of every built-in command.
pub(crate) fn registered_names() -> Vec<String> {
    MODULES
        .iter()
        .flat_map(|m| m.iter())
        .map(|r| (r.spec)().name)
        .collect()
}

#[derive(Clone)]
struct Command {
    spec: CommandSpec,
    handler: Handler,
}

/// The height field of the document, with the terrain it was built from.
#[derive(Clone)]
struct HeightCache {
    terrain: Option<Terrain>,
    field: Option<Arc<HeightField>>,
}

impl HeightCache {
    fn for_world(world: &World) -> HeightCache {
        HeightCache {
            terrain: world.terrain.clone(),
            field: world
                .terrain
                .as_ref()
                .and_then(|t| t.build().ok())
                .map(Arc::new),
        }
    }
}

/// A world being edited and, while playing, a simulation of it. Every action is a command.
///
/// **Editing**: World commands change the document; each change is one undo step.
/// **Playing** (after `sim.start`): the simulation runs on a copy of the document; World
/// commands change that running world and leave the document untouched; `sim.stop` throws
/// the running world away or, with `keep`, makes it the document as one undo step.
#[derive(Clone)]
pub struct Session {
    document: World,
    live: Option<Sim>,
    commands: BTreeMap<String, Command>,
    undo: VecDeque<World>,
    redo: Vec<World>,
    revision: u64,
    view_revision: u64,
    heights: HeightCache,
    in_transaction: bool,
    /// `replace_document` ran inside the current transaction: the history is cleared when it
    /// succeeds.
    cleared_in_transaction: bool,
    /// What the scenario keeps while playing: messages, trigger errors, variables changed
    /// since the last step.
    play: PlayLog,
    /// The journal of the current simulation, or of the last one until the next `sim.start`.
    journal: Option<Journal>,
    /// The journal a `sim.start` inside the running transaction replaced, to put back when
    /// the transaction fails.
    journal_stash: Option<Option<Journal>>,
    /// How deep [`Session::run`] is nested (a trigger's commands run inside a step): only
    /// the outermost command is recorded in the journal.
    depth: u32,
}

/// What a transaction remembers of the journal: its length, end and truncation.
type JournalMark = Option<(usize, Option<JournalEnd>, bool)>;

impl Session {
    /// A session editing `world`, with the built-in commands registered.
    pub fn new(world: World) -> Session {
        let mut commands = BTreeMap::new();
        for module in MODULES {
            for registration in module.iter() {
                let mut spec = (registration.spec)();
                // The handler decides what the command can touch, so it decides the scope.
                spec.scope = registration.handler.scope();
                commands.insert(
                    spec.name.clone(),
                    Command {
                        spec,
                        handler: registration.handler,
                    },
                );
            }
        }
        Session {
            heights: HeightCache::for_world(&world),
            document: world,
            live: None,
            commands,
            undo: VecDeque::new(),
            redo: Vec::new(),
            revision: 0,
            view_revision: 0,
            in_transaction: false,
            cleared_in_transaction: false,
            play: PlayLog::default(),
            journal: None,
            journal_stash: None,
            depth: 0,
        }
    }

    /// The journal of the running simulation, or of the last one until the next `sim.start`
    /// (`docs/design/journal.md`); `None` before the first `sim.start`.
    pub fn journal(&self) -> Option<&Journal> {
        self.journal.as_ref()
    }

    /// Records a command that succeeded while playing, at `tick` (the tick before it ran).
    fn record(&mut self, command: &str, params: &Value, tick: u64) {
        let Some(sim) = &self.live else {
            return;
        };
        let Some(journal) = &mut self.journal else {
            return;
        };
        if journal.truncated {
            return;
        }
        if journal.entries.len() >= MAX_JOURNAL_ENTRIES {
            journal.truncated = true;
            return;
        }
        let after = sim.tick();
        let hash = hash_text(sim.state_hash());
        journal.entries.push(JournalEntry {
            tick,
            command: command.to_string(),
            params: params.clone(),
            end_tick: (command == "sim.step").then_some(after),
            hash: Some(hash.clone()),
        });
        journal.end = Some(JournalEnd { tick: after, hash });
    }

    fn journal_mark(&self) -> JournalMark {
        self.journal
            .as_ref()
            .map(|j| (j.entries.len(), j.end.clone(), j.truncated))
    }

    /// The document being edited.
    pub fn document(&self) -> &World {
        &self.document
    }

    /// The running simulation while playing.
    pub fn live(&self) -> Option<&Sim> {
        self.live.as_ref()
    }

    /// Whether a simulation runs.
    pub fn is_playing(&self) -> bool {
        self.live.is_some()
    }

    /// The running world while playing, else the document.
    pub fn current_world(&self) -> &World {
        match &self.live {
            Some(sim) => sim.world(),
            None => &self.document,
        }
    }

    /// The height field of [`Session::current_world`], `None` without terrain. Derived and
    /// cached: rebuilt only when the terrain changes.
    pub fn current_heights(&self) -> Option<&HeightField> {
        match &self.live {
            Some(sim) => sim.heights(),
            None => self.heights.field.as_deref(),
        }
    }

    /// Grows with every change of the document (commands, undo, redo, `sim.stop` with
    /// `keep`, `replace_document`).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Grows with every change of what [`Session::current_world`] shows: document edits,
    /// `sim.start`, `sim.stop`, steps and World commands on the running world. A renderer's
    /// cache key.
    pub fn view_revision(&self) -> u64 {
        self.view_revision
    }

    /// The specs of all commands, sorted by name.
    pub fn specs(&self) -> Vec<&CommandSpec> {
        self.commands.values().map(|c| &c.spec).collect()
    }

    /// The spec of one command.
    pub fn spec(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name).map(|c| &c.spec)
    }

    /// Runs a command by id with JSON parameters (an object, or null for none).
    ///
    /// The parameters are checked against the spec first. A World command runs on a copy of
    /// [`Session::current_world`]; the result is validated, then it becomes the document (one
    /// undo step) or, while playing, the running world. On any error nothing changes, and the
    /// message starts with the command id.
    pub fn run(&mut self, command: &str, params: &Value) -> Result<Value, CommandError> {
        let Some(entry) = self.commands.get(command) else {
            let hint = nearest(command, self.commands.keys().map(String::as_str))
                .map(|n| format!("; did you mean `{n}`?"))
                .unwrap_or_else(|| "; list the commands to see them all".to_string());
            return Err(CommandError::Unknown(format!(
                "unknown command `{}`{hint}",
                crate::spec::excerpt(command)
            )));
        };
        let map = entry
            .spec
            .check(params)
            .map_err(|why| CommandError::BadParams(why).for_command(command))?;
        let recorded = crate::journal::records(command, entry.spec.scope);
        let handler = entry.handler;
        let outer = self.depth == 0;
        let tick = self.live.as_ref().map(Sim::tick);
        self.depth = self.depth.saturating_add(1);
        let outcome = match handler {
            Handler::World(f) => self.run_world(f, Params::new(map)),
            Handler::Query(f) => f(self, Params::new(map)),
            Handler::Play(f) => f(self, Params::new(map)),
        };
        self.depth = self.depth.saturating_sub(1);
        if outer
            && recorded
            && outcome.is_ok()
            && let Some(tick) = tick
        {
            self.record(command, params, tick);
        }
        outcome.map_err(|e| e.for_command(command))
    }

    fn run_world(&mut self, f: WorldFn, params: Params<'_>) -> Result<Value, CommandError> {
        let mut edit = Edit {
            world: self.current_world().clone(),
            heights: self.current_heights(),
            built: None,
        };
        let result = f(&mut edit, params)?;
        let Edit { world, built, .. } = edit;
        world.validate().map_err(failed)?;
        match &mut self.live {
            Some(sim) => {
                if world != *sim.world() {
                    let changed = crate::triggers::changed_vars(&sim.world().vars, &world.vars);
                    sim.replace_world(world).map_err(failed)?;
                    self.view_revision += 1;
                    self.play.pending_vars.extend(changed);
                }
            }
            None => {
                if world != self.document {
                    self.set_document(world, built);
                }
            }
        }
        Ok(result)
    }

    /// Makes `world` the document as one undo step (joined into the transaction while one
    /// runs). `built` is its height field when the caller has one.
    pub(crate) fn set_document(&mut self, world: World, built: Option<HeightField>) {
        let previous = std::mem::replace(&mut self.document, world);
        if !self.in_transaction {
            self.push_undo(previous);
            self.redo.clear();
        }
        self.revision += 1;
        self.view_revision += 1;
        self.refresh_heights(built);
    }

    fn push_undo(&mut self, world: World) {
        self.undo.push_back(world);
        while self.undo.len() > UNDO_LIMIT {
            self.undo.pop_front();
        }
    }

    /// Rebuilds the document's height field when its terrain changed: takes `built` when the
    /// handler made it, else rebuilds the tiles the change touches (a whole build for another
    /// grid or source).
    fn refresh_heights(&mut self, built: Option<HeightField>) {
        if self.heights.terrain == self.document.terrain {
            return;
        }
        let rebuilt = match (
            &self.document.terrain,
            &self.heights.terrain,
            &self.heights.field,
        ) {
            (Some(terrain), _, _) if built.is_some() => built.map(|f| (terrain, f)),
            (Some(terrain), Some(old), Some(field)) => {
                terrain.rebuild(old, field).ok().map(|r| (terrain, r.field))
            }
            _ => None,
        };
        self.heights = match rebuilt {
            Some((terrain, field)) => HeightCache {
                terrain: Some(terrain.clone()),
                field: Some(Arc::new(field)),
            },
            None => HeightCache::for_world(&self.document),
        };
    }

    /// The height field of the document (not of a running world), shared: what `sim.start`
    /// hands the simulation so it does not build the terrain again.
    pub(crate) fn document_heights_shared(&self) -> Option<Arc<HeightField>> {
        self.heights.field.clone()
    }

    /// The height field of the document (not of a running world), `None` without terrain.
    /// A host writes the document's terrain tile cache from it.
    pub fn document_heights(&self) -> Option<&HeightField> {
        self.heights.field.as_deref()
    }

    /// The running simulation, to change.
    pub(crate) fn live_mut(&mut self) -> Option<&mut Sim> {
        self.live.as_mut()
    }

    /// Starts playing `sim`, with an empty play log and a new journal of `world`, the
    /// document it started from.
    pub(crate) fn start_live(&mut self, sim: Sim, world: World) {
        self.live = Some(sim);
        self.play = PlayLog::default();
        self.view_revision += 1;
        let journal = Some(Journal::new(world));
        let old = std::mem::replace(&mut self.journal, journal);
        if self.in_transaction && self.journal_stash.is_none() {
            self.journal_stash = Some(old);
        }
    }

    /// Marks the journal's end at the simulation's current tick and hash (after `sim.start`
    /// and its triggers).
    pub(crate) fn mark_journal_start(&mut self) {
        if let (Some(sim), Some(journal)) = (&self.live, &mut self.journal) {
            journal.end = Some(JournalEnd {
                tick: sim.tick(),
                hash: hash_text(sim.state_hash()),
            });
        }
    }

    /// Stops playing and hands back the simulation; the play log is cleared.
    pub(crate) fn take_live(&mut self) -> Option<Sim> {
        let sim = self.live.take();
        if sim.is_some() {
            self.view_revision += 1;
        }
        self.play = PlayLog::default();
        sim
    }

    /// Puts back a running simulation saved before (a trigger that failed). Nothing happens
    /// when `sim` is `None`.
    pub(crate) fn restore_live(&mut self, sim: Option<Sim>) {
        if sim.is_some() {
            self.live = sim;
        }
    }

    /// The play log.
    pub(crate) fn play(&self) -> &PlayLog {
        &self.play
    }

    /// The play log, to change.
    pub(crate) fn play_mut(&mut self) -> &mut PlayLog {
        &mut self.play
    }

    /// What the running world shows changed (a step).
    pub(crate) fn bump_view(&mut self) {
        self.view_revision += 1;
    }

    /// Whether [`Session::undo`] has a step to take back.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether [`Session::redo`] has a step to do again.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn check_history(&self, what: &str) -> Result<(), CommandError> {
        if self.live.is_some() {
            return Err(failed(format!(
                "{what}: the document cannot change while a simulation runs, and a running \
                 world has no history; stop it first with sim.stop"
            )));
        }
        if self.in_transaction {
            return Err(failed(format!(
                "{what}: not inside a transaction; the transaction is one step as a whole"
            )));
        }
        Ok(())
    }

    /// Takes back the last change of the document. Refused while playing (a running world has
    /// no history) and inside a transaction; an error when there is nothing to undo.
    pub fn undo(&mut self) -> Result<(), CommandError> {
        self.check_history("undo")?;
        let Some(previous) = self.undo.pop_back() else {
            return Err(failed("undo: there is nothing to undo"));
        };
        let current = std::mem::replace(&mut self.document, previous);
        self.redo.push(current);
        self.revision += 1;
        self.view_revision += 1;
        self.refresh_heights(None);
        Ok(())
    }

    /// Does again what [`Session::undo`] took back. Refused while playing and inside a
    /// transaction; an error when there is nothing to redo.
    pub fn redo(&mut self) -> Result<(), CommandError> {
        self.check_history("redo")?;
        let Some(next) = self.redo.pop() else {
            return Err(failed("redo: there is nothing to redo"));
        };
        let current = std::mem::replace(&mut self.document, next);
        self.push_undo(current);
        self.revision += 1;
        self.view_revision += 1;
        self.refresh_heights(None);
        Ok(())
    }

    /// [`Session::replace_document`] with the height field of `world`'s terrain, which the
    /// host built (from a tile cache): it must be what the terrain builds to, and is ignored
    /// when the world has no terrain.
    pub fn replace_document_with_heights(&mut self, world: World, heights: HeightField) {
        self.heights = HeightCache {
            terrain: world.terrain.clone(),
            field: world.terrain.as_ref().map(|_| Arc::new(heights)),
        };
        self.replace_document(world);
    }

    /// Hands over the bounding boxes of the model assets (by asset path), which the host
    /// derived from the assets' bytes: they replace those of the document, of every world in
    /// the history and of the running world, where solid models start or stop blocking from
    /// the next tick. Derived data: neither revision changes and nothing is undone by it.
    pub fn set_model_bounds(&mut self, bounds: BTreeMap<String, ModelBounds>) {
        self.document.set_model_bounds(bounds.clone());
        for world in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            world.set_model_bounds(bounds.clone());
        }
        if let Some(sim) = self.live.as_mut() {
            let mut running = sim.world().clone();
            running.set_model_bounds(bounds);
            // The running world was valid and only its derived data changed, so this cannot
            // fail; if it did, the simulation would keep the bounds it had.
            let _ = sim.replace_world(running);
        }
    }

    /// A host opened a file or started a new world: stops any simulation, makes `world` the
    /// document and clears the history (inside a transaction: when it succeeds).
    pub fn replace_document(&mut self, world: World) {
        self.live = None;
        self.play = PlayLog::default();
        self.document = world;
        if self.in_transaction {
            self.cleared_in_transaction = true;
        } else {
            self.undo.clear();
            self.redo.clear();
        }
        self.revision += 1;
        self.view_revision += 1;
        self.refresh_heights(None);
    }

    /// Runs `f`, which may run any commands; all or nothing.
    ///
    /// On `Err` the document, the history, the play state and both revisions are exactly as
    /// before. On `Ok` the document changes of the whole closure are one undo step (none when
    /// the document ended as it was). A transaction inside a transaction joins the outer one;
    /// `undo` and `redo` are refused inside one.
    pub fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut Session) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        if self.in_transaction {
            return f(self);
        }
        let document = self.document.clone();
        let live = self.live.clone();
        let heights = self.heights.clone();
        let play = self.play.clone();
        let mark = self.journal_mark();
        self.journal_stash = None;
        let (revision, view_revision) = (self.revision, self.view_revision);
        self.in_transaction = true;
        self.cleared_in_transaction = false;
        let outcome = f(self);
        self.in_transaction = false;
        let cleared = std::mem::take(&mut self.cleared_in_transaction);
        let stash = self.journal_stash.take();
        match outcome {
            Ok(value) => {
                if cleared {
                    self.undo.clear();
                    self.redo.clear();
                } else if self.document != document {
                    self.push_undo(document);
                    self.redo.clear();
                }
                Ok(value)
            }
            Err(e) => {
                self.document = document;
                self.live = live;
                self.heights = heights;
                self.play = play;
                match stash {
                    Some(old) => self.journal = old,
                    None => {
                        if let (Some(j), Some((len, end, truncated))) = (&mut self.journal, mark) {
                            j.entries.truncate(len);
                            j.end = end;
                            j.truncated = truncated;
                        }
                    }
                }
                self.revision = revision;
                self.view_revision = view_revision;
                Err(e)
            }
        }
    }
}
