//! The view rules of a world: which cameras a player may use and how wide they see.

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{VIEW_FOV_MAX_DEG, VIEW_FOV_MIN_DEG};
use serde::{Deserialize, Serialize};

/// The vertical field of view of the player's camera when a world sets none, degrees.
pub const DEFAULT_FOV_Y_DEG: f64 = 70.0;

/// A camera a player may see the world through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// From the character's eyes.
    First,
    /// Behind and above the character.
    Third,
}

impl ViewMode {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ViewMode::First => "first",
            ViewMode::Third => "third",
        }
    }
}

fn default_modes() -> Vec<ViewMode> {
    vec![ViewMode::Third, ViewMode::First]
}

fn default_fov() -> f64 {
    DEFAULT_FOV_Y_DEG
}

fn default_crosshair() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

/// Which views a player may use, in order (a window starts in the first), and the vertical
/// field of view of the player's camera. A game that forbids the third-person view sets
/// `modes` to `["first"]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewRules {
    /// The views allowed, in order, at least one, none twice. Default `["third", "first"]`.
    #[serde(default = "default_modes")]
    pub modes: Vec<ViewMode>,
    /// The player camera's vertical field of view, degrees, 30 to 120. Default 70.
    #[serde(default = "default_fov")]
    pub fov_y_deg: f64,
    /// Whether the window draws a crosshair in the first-person view (hip fire; down the
    /// sights there is none anyway). Default true; written only when false. A realistic game
    /// sets false.
    #[serde(default = "default_crosshair", skip_serializing_if = "is_true")]
    pub crosshair: bool,
}

impl Default for ViewRules {
    fn default() -> Self {
        ViewRules {
            modes: default_modes(),
            fov_y_deg: DEFAULT_FOV_Y_DEG,
            crosshair: true,
        }
    }
}

impl ViewRules {
    /// Whether these are the rules of a world that sets none (bit for bit), so the file leaves
    /// them out.
    pub fn is_default(&self) -> bool {
        self.modes == default_modes()
            && self.fov_y_deg.to_bits() == DEFAULT_FOV_Y_DEG.to_bits()
            && self.crosshair
    }

    /// Whether `mode` is allowed.
    pub fn allows(&self, mode: ViewMode) -> bool {
        self.modes.contains(&mode)
    }

    /// Checks the rules: at least one mode, none twice, a finite field of view from 30 to 120
    /// degrees.
    pub(crate) fn validate(&self) -> Result<(), WorldError> {
        let ctx = Ctx::WORLD;
        let names: Vec<&str> = self.modes.iter().map(|m| m.name()).collect();
        if self.modes.is_empty() {
            return Err(ctx.invalid(
                "view.modes",
                "[]",
                "it must allow at least one view: \"first\", \"third\" or both",
            ));
        }
        for (i, mode) in self.modes.iter().enumerate() {
            if self.modes.iter().skip(i + 1).any(|m| m == mode) {
                return Err(ctx.invalid(
                    "view.modes",
                    format!("{names:?}"),
                    format!("each view is listed once; \"{}\" is twice", mode.name()),
                ));
            }
        }
        ctx.range(
            "view.fov_y_deg",
            self.fov_y_deg,
            VIEW_FOV_MIN_DEG,
            VIEW_FOV_MAX_DEG,
            " degrees",
        )
    }
}
