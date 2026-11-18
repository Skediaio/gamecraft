//! Hand-written argument parsing: positionals, and flags with their values in the order given.

use crate::CliError;

/// The arguments of one subcommand.
pub struct Args {
    /// The arguments that are not flags, in order.
    pub positionals: Vec<String>,
    /// Every flag with its value (`None` for a switch), in the order given.
    pub flags: Vec<(String, Option<String>)>,
}

impl Args {
    /// Splits `raw` into positionals and flags. `with_value` are the flags that take a value
    /// (`--flag value` or `--flag=value`), `switches` those that do not. `--help` and `-h` are
    /// always known; `--` ends the flags.
    pub fn parse(raw: &[String], with_value: &[&str], switches: &[&str]) -> Result<Args, CliError> {
        let (mut positionals, mut flags) = (Vec::new(), Vec::new());
        let mut iter = raw.iter();
        let mut only_positionals = false;
        while let Some(arg) = iter.next() {
            if only_positionals || !arg.starts_with('-') || arg == "-" {
                positionals.push(arg.clone());
                continue;
            }
            if arg == "--" {
                only_positionals = true;
                continue;
            }
            if arg == "-h" || arg == "--help" {
                flags.push(("--help".to_string(), None));
                continue;
            }
            let (name, inline) = match arg.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (arg.as_str(), None),
            };
            if switches.contains(&name) {
                if inline.is_some() {
                    return Err(CliError::Usage(format!("`{name}` takes no value")));
                }
                flags.push((name.to_string(), None));
            } else if with_value.contains(&name) {
                let value = match inline {
                    Some(v) => v,
                    None => iter
                        .next()
                        .cloned()
                        .ok_or_else(|| CliError::Usage(format!("`{name}` needs a value")))?,
                };
                flags.push((name.to_string(), Some(value)));
            } else {
                return Err(CliError::Usage(format!("unknown flag `{name}`")));
            }
        }
        Ok(Args { positionals, flags })
    }

    /// Whether `--help` or `-h` was given.
    pub fn help(&self) -> bool {
        self.has("--help")
    }

    /// Whether the flag was given.
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }

    /// The value of a flag that may be given once.
    pub fn value(&self, name: &str) -> Result<Option<&str>, CliError> {
        let mut found = self.flags.iter().filter(|(n, _)| n == name);
        let first = found.next().and_then(|(_, v)| v.as_deref());
        if found.next().is_some() {
            return Err(CliError::Usage(format!("`{name}` is given twice")));
        }
        Ok(first)
    }

    /// The only positional, if any; more than `max` positionals is a mistake.
    pub fn positional(&self, max: usize) -> Result<Option<&str>, CliError> {
        if let Some(extra) = self.positionals.get(max) {
            return Err(CliError::Usage(format!("unexpected argument `{extra}`")));
        }
        Ok(self.positionals.first().map(String::as_str))
    }

    /// The positional that must be there.
    pub fn required_file(&self, what: &str) -> Result<&str, CliError> {
        self.positional(1)?
            .ok_or_else(|| CliError::Usage(format!("give {what}")))
    }
}
