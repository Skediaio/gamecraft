//! The layer table and the layering check (`cargo xtask layers`).
//!
//! The logic is pure: it takes the parsed `cargo metadata` output and the manifest texts and
//! returns a list of violations, so the unit tests run on synthetic input.

use std::collections::BTreeMap;

use serde_json::Value;

/// The tool crate: no layer, depends on no workspace crate, and no workspace crate depends on it.
pub const TOOL: &str = "xtask";

/// Every workspace crate and its layer (AGENTS.md, section 2).
pub const LAYERS: &[(&str, u8)] = &[
    ("gamecraft-math", 0),
    ("gamecraft-world", 1),
    ("gamecraft-sim", 2),
    ("gamecraft-project", 2),
    ("gamecraft-engine", 3),
    ("gamecraft-render", 4),
    ("gamecraft-automation", 4),
    ("gamecraft-index", 4),
    ("gamecraft-cli", 5),
    ("gamecraft", 5),
];

/// Same-layer dependencies that are allowed, as (dependent, dependency). Empty today.
pub const SAME_LAYER_ALLOWED: &[(&str, &str)] = &[];

/// Highest layer that must build for wasm (`cargo xtask wasm`).
pub const WASM_MAX_LAYER: u8 = 4;

/// Who may depend on an external crate.
enum Allow {
    /// Only these workspace crates.
    Only(&'static [&'static str]),
    /// Only crates of this layer or higher.
    FromLayer(u8),
}

/// External crates that are restricted (normal and build dependencies).
const FORBIDDEN: &[(&str, Allow)] = &[
    ("visioncraft-render", Allow::Only(&["gamecraft-render"])),
    (
        "visioncraft-flora",
        Allow::Only(&["gamecraft-render", "gamecraft-engine"]),
    ),
    ("wgpu", Allow::FromLayer(4)),
    ("naga", Allow::FromLayer(4)),
    ("winit", Allow::FromLayer(5)),
    ("eframe", Allow::FromLayer(5)),
    ("egui", Allow::FromLayer(5)),
    ("egui-wgpu", Allow::FromLayer(5)),
    ("rfd", Allow::FromLayer(5)),
];

/// The layer of a registered crate.
pub fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Names of the crates of layers L0 up to and including `max`.
pub fn crates_up_to(max: u8) -> Vec<&'static str> {
    LAYERS
        .iter()
        .filter(|(_, l)| *l <= max)
        .map(|(n, _)| *n)
        .collect()
}

/// One workspace member as read from the metadata.
struct Member<'a> {
    name: &'a str,
    manifest_path: &'a str,
    /// Normal and build dependencies (by package name).
    deps: Vec<&'a str>,
}

fn members(metadata: &Value) -> Result<Vec<Member<'_>>, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("the cargo metadata has no `packages` array; run `cargo metadata --format-version 1 --no-deps` and check that it prints JSON")?;
    let mut out = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("a package in the cargo metadata has no string `name`")?;
        let manifest_path = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("package `{name}` has no string `manifest_path` in the cargo metadata")
            })?;
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("package `{name}` has no `dependencies` array in the cargo metadata")
            })?;
        let mut deps = Vec::new();
        for dep in dependencies {
            let dep_name = dep.get("name").and_then(Value::as_str).ok_or_else(|| {
                format!("a dependency of `{name}` has no string `name` in the cargo metadata")
            })?;
            let kind = match dep.get("kind") {
                None | Some(Value::Null) => None,
                Some(Value::String(kind)) => Some(kind.as_str()),
                Some(_) => {
                    return Err(format!(
                        "dependency `{dep_name}` of `{name}` has a `kind` that is neither null nor a string"
                    ));
                }
            };
            if kind != Some("dev") {
                deps.push(dep_name);
            }
        }
        out.push(Member {
            name,
            manifest_path,
            deps,
        });
    }
    Ok(out)
}

/// The manifest paths of all workspace members, for the caller to read.
pub fn manifest_paths(metadata: &Value) -> Result<Vec<String>, String> {
    Ok(members(metadata)?
        .iter()
        .map(|m| m.manifest_path.to_owned())
        .collect())
}

/// Whether a manifest opts into the workspace lints (a line-based check, no TOML parser).
pub fn has_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact == "lints.workspace=true" {
            return true;
        }
        if line.starts_with('[') {
            in_lints = compact == "[lints]";
        } else if in_lints && compact == "workspace=true" {
            return true;
        }
    }
    false
}

/// Checks the workspace and returns the violations (empty when it is clean). `manifests` maps
/// a manifest path to its text. An `Err` means the metadata itself is malformed.
pub fn check(
    metadata: &Value,
    manifests: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let members = members(metadata)?;
    let mut violations = Vec::new();

    for member in &members {
        if member.name != TOOL && layer_of(member.name).is_none() {
            violations.push(format!(
                "Workspace member `{}` is not registered in the layer table. Add it to `LAYERS` in xtask/src/layers.rs with its layer (AGENTS.md, section 2), or remove the crate.",
                member.name
            ));
        }
    }
    for (name, layer) in LAYERS {
        if !members.iter().any(|m| m.name == *name) {
            violations.push(format!(
                "The layer table lists `{name}` (L{layer}) but the workspace has no such crate. Create the crate under crates/ or apps/, or remove the entry from `LAYERS` in xtask/src/layers.rs."
            ));
        }
    }

    for member in &members {
        let own_layer = layer_of(member.name);
        for dep in &member.deps {
            if *dep == TOOL {
                violations.push(format!(
                    "`{}` depends on `{TOOL}`. No crate may depend on the tool crate; move the shared code into a layered crate.",
                    member.name
                ));
                continue;
            }
            if member.name == TOOL && layer_of(dep).is_some() {
                violations.push(format!(
                    "`{TOOL}` depends on the workspace crate `{dep}`. The tool crate may not depend on any workspace crate; remove the dependency."
                ));
                continue;
            }
            if let (Some(own), Some(dep_layer)) = (own_layer, layer_of(dep)) {
                let allowed = SAME_LAYER_ALLOWED
                    .iter()
                    .any(|(a, b)| a == &member.name && b == dep);
                if dep_layer > own || (dep_layer == own && !allowed) {
                    let why = if dep_layer == own {
                        "is in the same layer"
                    } else {
                        "is in a higher layer"
                    };
                    violations.push(format!(
                        "`{}` (L{own}) depends on `{dep}` (L{dep_layer}), which {why}. A crate may depend only on strictly lower layers; move the shared code down or invert the dependency (AGENTS.md, section 2).",
                        member.name
                    ));
                }
            }
            if let Some((_, allow)) = FORBIDDEN.iter().find(|(n, _)| n == dep) {
                let ok = match allow {
                    Allow::Only(names) => names.contains(&member.name),
                    Allow::FromLayer(min) => own_layer.is_none_or(|l| l >= *min),
                };
                if !ok {
                    let rule = match allow {
                        Allow::Only(names) => {
                            format!("only {} may depend on it", names.join(" and "))
                        }
                        Allow::FromLayer(min) => format!("nothing below L{min} may depend on it"),
                    };
                    violations.push(format!(
                        "`{}` depends on `{dep}`, but {rule}. Remove the dependency or move the code into a crate that may use it (AGENTS.md, section 2).",
                        member.name
                    ));
                }
            }
        }

        match manifests.get(member.manifest_path) {
            Some(text) if has_workspace_lints(text) => {}
            Some(_) => violations.push(format!(
                "`{}` does not opt into the workspace lints. Add `[lints]` with `workspace = true` to {}.",
                member.name, member.manifest_path
            )),
            None => violations.push(format!(
                "The manifest of `{}` ({}) could not be read, so its lints could not be checked.",
                member.name, member.manifest_path
            )),
        }
    }

    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LINTS: &str = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n";

    /// A package with normal dependencies and dev-dependencies.
    fn package(name: &str, deps: &[&str], dev: &[&str]) -> Value {
        let mut all: Vec<Value> = deps
            .iter()
            .map(|d| json!({"name": d, "kind": null}))
            .collect();
        all.extend(dev.iter().map(|d| json!({"name": d, "kind": "dev"})));
        json!({"name": name, "manifest_path": format!("/w/{name}/Cargo.toml"), "dependencies": all})
    }

    fn clean_packages() -> Vec<Value> {
        vec![
            package("gamecraft-math", &["libm"], &[]),
            package("gamecraft-world", &["gamecraft-math", "serde"], &[]),
            package("gamecraft-sim", &["gamecraft-world"], &[]),
            package(
                "gamecraft-engine",
                &["gamecraft-sim", "visioncraft-flora"],
                &[],
            ),
            package(
                "gamecraft-render",
                &[
                    "gamecraft-engine",
                    "visioncraft-render",
                    "visioncraft-flora",
                    "wgpu",
                ],
                &[],
            ),
            package("gamecraft-automation", &["gamecraft-engine"], &[]),
            package(
                "gamecraft-cli",
                &["gamecraft-render", "gamecraft-automation", "winit"],
                &[],
            ),
            package(
                "gamecraft",
                &["gamecraft-render", "gamecraft-automation", "winit"],
                &[],
            ),
            package("xtask", &["serde_json"], &[]),
        ]
    }

    fn run(packages: Vec<Value>) -> Vec<String> {
        let manifests = packages
            .iter()
            .filter_map(|p| p["manifest_path"].as_str())
            .map(|p| (p.to_owned(), LINTS.to_owned()))
            .collect();
        check(&json!({ "packages": packages }), &manifests).unwrap()
    }

    /// Replaces the package `name` in the clean workspace.
    fn with(name: &str, deps: &[&str], dev: &[&str]) -> Vec<String> {
        let mut packages = clean_packages();
        packages.retain(|p| p["name"] != name);
        packages.push(package(name, deps, dev));
        run(packages)
    }

    fn mentions(v: &[String], parts: &[&str]) -> bool {
        v.iter().any(|m| parts.iter().all(|p| m.contains(p)))
    }

    #[test]
    fn clean_workspace_passes() {
        assert_eq!(run(clean_packages()), Vec::<String>::new());
    }

    #[test]
    fn upward_dependency_is_reported_with_both_layers() {
        let v = with("gamecraft-world", &["gamecraft-sim"], &[]);
        assert!(
            mentions(&v, &["gamecraft-world", "L1", "gamecraft-sim", "L2"]),
            "{v:?}"
        );
    }

    #[test]
    fn same_layer_dependency_is_reported() {
        let v = with("gamecraft-render", &["gamecraft-automation"], &[]);
        assert!(
            mentions(
                &v,
                &["gamecraft-render", "gamecraft-automation", "same layer"]
            ),
            "{v:?}"
        );
    }

    #[test]
    fn unregistered_member_is_reported() {
        let mut packages = clean_packages();
        packages.push(package("gamecraft-extra", &[], &[]));
        let v = run(packages);
        assert!(
            mentions(&v, &["gamecraft-extra", "not registered"]),
            "{v:?}"
        );
    }

    #[test]
    fn table_entry_without_crate_is_reported() {
        let mut packages = clean_packages();
        packages.retain(|p| p["name"] != "gamecraft-sim");
        let v = run(packages);
        assert!(mentions(&v, &["gamecraft-sim", "no such crate"]), "{v:?}");
    }

    #[test]
    fn sim_depending_on_wgpu_is_reported() {
        let v = with("gamecraft-sim", &["gamecraft-world", "wgpu"], &[]);
        assert!(mentions(&v, &["gamecraft-sim", "wgpu", "L4"]), "{v:?}");
    }

    #[test]
    fn flora_is_restricted_to_render_and_engine() {
        let v = with(
            "gamecraft-world",
            &["gamecraft-math", "visioncraft-flora"],
            &[],
        );
        assert!(
            mentions(&v, &["gamecraft-world", "visioncraft-flora"]),
            "{v:?}"
        );
        assert!(
            !v.iter().any(|m| m.starts_with("`gamecraft-engine`")),
            "{v:?}"
        );
    }

    #[test]
    fn visioncraft_render_is_restricted_to_render() {
        let v = with(
            "gamecraft-engine",
            &["gamecraft-sim", "visioncraft-render"],
            &[],
        );
        assert!(
            mentions(&v, &["gamecraft-engine", "visioncraft-render"]),
            "{v:?}"
        );
    }

    #[test]
    fn window_crates_are_restricted_to_l5() {
        let v = with("gamecraft-automation", &["gamecraft-engine", "winit"], &[]);
        assert!(
            mentions(&v, &["gamecraft-automation", "winit", "L5"]),
            "{v:?}"
        );
    }

    #[test]
    fn dev_dependency_upward_is_not_reported() {
        let v = with(
            "gamecraft-world",
            &["gamecraft-math"],
            &["gamecraft-sim", "wgpu"],
        );
        assert_eq!(v, Vec::<String>::new());
    }

    #[test]
    fn manifest_without_workspace_lints_is_reported() {
        let packages = clean_packages();
        let mut manifests: BTreeMap<String, String> = packages
            .iter()
            .filter_map(|p| p["manifest_path"].as_str())
            .map(|p| (p.to_owned(), LINTS.to_owned()))
            .collect();
        manifests.insert(
            "/w/gamecraft-sim/Cargo.toml".into(),
            "[package]\nname = \"x\"\n[lints.clippy]\nfoo = \"deny\"\n".into(),
        );
        let v = check(&json!({ "packages": packages }), &manifests).unwrap();
        assert!(mentions(&v, &["gamecraft-sim", "workspace lints"]), "{v:?}");
        assert_eq!(v.len(), 1, "{v:?}");
    }

    #[test]
    fn lints_detection_accepts_dotted_form_and_rejects_false() {
        assert!(has_workspace_lints("lints.workspace = true\n"));
        assert!(has_workspace_lints("[lints]\nworkspace=true # yes\n"));
        assert!(!has_workspace_lints("[lints]\nworkspace = false\n"));
        assert!(!has_workspace_lints(
            "[lints]\n[dependencies]\nworkspace = true\n"
        ));
    }

    #[test]
    fn depending_on_the_tool_is_reported() {
        let v = with("gamecraft-cli", &["gamecraft-render", "xtask"], &[]);
        assert!(mentions(&v, &["gamecraft-cli", "xtask"]), "{v:?}");
    }

    #[test]
    fn tool_depending_on_a_workspace_crate_is_reported() {
        let v = with("xtask", &["gamecraft-math"], &[]);
        assert!(mentions(&v, &["xtask", "gamecraft-math"]), "{v:?}");
    }

    #[test]
    fn malformed_metadata_is_an_error_not_a_panic() {
        let none = BTreeMap::new();
        for bad in [
            json!(null),
            json!({}),
            json!({"packages": "x"}),
            json!({"packages": [{}]}),
            json!({"packages": [{"name": 3}]}),
            json!({"packages": [{"name": "a", "manifest_path": "/a"}]}),
            json!({"packages": [{"name": "a", "manifest_path": "/a", "dependencies": [{}]}]}),
            json!({"packages": [{"name": "a", "manifest_path": "/a", "dependencies": [{"name": "b", "kind": 1}]}]}),
        ] {
            assert!(check(&bad, &none).is_err(), "{bad}");
        }
    }
}
