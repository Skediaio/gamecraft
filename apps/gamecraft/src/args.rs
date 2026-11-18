//! The native command line: `gamecraft [world.gamecraft] [--stdin] [--quality low|medium|high]`.

use gamecraft_render::Quality;
use std::path::PathBuf;

/// The usage, as `--help` prints it.
pub const USAGE: &str = "\
Usage: gamecraft [world.gamecraft] [--stdin] [--quality low|medium|high]

Opens a window in which you walk the world's first character (by id). Without a world, the
engine's example valley opens.

  --stdin            read JSON-lines requests on stdin (the protocol of `gamecraft-cli serve`),
                     run them between frames and answer on stdout
  --quality <q>      live frames' effects: low, medium (default) or high
  --help             print this

Keys: W/S forward and back, A/D strafe, Shift run, Space jump, E interact, mouse look (click
the window to lock the pointer, Esc releases it), F1 first or third person, F5 restart, P pause.";

/// The flags it knows, for the nearest name in an error.
const FLAGS: [&str; 3] = ["--stdin", "--quality", "--help"];

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The world file; `None` opens the example world.
    pub world: Option<PathBuf>,
    /// Read the control protocol on stdin.
    pub stdin: bool,
    /// The live frames' quality.
    pub quality: Quality,
    /// Print the usage and stop.
    pub help: bool,
}

/// Reads the arguments (without the program's name). The error names the argument and what is
/// allowed.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdin" => out.stdin = true,
            "--help" | "-h" => out.help = true,
            "--quality" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--quality needs a value: low, medium or high".to_string())?;
                out.quality = Quality::parse(&value)?;
            }
            flag if flag.starts_with('-') => {
                let nearest = FLAGS
                    .iter()
                    .min_by_key(|f| distance(f, flag))
                    .map(|f| format!("; did you mean {f}?"))
                    .unwrap_or_default();
                return Err(format!("unknown option {flag}{nearest} (see --help)"));
            }
            path => {
                if let Some(first) = &out.world {
                    return Err(format!(
                        "a second world {path} was given after {}; open one world at a time",
                        first.display()
                    ));
                }
                out.world = Some(PathBuf::from(path));
            }
        }
    }
    Ok(out)
}

/// Edit distance between two short strings.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row.first().copied().unwrap_or(0);
        if let Some(first) = row.first_mut() {
            *first = i + 1;
        }
        for (j, cb) in b.iter().enumerate() {
            let above = row.get(j + 1).copied().unwrap_or(usize::MAX);
            let left = row.get(j).copied().unwrap_or(usize::MAX);
            let diagonal = previous + usize::from(ca != *cb);
            previous = above;
            if let Some(cell) = row.get_mut(j + 1) {
                *cell = diagonal
                    .min(above.saturating_add(1))
                    .min(left.saturating_add(1));
            }
        }
    }
    row.last().copied().unwrap_or(0)
}
