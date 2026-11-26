//! What the servers run on: a [`Host`] is an engine [`Session`] plus what the session does not
//! hold: the open file, the folders automation may touch and, when the program has a GPU,
//! pictures.
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/host.rs` (MIT) and adapted:
//! GameCraft's host is one struct, not a trait over a window; there are no models, images or
//! catalogue; a renderer is handed the world, its height field and the view revision.

use gamecraft_engine::Session;
use gamecraft_project::Project;
use gamecraft_world::{EntityId, HeightField, World};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The default of [`Access::max_world_bytes`]: the largest world file `file.open` reads (and
/// file of a project's pack), bytes. A safety net against a file that would exhaust memory,
/// not a limit of the design: a host raises it (`gamecraft-cli --max-world-mb`).
pub const DEFAULT_MAX_WORLD_BYTES: u64 = 512 * 1024 * 1024;

/// The folders automation may read from and write to. A relative path is inside the root; an
/// absolute one must lie inside it. `None` refuses that kind of file access, with a sentence
/// that names the command-line flag that sets it (`--automation-read-root`,
/// `--automation-write-root`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Access {
    /// Where `file.open` and `project.open` read.
    pub read_root: Option<PathBuf>,
    /// Where `file.save`, the `project.*` commands that write and `view.render` with a
    /// `path` write.
    pub write_root: Option<PathBuf>,
    /// The largest world file `file.open` reads, and the largest file of a project's pack,
    /// bytes; `None` is [`DEFAULT_MAX_WORLD_BYTES`] (512 MiB). A safety net, not a
    /// limit of the design (entity counts are bounded by the world itself). Models and
    /// textures keep their own cap of 64 MiB.
    pub max_world_bytes: Option<u64>,
    /// Write the cooked cache of a world file (`.gamecraft-cache/` beside it, derived data)
    /// even where the write root does not reach: a host that owns its files (the command
    /// line) sets it. Inside the write root the cache is written anyway.
    pub cache_beside_files: bool,
}

impl Access {
    /// The cap on world files and pack files in effect, bytes.
    pub fn max_world_bytes(&self) -> u64 {
        self.max_world_bytes.unwrap_or(DEFAULT_MAX_WORLD_BYTES)
    }
}

/// Where a picture is taken from.
#[derive(Clone, Debug, PartialEq)]
pub enum CameraRequest {
    /// The whole world framed, from a default angle chosen by the renderer.
    Framing,
    /// A camera at `position` looking at `target`, metres, with a vertical field of view.
    LookAt {
        /// Where the camera is, `[x, y, z]`.
        position: [f64; 3],
        /// What it looks at, `[x, y, z]`; never equal to `position`.
        target: [f64; 3],
        /// Vertical field of view, degrees, 1 to 179.
        fov_y_deg: f64,
    },
    /// Through the eyes of an entity (a character's `eye_height_m`), looking where it faces.
    FirstPerson {
        /// The entity followed.
        entity: EntityId,
    },
    /// Behind and above an entity, looking at it in the direction it faces.
    ThirdPerson {
        /// The entity followed.
        entity: EntityId,
        /// Distance from the entity, metres.
        distance_m: f64,
    },
}

/// What `view.render` asks a renderer for.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderRequest {
    /// Picture width, pixels, 16 to 4096.
    pub width: u32,
    /// Picture height, pixels, 16 to 4096.
    pub height: u32,
    /// Where the picture is taken from.
    pub camera: CameraRequest,
    /// The animation clock, seconds: wind and water move with it. The simulation's time while
    /// playing unless the request gives one, else 0.
    pub time_s: f64,
}

/// A picture: PNG bytes and what the renderer says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    /// The encoded PNG.
    pub png: Vec<u8>,
    /// What the renderer says about the picture (the camera it used, counts of what it drew,
    /// problems), merged into the answer of `view.render`. Best an object.
    pub info: Value,
}

/// Draws pictures for a [`Host`] (the CLI's is `gamecraft-render` on a headless GPU device).
pub trait Pictures {
    /// Draws `world` with its height field (`None` without terrain). `view_revision` changes
    /// whenever the world shown changes: a cache key.
    fn render(
        &mut self,
        world: &World,
        heights: Option<&HeightField>,
        view_revision: u64,
        request: &RenderRequest,
    ) -> Result<Rendered, String>;

    /// Hands over the bytes of an asset file the world's models need, by its path inside a
    /// pack's `assets/` folder (`models/crate.glb`; a `.gltf`'s buffers and images under their
    /// own paths). The host calls it before [`Pictures::render`] for every model asset the
    /// world shows that it has not handed over yet. The default keeps nothing.
    fn provide_asset(&mut self, path: &str, bytes: Vec<u8>) {
        let _ = (path, bytes);
    }
}

impl Host {
    /// The sentence for models no pack provides ([`crate::assets`]): none in the browser,
    /// where the models of a world are fetched with it and the renderer says which were not.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn missing_pack_line(&self) -> Option<String> {
        None
    }
}

/// The SQL index of an open project (the CLI's is `gamecraft-index`, on Turso): rebuilt from
/// the world, queried with one read-only `SELECT`.
pub trait ProjectIndex {
    /// Replaces every row with the entities and prefabs of `world`, with their string ids,
    /// layers and packs from `project`. Answers what it wrote, as JSON.
    fn rebuild(&mut self, world: &World, project: &Project) -> Result<Value, String>;
    /// Runs one read-only `SELECT` and answers `{columns, rows, truncated}` with at most
    /// `max_rows` rows.
    fn query(&mut self, sql: &str, max_rows: usize) -> Result<Value, String>;
    /// Where the index is stored (a file path), or `None` in memory.
    fn location(&self) -> Option<String>;
}

/// Opens project indexes for a [`Host`].
pub trait Indexer {
    /// Opens the index of the project in `project_dir` (stored beside it, in
    /// `.gamecraft-index/`), or in memory when `project_dir` is `None` (the project's folder
    /// is not writable).
    fn open(&mut self, project_dir: Option<&Path>) -> Result<Box<dyn ProjectIndex>, String>;
}

/// The session the servers drive, the open file, the folders they may touch and the pictures.
pub struct Host {
    pub(crate) session: Session,
    /// The `.gamecraft` file the document was opened from or last saved to.
    pub(crate) path: Option<PathBuf>,
    /// The session's revision when the document was last opened, saved or started anew.
    pub(crate) saved_revision: u64,
    pub(crate) access: Access,
    pub(crate) pictures: Option<Box<dyn Pictures>>,
    /// The open project, if the document came from one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) project: Option<crate::projects::OpenProject>,
    /// Opens the index of a project; without one, `project.query` says an index is needed.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) indexer: Option<Box<dyn Indexer>>,
    /// The assets of the open project, or of the pack the open world file found
    /// (`file.open`): the models measured. Empty without either.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) assets: crate::assets::Catalog,
    /// The model assets whose files the renderer was given since the assets were set.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) provided: std::collections::BTreeSet<String>,
    /// The folder of the pack whose assets are open: the open project's root pack, or the
    /// pack of the open world file.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pack: Option<PathBuf>,
    /// The pack a world file takes its assets from when `file.open` names none (the host's
    /// `--pack DIR`); it overrides the pack above the file too.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) default_pack: Option<PathBuf>,
}

impl Host {
    /// A host editing `world`, with no open file and no renderer: `view.render` answers with an
    /// error that says a renderer is needed.
    pub fn new(world: World, access: Access) -> Host {
        let session = Session::new(world);
        Host {
            saved_revision: session.revision(),
            session,
            path: None,
            access,
            pictures: None,
            #[cfg(not(target_arch = "wasm32"))]
            project: None,
            #[cfg(not(target_arch = "wasm32"))]
            indexer: None,
            #[cfg(not(target_arch = "wasm32"))]
            assets: std::collections::BTreeMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            provided: std::collections::BTreeSet::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pack: None,
            #[cfg(not(target_arch = "wasm32"))]
            default_pack: None,
        }
    }

    /// The same host, taking the assets of a world file from the pack in `dir` when
    /// `file.open` names no `pack`: where a command line's `--pack DIR` lands. The folder
    /// must lie inside the read root when a world file is opened.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_pack(mut self, dir: PathBuf) -> Host {
        self.default_pack = Some(dir);
        self
    }

    /// The folders automation may touch from now on (a host that opened a world file with a
    /// wider reach, to hand its pack over, narrows itself to what a server was started with).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_access(&mut self, access: Access) {
        self.access = access;
    }

    /// The same host, indexing projects with `indexer` (`project.query`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_indexer(mut self, indexer: Box<dyn Indexer>) -> Host {
        self.indexer = Some(indexer);
        self
    }

    /// The same host; in the browser there is no index, and the indexer is not kept.
    #[cfg(target_arch = "wasm32")]
    pub fn with_indexer(self, indexer: Box<dyn Indexer>) -> Host {
        drop(indexer);
        self
    }

    /// The open project and its folder, if the document came from one.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn project(&self) -> Option<(&Project, &Path)> {
        self.project.as_ref().map(|p| (&p.project, p.dir.as_path()))
    }

    /// The same host, drawing pictures with `pictures`.
    pub fn with_pictures(mut self, pictures: Box<dyn Pictures>) -> Host {
        self.pictures = Some(pictures);
        self
    }

    /// The session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session, to change.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// The `.gamecraft` file the document was opened from or last saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether the document changed since it was opened, saved or started anew. Undoing back
    /// to the saved state still counts as a change.
    pub fn is_dirty(&self) -> bool {
        self.session.revision() != self.saved_revision
    }
}
