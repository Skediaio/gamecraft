//! Files on disk (not in the browser): paths kept inside their root, documents read with a
//! size cap, and writes that never leave half a file.
//!
//! The root rule is VisionCraft's `Access::resolve` (`crates/visioncraft-automation/src/host.rs`,
//! MIT), adapted: the root must be set (there is no unrestricted mode), and the existing parent
//! folder is resolved with its links, so neither `..`, an absolute path nor a link leads out.

use crate::host::Access;
use gamecraft_world::World;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Largest `.gamecraft` file `file.open` reads, bytes.
pub(crate) const MAX_OPEN_BYTES: u64 = 64 * 1024 * 1024;

/// Which root a path is resolved against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    /// The read root (`file.open`).
    Read,
    /// The write root (`file.save`, `view.render` with a path).
    Write,
}

impl Direction {
    fn verb(self) -> &'static str {
        match self {
            Direction::Read => "read",
            Direction::Write => "write",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Direction::Read => "--automation-read-root",
            Direction::Write => "--automation-write-root",
        }
    }
}

/// Where `given` is on disk, inside the root of `direction`, with the extension `extension`.
///
/// Refused, each with a sentence that says why: no root set, an empty path, `..`, a path or a
/// link that leads out of the root, a folder that does not exist, another extension. The
/// answer is the resolved folder joined with the file name; for reading, a link at the file
/// itself is resolved too.
pub(crate) fn resolve(
    access: &Access,
    given: &str,
    direction: Direction,
    extension: &str,
) -> Result<PathBuf, String> {
    let verb = direction.verb();
    let root = match direction {
        Direction::Read => &access.read_root,
        Direction::Write => &access.write_root,
    };
    let Some(root) = root else {
        return Err(format!(
            "this server may not {verb} files: no {verb} root is set; start it with {} DIR",
            direction.flag()
        ));
    };
    if given.trim().is_empty() {
        return Err(format!(
            "`path` is empty; give a file name inside the {verb} root"
        ));
    }
    let path = Path::new(given);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "`{given}`: `..` is not allowed in a path; give a path inside the {verb} root"
        ));
    }
    if path.extension().and_then(|e| e.to_str()) != Some(extension) {
        return Err(format!(
            "`{given}`: the file must have the extension .{extension}"
        ));
    }
    let real_root = fs::canonicalize(root)
        .map_err(|e| format!("the {verb} root `{}` cannot be used: {e}", root.display()))?;
    let joined = if path.is_relative() {
        real_root.join(path)
    } else {
        path.to_path_buf()
    };
    let outside = || {
        format!(
            "`{given}` is outside the {verb} root `{}`; give a path inside it",
            root.display()
        )
    };
    let (Some(name), Some(parent)) = (joined.file_name(), joined.parent()) else {
        return Err(format!("`{given}` names no file"));
    };
    let real_parent = fs::canonicalize(parent).map_err(|_| {
        if parent.starts_with(&real_root) || parent.starts_with(root) {
            format!(
                "`{given}`: the folder `{}` does not exist; create it first or save beside it",
                parent.display()
            )
        } else {
            outside()
        }
    })?;
    if !real_parent.starts_with(&real_root) {
        return Err(outside());
    }
    let target = real_parent.join(name);
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        let real = fs::canonicalize(&target).map_err(|_| outside())?;
        if !real.starts_with(&real_root) {
            return Err(outside());
        }
        if direction == Direction::Read {
            return Ok(real);
        }
    }
    Ok(target)
}

/// Reads and checks the `.gamecraft` document at `path` (`given` is how the request named it).
pub(crate) fn read_world(path: &Path, given: &str) -> Result<World, String> {
    let meta = fs::metadata(path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
    if meta.is_dir() {
        return Err(format!("`{given}` is a folder, not a .gamecraft file"));
    }
    if meta.len() > MAX_OPEN_BYTES {
        return Err(format!(
            "`{given}` is {} MB; a world file may be at most {} MB",
            meta.len().div_ceil(1024 * 1024),
            MAX_OPEN_BYTES / (1024 * 1024)
        ));
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("`{given}` is not UTF-8 text, so it is no .gamecraft file"))?;
    World::from_json(&text).map_err(|e| format!("`{given}` is not a valid world: {e}"))
}

/// Writes `bytes` to `target` through a temporary file beside it and a rename, so a failed
/// write never leaves half a file. The temporary file is removed on failure.
pub(crate) fn write_whole(target: &Path, bytes: &[u8]) -> Result<(), String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = target.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, target));
    written.map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot write `{}`: {e}", target.display())
    })
}
