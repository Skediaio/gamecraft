//! `gamecraft-cli`: the command line of GameCraft. One binary that builds a
//! [`gamecraft_automation::Host`], gives it a renderer and exposes it as subcommands: `run`,
//! `play`, `replay`, `render`, `info`, `new`, `commands`, and the two servers `serve` and `mcp`.
//!
//! Shaped after 3DCraft's `craft3d-cli` and VisionCraft's `visioncraft-cli` (MIT): results as
//! JSON on stdout, diagnostics on stderr; exit code 0 on success, 1 when a command or file
//! failed, 2 for a mistake in the command line.

// This app is outside the deterministic core (layers L0 to L3): it reads the clock, files and
// the GPU, and its float rounding never reaches a simulation result.
#![allow(clippy::disallowed_methods)]

mod args;
mod commands;
mod gpu;
mod help;
mod script;

use std::io::Write;
use std::process::ExitCode;

/// Why a subcommand stopped.
pub enum CliError {
    /// A mistake in the command line: exit code 2.
    Usage(String),
    /// A command, a file or the GPU failed: exit code 1.
    Failed(String),
}

/// What a subcommand returns.
pub type Res = Result<(), CliError>;

fn dispatch(raw: &[String]) -> Res {
    let Some((first, rest)) = raw.split_first() else {
        return commands::print(help::HELP);
    };
    let name = first.as_str();
    let ran = match name {
        "--help" | "-h" => return commands::print(help::HELP),
        "help" => return help_for(rest),
        "--version" | "-V" => {
            return commands::print(&format!("gamecraft-cli {}", env!("CARGO_PKG_VERSION")));
        }
        "commands" => commands::list(rest),
        "run" => commands::run(rest),
        "info" => commands::info(rest),
        "render" => commands::render(rest),
        "play" => commands::play(rest),
        "replay" => commands::replay(rest),
        "new" => commands::new(rest),
        "serve" => commands::serve(rest, false),
        "mcp" => commands::serve(rest, true),
        other if other.starts_with('-') => {
            return Err(CliError::Usage(format!(
                "unknown flag `{other}`; a subcommand comes first (gamecraft-cli --help lists them)"
            )));
        }
        other => {
            return Err(CliError::Usage(format!(
                "unknown subcommand `{other}`; the subcommands are: commands, run, info, render, \
                 play, replay, new, serve, mcp, help"
            )));
        }
    };
    ran.map_err(|e| match e {
        CliError::Usage(m) => CliError::Usage(format!(
            "{name}: {m}\n{}\nTry `gamecraft-cli {name} --help`.",
            sub_help(name).and_then(|h| h.lines().next()).unwrap_or("")
        )),
        other => other,
    })
}

/// The help of one subcommand.
fn sub_help(name: &str) -> Option<&'static str> {
    Some(match name {
        "commands" => help::COMMANDS,
        "run" => help::RUN,
        "info" => help::INFO,
        "render" => help::RENDER,
        "play" => help::PLAY,
        "replay" => help::REPLAY,
        "new" => help::NEW,
        "serve" => help::SERVE,
        "mcp" => help::MCP,
        _ => return None,
    })
}

fn help_for(rest: &[String]) -> Res {
    match rest.first() {
        None => commands::print(help::HELP),
        Some(name) => match sub_help(name) {
            Some(text) => commands::print(text),
            None => Err(CliError::Usage(format!(
                "`{name}` is not a subcommand; gamecraft-cli --help lists them"
            ))),
        },
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            let _ = writeln!(std::io::stderr(), "gamecraft-cli: {message}");
            ExitCode::from(2)
        }
        Err(CliError::Failed(message)) => {
            let _ = writeln!(std::io::stderr(), "gamecraft-cli: {message}");
            ExitCode::FAILURE
        }
    }
}
