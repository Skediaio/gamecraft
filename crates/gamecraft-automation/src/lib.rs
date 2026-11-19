//! Automation for GameCraft (layer L4): the JSON-lines protocol and the MCP server over one
//! [`Host`] (a session, the open file, the folders it may touch and optional pictures), so
//! scripts and AI agents drive everything the engine can do (`docs/control-protocol.md`,
//! `docs/agent.md`).
//!
//! Ported from VisionCraft's `crates/visioncraft-automation/src/lib.rs` (MIT), itself modelled
//! on 3DCraft's automation crate: the same envelope, method names, tool names, limits and error
//! conventions. Each ported file names its origin at the top.
//!
//! - [`rpc`]: `{"id", "method", "params"}` in, `{"id", "ok", "result" | "error"}` out.
//! - [`mcp`]: the Model Context Protocol (JSON-RPC 2.0, one message per line).
//! - [`Host`]: what both run: the engine's commands plus the host's own (`file.*`,
//!   `project.*`, `pack.*`, `edit.*`, `view.render`), one namespace and one listing
//!   ([`Host::command_docs`]), and batches.
//!
//! Each server is a function from one input line to at most one reply line (`handle_line`),
//! plus a loop over a reader and a writer (`serve`). No line, however malformed, makes a server
//! panic or stop. The file system is used only outside the browser: on `wasm32` there are no
//! `file.*`, `project.*` or `pack.*` commands (but `project.query`, which says the index is
//! native only), and `view.render` cannot write a file.

mod commands;
pub mod docs;
mod fields;
#[cfg(not(target_arch = "wasm32"))]
mod files;
mod host;
mod lines;
pub mod mcp;
#[cfg(not(target_arch = "wasm32"))]
mod projects;
pub mod rpc;
#[cfg(not(target_arch = "wasm32"))]
mod terrain_cache;

pub use docs::{CommandDoc, ParamDoc};
pub use host::{
    Access, CameraRequest, Host, Indexer, Pictures, ProjectIndex, RenderRequest, Rendered,
};
pub use lines::{Line, read_line};

/// Longest request line a server reads, in bytes (as 3DCraft and VisionCraft).
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Longest reply line a server writes, in bytes.
pub const MAX_REPLY_BYTES: usize = 8 << 20;
/// Most steps in one batch.
pub const MAX_BATCH_STEPS: usize = 256;

/// Standard base64 with padding (RFC 4648), for PNGs in replies.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                let index = ((n >> shift) & 63) as usize;
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'=')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc_4648() {
        for (raw, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(super::base64(raw.as_bytes()), coded);
        }
    }
}
