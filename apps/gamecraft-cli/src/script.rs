//! The steps of `run` and `play`: `--cmd`, `--params` and `--script` in the order given.

use crate::CliError;
use crate::args::Args;
use serde_json::Value;

/// One command to run.
pub struct Step {
    /// The command id.
    pub id: String,
    /// Its parameters (null for none).
    pub params: Value,
}

/// Reads a script file: a JSON list of `["id", {params}]` or `{"command": id, "params": {...}}`.
pub fn read_script(path: &str) -> Result<Vec<Step>, CliError> {
    let failed = |why: String| CliError::Failed(why);
    let text = std::fs::read_to_string(path)
        .map_err(|e| failed(format!("cannot read the script `{path}`: {e}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| failed(format!("the script `{path}` is not valid JSON: {e}")))?;
    let Value::Array(list) = value else {
        return Err(failed(format!(
            "the script `{path}` must be a JSON list of steps, [[\"id\", {{params}}], ...]"
        )));
    };
    list.iter()
        .enumerate()
        .map(|(index, step)| {
            let bad = || {
                failed(format!(
                    "{path}: step {index} must be [\"id\", {{params}}] or \
                     {{\"command\": \"id\", \"params\": {{...}}}}"
                ))
            };
            match step {
                Value::Array(items) => {
                    if items.len() > 2 {
                        return Err(bad());
                    }
                    let id = items.first().and_then(Value::as_str).ok_or_else(bad)?;
                    Ok(Step {
                        id: id.to_string(),
                        params: items.get(1).cloned().unwrap_or(Value::Null),
                    })
                }
                Value::Object(map) => {
                    if let Some(key) = map.keys().find(|k| !matches!(k.as_str(), "command" | "params")) {
                        return Err(failed(format!(
                            "{path}: step {index} has the unknown key `{key}`; a step has `command` and `params`"
                        )));
                    }
                    let id = map.get("command").and_then(Value::as_str).ok_or_else(bad)?;
                    Ok(Step {
                        id: id.to_string(),
                        params: map.get("params").cloned().unwrap_or(Value::Null),
                    })
                }
                _ => Err(bad()),
            }
        })
        .collect()
}

/// The steps of the command line, `--cmd` (with the `--params` right after it) and `--script`
/// mixed in the order given.
pub fn collect_steps(args: &Args) -> Result<Vec<Step>, CliError> {
    let mut steps: Vec<Step> = Vec::new();
    // Index of the step a `--params` would belong to: the last one, if `--cmd` made it.
    let mut open_cmd: Option<usize> = None;
    for (name, value) in &args.flags {
        match (name.as_str(), value) {
            ("--cmd", Some(id)) => {
                open_cmd = Some(steps.len());
                steps.push(Step {
                    id: id.clone(),
                    params: Value::Null,
                });
            }
            ("--params", Some(text)) => {
                let step = open_cmd
                    .take()
                    .and_then(|i| steps.get_mut(i))
                    .ok_or_else(|| {
                        CliError::Usage(
                            "`--params` must come right after the `--cmd` it belongs to".into(),
                        )
                    })?;
                step.params = serde_json::from_str(text)
                    .map_err(|e| CliError::Usage(format!("`--params` is not valid JSON: {e}")))?;
            }
            ("--script", Some(path)) => {
                open_cmd = None;
                steps.extend(read_script(path)?);
            }
            _ => {}
        }
    }
    Ok(steps)
}
