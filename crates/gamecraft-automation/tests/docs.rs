//! The protocol document cannot drift from the commands: every command has a row in a table of
//! `docs/control-protocol.md`, and the parameters the row lists are the parameters of its spec.
//!
//! Adapted from VisionCraft's `crates/visioncraft-automation/tests/docs.rs` (MIT).

// Test code: clippy.toml allows these inside #[test] functions only, not in the helpers of an
// integration test crate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

const PROTOCOL: &str = include_str!("../../../docs/control-protocol.md");

/// The cells of a markdown table row (an escaped `\|` is part of a cell).
fn cells(row: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = row.trim().trim_start_matches('|').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                out.last_mut().unwrap().push_str("\\|");
                chars.next();
            }
            '|' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

/// The parameter names in a cell like `` `{name? = "", position?, at?}` ``: the leading word of
/// each top-level item.
fn names(cell: &str) -> Option<Vec<String>> {
    let body = cell.strip_prefix("`{")?;
    let body = &body[..body.rfind("}`")?];
    let mut items = Vec::new();
    let (mut depth, mut quoted, mut cur) = (0i32, false, String::new());
    for c in body.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '[' | '{' | '(' if !quoted => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' | ')' if !quoted => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 && !quoted => items.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    items.push(cur);
    let mut out: Vec<String> = items
        .iter()
        .map(|item| {
            item.trim()
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    out.sort();
    out.dedup();
    Some(out)
}

#[test]
fn every_command_has_a_row_with_the_params_of_its_spec() {
    let host = common::host();
    let docs = host.command_docs();
    let mut drift = Vec::new();
    for doc in &docs {
        let row = PROTOCOL.lines().find(|l| {
            l.starts_with("| `")
                && cells(l)[0]
                    .split(", ")
                    .any(|id| id.trim_matches('`') == doc.id)
        });
        let Some(row) = row else {
            drift.push(format!("`{}` has no row", doc.id));
            continue;
        };
        let Some(listed) = cells(row).get(1).and_then(|c| names(c)) else {
            drift.push(format!("`{}`: the params cell is not `{{...}}`", doc.id));
            continue;
        };
        let mut have: Vec<String> = doc.params.iter().map(|p| p.name.clone()).collect();
        have.sort();
        if listed != have {
            drift.push(format!(
                "`{}`: the row lists {listed:?}, the spec has {have:?}",
                doc.id
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "docs/control-protocol.md drifted from the specs:\n{}",
        drift.join("\n")
    );
}

#[test]
fn the_document_names_the_methods_tools_and_limits() {
    for word in [
        "engine.execute",
        "engine.commands",
        "`batch`",
        "`methods`",
        "command_list",
        "command_run",
        "command_batch",
        "doc_inspect",
        "doc_render_preview",
        "gamecraft://world",
        "gamecraft://guide",
        "build_world",
        "1048576",
        "8388608",
        "256",
        "Differences from VisionCraft and 3DCraft",
    ] {
        assert!(
            PROTOCOL.contains(word),
            "docs/control-protocol.md lacks {word}"
        );
    }
}
