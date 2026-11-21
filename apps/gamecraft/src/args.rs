//! The native command line: `gamecraft [world.gamecraft] [--stdin] [--quality low|medium|high]
//! [--invert-look] [--fov DEG]`.

use gamecraft_render::Quality;
use gamecraft_world::limits::{VIEW_FOV_MAX_DEG, VIEW_FOV_MIN_DEG};
use std::path::PathBuf;

/// The usage, as `--help` prints it.
pub const USAGE: &str = "\
Usage: gamecraft [world.gamecraft | PROJECT] [--stdin] [--quality low|medium|high] [--invert-look]
                [--fov DEG]

Opens a window in which you walk the world's first character (by id). A folder is a project:
its packs are layered and its models drawn from their assets. Without a world, the engine's
example valley opens.

  --stdin            read JSON-lines requests on stdin (the protocol of `gamecraft-cli serve`),
                     run them between frames and answer on stdout
  --quality <q>      live frames' effects: low, medium (default) or high
  --invert-look      a gamepad's right stick pushed up looks down
  --fov <deg>        your own vertical field of view, 30 to 120 degrees (default: the world's,
                     70 unless it sets one)
  --help             print this

Keys: W/S forward and back, A/D strafe, Shift run, Space jump, E interact, mouse look (click
the window to lock the pointer, Esc releases it), left button fires, hold Left Alt to look
around without turning or moving the aim, F1 switches among the views the world allows (first
or third person), F5 restart, P pause.
Gamepad: left stick walks, right stick looks, stick click or left trigger runs, south button
jumps, west interacts, right trigger fires, start pauses, select switches the view.";

/// The flags it knows, for the nearest name in an error.
const FLAGS: [&str; 5] = ["--stdin", "--quality", "--invert-look", "--fov", "--help"];

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    /// The world file or project folder; `None` opens the example world.
    pub world: Option<PathBuf>,
    /// Read the control protocol on stdin.
    pub stdin: bool,
    /// The live frames' quality.
    pub quality: Quality,
    /// A gamepad's right stick pushed up looks down.
    pub invert_look: bool,
    /// The player's own vertical field of view, degrees; `None` uses the world's.
    pub fov: Option<f64>,
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
            "--invert-look" => out.invert_look = true,
            "--help" | "-h" => out.help = true,
            "--quality" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--quality needs a value: low, medium or high".to_string())?;
                out.quality = Quality::parse(&value)?;
            }
            "--fov" => {
                let value = args.next().ok_or_else(|| {
                    format!(
                        "--fov needs a value: the vertical field of view in degrees, \
                         {VIEW_FOV_MIN_DEG} to {VIEW_FOV_MAX_DEG}"
                    )
                })?;
                out.fov = Some(parse_fov(&value).map_err(|e| format!("--fov {e}"))?);
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

/// A vertical field of view in degrees, 30 to 120, from text (the native `--fov` and the
/// browser's `?fov=`). The error says what is wrong and what is allowed.
pub fn parse_fov(text: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(fov) if fov.is_finite() && (VIEW_FOV_MIN_DEG..=VIEW_FOV_MAX_DEG).contains(&fov) => {
            Ok(fov)
        }
        Ok(_) => Err(format!(
            "{text} is outside {VIEW_FOV_MIN_DEG} to {VIEW_FOV_MAX_DEG} degrees; 70 is the usual \
             view, 90 a wide one"
        )),
        Err(_) => Err(format!(
            "`{text}` is not a number of degrees; give {VIEW_FOV_MIN_DEG} to {VIEW_FOV_MAX_DEG}, \
             such as 90"
        )),
    }
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
