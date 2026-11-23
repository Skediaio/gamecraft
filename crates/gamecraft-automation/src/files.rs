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

/// Largest asset file (a model, a texture) read, bytes.
pub(crate) const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

/// How a host raises the world-file cap, for error messages.
pub(crate) const RAISE_CAP: &str = "it is a safety net; raise it with `--max-world-mb` (or \
    `GAMECRAFT_MAX_WORLD_MB`) on gamecraft-cli, or `Access::max_world_bytes` in a host";

/// Mebibytes of `bytes`, rounded up.
pub(crate) fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

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

/// The folder beside a world file that holds its cooked cache: derived data, ignored by git
/// (it holds a `.gitignore` of `*`) and safe to delete.
pub const CACHE_DIR: &str = ".gamecraft-cache";

/// Where the cooked cache of the world file `path` is: `<folder>/.gamecraft-cache/<name>.cooked`.
pub(crate) fn cache_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some(
        path.parent()?
            .join(CACHE_DIR)
            .join(format!("{name}.cooked")),
    )
}

/// Whether `access` lets a cooked cache be written beside the world file `path`.
pub(crate) fn may_cache(access: &Access, path: &Path) -> bool {
    access.cache_beside_files
        || path
            .parent()
            .and_then(|p| fs::canonicalize(p).ok())
            .is_some_and(|dir| crate::projects::writable(access, &dir))
}

/// Writes the cooked form of `world`, read from (or saved as) source bytes with hash `source`,
/// as the cache of `path`. Returns its size; a failure is reported, never fatal.
pub(crate) fn write_cache(path: &Path, world: &World, source: u64) -> Result<u64, String> {
    let target = cache_path(path).ok_or("the file has no folder for a cache")?;
    let bytes = world.to_cooked(source).map_err(|e| e.to_string())?;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create `{}`: {e}", dir.display()))?;
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            write_whole(
                &ignore,
                b"# Derived data (cooked worlds): never commit.\n*\n",
            )?;
        }
    }
    write_whole(&target, &bytes)?;
    Ok(bytes.len() as u64)
}

/// Reads and checks the `.gamecraft` document at `path` (`given` is how the request named it),
/// refusing a file of more than `cap` bytes. With `cache`, the cooked cache beside it is used
/// when it was made from these very bytes, and written after reading the text otherwise
/// (when `may_write`); a cache that is missing, stale, of another version or damaged is
/// ignored, never an error. Returns the world and what happened to the cache.
pub(crate) fn read_world_cached(
    path: &Path,
    given: &str,
    cap: u64,
    may_write: bool,
) -> Result<(World, serde_json::Value), String> {
    use serde_json::json;
    let clock = crate::commands::Stopwatch::start();
    let bytes = read_capped(path, given, cap)?;
    let source = gamecraft_world::cooked::source_hash(&bytes);
    let cache = cache_path(path);
    let mut ignored: Option<String> = None;
    if let Some(cache) = &cache {
        match fs::metadata(cache) {
            Ok(m) if m.len() > cap => ignored = Some("larger than the cap".into()),
            Ok(_) => match fs::read(cache) {
                Ok(cooked) => match World::from_cooked(&cooked, source) {
                    Ok(world) => {
                        let report = json!({
                            "used": true,
                            "path": cache.display().to_string(),
                            "bytes": cooked.len(),
                            "source_bytes": bytes.len(),
                            "ms": clock.ms(),
                        });
                        return Ok((world, report));
                    }
                    Err(e) => ignored = Some(e.to_string()),
                },
                Err(e) => ignored = Some(format!("cannot read it: {e}")),
            },
            Err(_) => ignored = Some("none yet".into()),
        }
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("`{given}` is not UTF-8 text, so it is no .gamecraft file"))?;
    let world =
        World::from_json(&text).map_err(|e| format!("`{given}` is not a valid world: {e}"))?;
    let read_ms = clock.ms();
    let written = if may_write {
        match write_cache(path, &world, source) {
            Ok(n) => json!(n),
            Err(e) => json!({"error": e}),
        }
    } else {
        serde_json::Value::Null
    };
    let report = json!({
        "used": false,
        "path": cache.map(|c| c.display().to_string()),
        "ignored": ignored,
        "written": written,
        "source_bytes": text.len(),
        "ms": read_ms,
    });
    Ok((world, report))
}

/// The bytes of the file at `path`, refusing a folder or more than `cap` bytes.
fn read_capped(path: &Path, given: &str, cap: u64) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
    if meta.is_dir() {
        return Err(format!("`{given}` is a folder, not a .gamecraft file"));
    }
    if meta.len() > cap {
        return Err(format!(
            "`{given}` is {} MB; this host reads world files of at most {} MB ({RAISE_CAP})",
            mb(meta.len()),
            mb(cap)
        ));
    }
    fs::read(path).map_err(|e| format!("cannot read `{given}`: {e}"))
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
