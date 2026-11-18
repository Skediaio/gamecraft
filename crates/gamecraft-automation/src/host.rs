//! What the servers run on: a [`Host`] is an engine [`Session`] plus what the session does not
//! hold: the open file, the folders automation may touch and, when the program has a GPU,
//! pictures.
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/host.rs` (MIT) and adapted:
//! GameCraft's host is one struct, not a trait over a window; there are no models, images or
//! catalogue; a renderer is handed the world, its height field and the view revision.

use gamecraft_engine::Session;
use gamecraft_world::{EntityId, HeightField, World};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The folders automation may read from and write to. A relative path is inside the root; an
/// absolute one must lie inside it. `None` refuses that kind of file access, with a sentence
/// that names the command-line flag that sets it (`--automation-read-root`,
/// `--automation-write-root`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Access {
    /// Where `file.open` reads.
    pub read_root: Option<PathBuf>,
    /// Where `file.save` and `view.render` with a `path` write.
    pub write_root: Option<PathBuf>,
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
        }
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
