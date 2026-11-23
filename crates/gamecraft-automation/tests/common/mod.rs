//! Helpers the tests share: hosts, a request in and the reply out, a temporary folder, and a
//! fake renderer that echoes what it was asked.

#![allow(dead_code)]

use gamecraft_automation::{Access, Host, Pictures, RenderRequest, Rendered, mcp, rpc};
use gamecraft_world::{HeightField, World};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A host on the engine's example world, without file access or pictures.
pub fn host() -> Host {
    Host::new(gamecraft_engine::example_world(), Access::default())
}

/// A host on an empty world, without file access or pictures.
pub fn empty_host() -> Host {
    Host::new(World::new("Untitled", 0), Access::default())
}

/// The reply of the JSON-lines server to one request.
pub fn ask(host: &mut Host, request: Value) -> Value {
    let line = rpc::handle_line(host, &request.to_string()).expect("a reply");
    serde_json::from_str(&line).expect("the reply is JSON")
}

/// `engine.execute` of one command.
pub fn execute(host: &mut Host, command: &str, params: Value) -> Value {
    ask(
        host,
        json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}),
    )
}

/// The result of a command that must succeed.
pub fn ok(host: &mut Host, command: &str, params: Value) -> Value {
    let reply = execute(host, command, params);
    assert_eq!(reply["ok"], true, "{command}: {reply}");
    reply["result"].clone()
}

/// The error of a command that must fail.
pub fn err(host: &mut Host, command: &str, params: Value) -> String {
    let reply = execute(host, command, params);
    assert_eq!(reply["ok"], false, "{command} should fail: {reply}");
    reply["error"].as_str().expect("an error text").to_string()
}

/// The replies of the JSON-lines server to a stream of lines.
pub fn serve_rpc(host: &mut Host, input: &str) -> Vec<String> {
    let mut output = Vec::new();
    rpc::serve(host, Cursor::new(input.as_bytes().to_vec()), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The replies of the MCP server to a list of messages.
pub fn converse(host: &mut Host, messages: &[Value]) -> Vec<Value> {
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let mut output = Vec::new();
    mcp::serve(host, Cursor::new(input.into_bytes()), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// A folder of its own under the system's temporary folder, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!(
            "gamecraft-automation-{tag}-{}-{n}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A host that may read and write in `dir`.
pub fn host_in(dir: &TempDir) -> Host {
    Host::new(
        gamecraft_engine::example_world(),
        Access {
            read_root: Some(dir.path().to_path_buf()),
            write_root: Some(dir.path().to_path_buf()),
            max_world_bytes: None,
            cache_beside_files: false,
        },
    )
}

/// A transparent PNG of 1 x 1 pixels.
pub const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// What the fake renderer was handed.
pub struct Seen {
    pub request: RenderRequest,
    pub world: World,
    pub has_heights: bool,
    pub view_revision: u64,
}

/// Returns [`PNG_1X1`] and remembers every request.
pub struct FakePictures {
    pub seen: Rc<RefCell<Vec<Seen>>>,
}

impl Pictures for FakePictures {
    fn render(
        &mut self,
        world: &World,
        heights: Option<&HeightField>,
        view_revision: u64,
        request: &RenderRequest,
    ) -> Result<Rendered, String> {
        self.seen.borrow_mut().push(Seen {
            request: request.clone(),
            world: world.clone(),
            has_heights: heights.is_some(),
            view_revision,
        });
        Ok(Rendered {
            png: PNG_1X1.to_vec(),
            info: json!({"renderer": "fake", "entities": world.entity_count()}),
        })
    }
}

/// A host with the fake renderer, and what it is handed.
pub fn host_with_pictures(host: Host) -> (Host, Rc<RefCell<Vec<Seen>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let host = host.with_pictures(Box::new(FakePictures { seen: seen.clone() }));
    (host, seen)
}
