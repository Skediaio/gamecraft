//! The steps of `run` and `play`: `--cmd`, `--params` and `--script` in the order given; and
//! a script read one step at a time for `project bake`.

use crate::CliError;
use crate::args::Args;
use serde_json::Value;

/// One command to run.
#[derive(Debug, PartialEq)]
pub struct Step {
    /// The command id.
    pub id: String,
    /// Its parameters (null for none).
    pub params: Value,
}

/// A step of the script `path` as the command it names; `where_` says where in the script it
/// is for the error ("step 3" or "line 12, step 3").
pub fn step_of(path: &str, step: &Value, where_: &str) -> Result<Step, CliError> {
    let failed = |why: String| CliError::Failed(why);
    let bad = || {
        failed(format!(
            "{path}: {where_} must be [\"id\", {{params}}] or \
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
            if let Some(key) = map
                .keys()
                .find(|k| !matches!(k.as_str(), "command" | "params"))
            {
                return Err(failed(format!(
                    "{path}: {where_} has the unknown key `{key}`; a step has `command` and \
                     `params`"
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
        .map(|(index, step)| step_of(path, step, &format!("step {index}")))
        .collect()
}

/// A script read one step at a time: the JSON list `read_script` takes, held never whole, so
/// that a generated script of a hundred thousand steps costs no more than one step. Each
/// step comes with its number (from 0) and the line it starts on (from 1).
pub struct Script {
    path: String,
    steps: serde_json::StreamDeserializer<
        'static,
        serde_json::de::IoRead<Between<std::io::BufReader<std::fs::File>>>,
        Value,
    >,
    /// What the reader between the brackets saw (shared with it).
    seen: std::rc::Rc<std::cell::RefCell<Seen>>,
    index: usize,
    /// Where in the stream of steps the last one ended.
    after: usize,
}

/// Opens the script `path`; an error that names it when it cannot be read or does not start
/// as a list.
impl Script {
    /// Opens the script `path`; an error that names it when it cannot be read.
    pub fn open(path: &str) -> Result<Script, CliError> {
        let failed = |why: String| CliError::Failed(why);
        let file = std::fs::File::open(path)
            .map_err(|e| failed(format!("cannot read the script `{path}`: {e}")))?;
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Seen {
            line: 1,
            ..Seen::default()
        }));
        let between = Between {
            inner: std::io::BufReader::new(file),
            seen: seen.clone(),
            depth: 0,
            list_open: false,
            in_string: false,
            escaped: false,
            done: false,
            at_value_start: true,
            scratch: vec![0; 8192],
            from: 0,
            to: 0,
        };
        let steps = serde_json::Deserializer::from_reader(between).into_iter::<Value>();
        Ok(Script {
            path: path.to_string(),
            steps,
            seen,
            index: 0,
            after: 0,
        })
    }

    /// The next step: `(number, line, step)`, `None` at the end of the script. An error names
    /// the script, the line and the step.
    pub fn next_step(&mut self) -> Result<Option<(usize, usize, Step)>, CliError> {
        let after = self.after;
        let index = self.index;
        let path = self.path.clone();
        match self.steps.next() {
            None if self.seen.borrow().closed => Ok(None),
            None => Err(CliError::Failed(format!(
                "{path}: the list of steps is not closed (its last `]` is missing)"
            ))),
            Some(Err(e)) => {
                let what = e.to_string();
                // What the reader between the brackets refused is said as it is; the rest is
                // a step that is not JSON, placed on its line.
                let whole = [
                    "the script is not a JSON list",
                    "a comma before the list's closing bracket",
                ];
                Err(CliError::Failed(
                    if whole.iter().any(|w| what.contains(w)) {
                        format!("{path}: {what}")
                    } else {
                        let (_, line) = self.seen.borrow().start_after(after);
                        format!("{path}:{line}: step {index} is not JSON: {what}")
                    },
                ))
            }
            Some(Ok(value)) => {
                self.after = self.steps.byte_offset();
                self.index += 1;
                let (_, line) = self.seen.borrow().start_after(after);
                let step = step_of(&path, &value, &format!("line {line}, step {index}"))?;
                Ok(Some((index, line, step)))
            }
        }
    }
}

/// What [`Between`] saw of the script: where each step starts, to name the line of one that
/// fails.
#[derive(Default)]
struct Seen {
    /// Bytes let through so far.
    passed: usize,
    /// Lines so far: the line (from 1) of the byte that was let through last.
    line: usize,
    /// The start of every step: `(bytes let through before it, its line)`.
    starts: Vec<(usize, usize)>,
    /// The list's closing bracket was seen: the script is whole.
    closed: bool,
}

impl Seen {
    /// The start of the first step after `after` bytes were let through: `(bytes let through
    /// before it, its line)`; `(after, the line there)` when none was seen.
    fn start_after(&self, after: usize) -> (usize, usize) {
        let at = self.starts.partition_point(|(offset, _)| *offset <= after);
        match self.starts.get(at).copied() {
            Some(start) => start,
            None => (after, self.line),
        }
    }
}

/// The bytes between the brackets of a JSON list, one value after another: the reader
/// `serde_json` needs to take the list's items as a stream. The opening `[` and the final `]`
/// are kept back, and so is every comma that separates two items (a comma inside an item, or
/// inside a string, goes through). The start of every item is remembered with its line.
struct Between<R> {
    inner: R,
    seen: std::rc::Rc<std::cell::RefCell<Seen>>,
    /// Brackets and braces open: 0 before the list, 1 between its items, more inside one.
    depth: usize,
    /// The list's own bracket was seen.
    list_open: bool,
    in_string: bool,
    escaped: bool,
    /// The list is closed: only blank bytes may follow; the rest goes through, to be refused.
    done: bool,
    /// The next byte that is not blank starts an item.
    at_value_start: bool,
    scratch: Vec<u8>,
    /// Bytes of `scratch` read from the file and not let through yet: `from..to`.
    from: usize,
    to: usize,
}

impl<R: std::io::Read> std::io::Read for Between<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // The reader asks for little (a byte at a time): what was read of the file and not
        // let through yet stays in `scratch`, between `from` and `to`.
        let mut out = 0;
        while out < buf.len() {
            if self.from == self.to {
                self.from = 0;
                self.to = self.inner.read(&mut self.scratch)?;
                if self.to == 0 {
                    return Ok(out);
                }
            }
            let b = match self.scratch.get(self.from) {
                Some(&b) => b,
                None => return Ok(out),
            };
            self.from += 1;
            let mut seen = self.seen.borrow_mut();
            // Lets one byte through (counting the lines).
            let mut pass = |seen: &mut Seen, out: &mut usize, b: u8| {
                if b == b'\n' {
                    seen.line += 1;
                }
                seen.passed += 1;
                if let Some(slot) = buf.get_mut(*out) {
                    *slot = b;
                    *out += 1;
                }
            };
            if self.done {
                if !b.is_ascii_whitespace() {
                    pass(&mut seen, &mut out, b);
                }
                continue;
            }
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'"' {
                    self.in_string = false;
                }
                pass(&mut seen, &mut out, b);
                continue;
            }
            match b {
                b'[' | b'{' if !self.list_open => {
                    if b != b'[' {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "the script is not a JSON list",
                        ));
                    }
                    self.list_open = true;
                    self.depth = 1;
                }
                b'[' | b'{' => {
                    self.depth += 1;
                    if self.depth == 2 && self.at_value_start {
                        let start = (seen.passed, seen.line);
                        seen.starts.push(start);
                        self.at_value_start = false;
                    }
                    pass(&mut seen, &mut out, b);
                }
                b']' | b'}' => {
                    if self.depth > 1 {
                        self.depth -= 1;
                        pass(&mut seen, &mut out, b);
                    } else if b == b']' {
                        if self.at_value_start && !seen.starts.is_empty() {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "a comma before the list's closing bracket",
                            ));
                        }
                        self.done = true;
                        seen.closed = true;
                    } else {
                        // A brace closing where nothing opened: let it be refused.
                        pass(&mut seen, &mut out, b);
                    }
                }
                b',' if self.depth == 1 => self.at_value_start = true,
                b if b.is_ascii_whitespace() => pass(&mut seen, &mut out, b),
                b'"' => {
                    if self.depth == 1 && self.at_value_start {
                        let start = (seen.passed, seen.line);
                        seen.starts.push(start);
                        self.at_value_start = false;
                    }
                    self.in_string = true;
                    pass(&mut seen, &mut out, b);
                }
                _ => {
                    if self.depth == 1 && self.at_value_start {
                        let start = (seen.passed, seen.line);
                        seen.starts.push(start);
                        self.at_value_start = false;
                    }
                    pass(&mut seen, &mut out, b);
                }
            }
        }
        Ok(out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A script written to a file that lives as long as the test.
    fn written(text: &str, tag: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("gamecraft-cli-script-{tag}-{}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn steps(text: &str, tag: &str) -> Result<Vec<(usize, usize, Step)>, CliError> {
        let path = written(text, tag);
        let mut script = Script::open(&path.display().to_string())?;
        let mut out = Vec::new();
        while let Some(step) = script.next_step()? {
            out.push(step);
        }
        let _ = std::fs::remove_file(&path);
        Ok(out)
    }

    #[test]
    fn steps_come_with_their_numbers_and_lines() {
        let pretty = "[\n  [\"terrain.set\", {\"kind\": \"flat\"}],\n  [\"entity.add\", {}],\n  {\"command\": \"world.inspect\"}\n]\n";
        let read = steps(pretty, "pretty").unwrap();
        assert_eq!(
            read.iter()
                .map(|(i, line, s)| (*i, *line, s.id.clone()))
                .collect::<Vec<_>>(),
            vec![
                (0, 2, "terrain.set".to_string()),
                (1, 3, "entity.add".to_string()),
                (2, 4, "world.inspect".to_string())
            ]
        );
        assert_eq!(read[1].2.params, Value::Object(Default::default()));
        assert_eq!(read[2].2.params, Value::Null);
        let compact = "[[\"a\", null], [\"b\", [1, 2]], [\"c\"]]";
        let read = steps(compact, "compact").unwrap();
        assert_eq!(
            read.iter().map(|(_, line, _)| *line).collect::<Vec<_>>(),
            vec![1, 1, 1]
        );
        assert_eq!(read[1].2.params, json!([1, 2]));
        assert_eq!(steps("[ ]", "blank").unwrap(), Vec::new());
        assert_eq!(steps("[\n\n]", "blank-lines").unwrap(), Vec::new());
    }

    #[test]
    fn commas_and_brackets_inside_a_step_go_through() {
        // Strings with commas and brackets, and values nested in values: the same steps as
        // reading the whole list.
        let text = r#"[
  ["entity.add", {"name": "A, [not a step]", "tags": ["x,y", "]"], "at": [[1, 2], [3, 4]]}],
  ["entity.set", {"id": 1, "position": [0, 0, 0], "note": "quote: \", brace: }"}]
]"#;
        let read = steps(text, "nested").unwrap();
        let whole: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(read.len(), whole.len());
        for ((_, line, step), value) in read.iter().zip(&whole) {
            assert_eq!(
                json!([step.id.clone(), step.params.clone()]),
                *value,
                "{line}"
            );
        }
        assert_eq!(read[1].1, 3, "the line of the second step");
    }

    #[test]
    fn what_is_not_a_list_of_steps_is_an_error_with_its_place() {
        let not_a_list = steps("{\"command\": \"world.inspect\"}", "object").unwrap_err();
        assert!(
            matches!(&not_a_list, CliError::Failed(text) if text.contains(": the script is not a JSON list")),
            "{not_a_list:?}"
        );
        let unclosed = steps("[[\"world.inspect\"]", "unclosed").unwrap_err();
        assert!(
            matches!(&unclosed, CliError::Failed(text) if text.contains("not closed")),
            "{unclosed:?}"
        );
        let bad = steps("[\n  [\"world.inspect\"],\n  7\n]", "number").unwrap_err();
        assert!(
            matches!(&bad, CliError::Failed(text) if text.contains(": line 3, step 1")),
            "{bad:?}"
        );
        let broken = steps("[\n  [\"a\", {]\n]", "broken-json").unwrap_err();
        assert!(
            matches!(&broken, CliError::Failed(text) if text.contains(":2:") && text.contains("step 0")),
            "{broken:?}"
        );
        let leading = steps("\n  [\"world.inspect\"],\n  7\n]\n", "leading");
        match leading {
            Err(e) => println!("leading: {e:?}"),
            Ok(read) => println!("leading: {read:?}"),
        }
        let comma = steps("[[\"world.inspect\"],]\n", "comma").unwrap_err();
        assert!(
            matches!(&comma, CliError::Failed(text) if text.contains("comma before")),
            "{comma:?}"
        );
        let missing = Script::open("/nowhere/no-script.json").err().unwrap();
        assert!(
            matches!(&missing, CliError::Failed(text) if text.contains("cannot read the script")),
            "{missing:?}"
        );
    }
}
