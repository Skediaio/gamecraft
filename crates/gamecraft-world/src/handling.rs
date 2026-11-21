//! How a launcher handles in a character's hands: its fire modes, the recoil of each shot and
//! the sway of the aim. Mechanisms only, with their numbers as data: a pack fills them in for a
//! service rifle, a pistol or a bow (`docs/design/ballistics.md`).

use crate::WorldError;
use crate::error::Ctx;
use crate::limits::{
    BURST_COUNT_MAX, CYCLIC_RPM_MAX, HANDLING_TIME_MAX_S, LAUNCHER_TIME_MAX_S, MAGAZINE_MAX,
    RECOIL_KICK_MAX_MRAD, SWAY_FACTOR_MAX,
};
use crate::{Launcher, combat::above_zero};
use serde::{Deserialize, Serialize};

/// A position of a launcher's selector.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum FireMode {
    /// It does not fire.
    Safe,
    /// One shot per pull of the trigger, at most one every `rate_s`.
    Semi,
    /// Shots at the cyclic rate while the trigger is held.
    #[default]
    Auto,
    /// `burst_count` shots at the cyclic rate per pull, fewer when the trigger is let go.
    Burst,
}

impl FireMode {
    /// Every mode name, as in JSON.
    pub const NAMES: [&'static str; 4] = ["safe", "semi", "auto", "burst"];

    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            FireMode::Safe => "safe",
            FireMode::Semi => "semi",
            FireMode::Auto => "auto",
            FireMode::Burst => "burst",
        }
    }

    /// The mode of a name as in JSON, `None` for another name.
    pub fn from_name(name: &str) -> Option<FireMode> {
        match name {
            "safe" => Some(FireMode::Safe),
            "semi" => Some(FireMode::Semi),
            "auto" => Some(FireMode::Auto),
            "burst" => Some(FireMode::Burst),
            _ => None,
        }
    }
}

/// How each shot of a launcher kicks its shooter's aim: `kick_up_mrad` up and up to
/// `kick_side_mrad` to either side (a seeded draw, so deterministic), on top of the aim asked
/// for. The kick decays back towards the aim asked for: after `recovery_s` about 5 % of it is
/// left (three time constants of an exponential decay). Kicks of shots fired faster than that
/// stack, so automatic fire climbs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Recoil {
    /// Upward kick per shot, milliradians, 0 to 100. Default 0.
    pub kick_up_mrad: f64,
    /// Largest sideways kick per shot, milliradians, 0 to 100. Default 0.
    pub kick_side_mrad: f64,
    /// Seconds to recover to the aim, 0 to 600; 0 recovers by the next tick. Default 0.25.
    pub recovery_s: f64,
}

impl Default for Recoil {
    fn default() -> Self {
        Recoil {
            kick_up_mrad: 0.0,
            kick_side_mrad: 0.0,
            recovery_s: 0.25,
        }
    }
}

impl Recoil {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        for (field, v) in [
            ("launcher.recoil.kick_up_mrad", self.kick_up_mrad),
            ("launcher.recoil.kick_side_mrad", self.kick_side_mrad),
        ] {
            ctx.range(field, v, 0.0, RECOIL_KICK_MAX_MRAD, " milliradians")?;
        }
        ctx.range(
            "launcher.recoil.recovery_s",
            self.recovery_s,
            0.0,
            HANDLING_TIME_MAX_S,
            " seconds",
        )
    }
}

/// How a character's aim drifts while it holds a launcher: a slow, smooth wander (a sum of a
/// few sines with seeded phases) whose size is the stance's amplitude, times `moving_factor`
/// while it moves, times up to `fatigue_factor` after running (fatigue builds over
/// `fatigue_build_s` of running and fades over `fatigue_fade_s`), times the launcher's
/// `ads_sway_factor` while aiming down the sights. Holding the breath steadies it to
/// `breath_steady_factor` for at most `breath_hold_s`; for `breath_recover_s` after that it is
/// `breath_recover_factor` (worse).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sway {
    /// Amplitude standing, milliradians, 0 to 100. Default 0 (no sway).
    pub standing_mrad: f64,
    /// Amplitude crouched, milliradians, 0 to 100. Default 0. Characters have no stances
    /// yet: they all stand, so this is kept for when they do.
    pub crouched_mrad: f64,
    /// Amplitude lying prone, milliradians, 0 to 100. Default 0 (kept like `crouched_mrad`).
    pub prone_mrad: f64,
    /// Times the amplitude while the character moves, 0 to 100. Default 2.
    pub moving_factor: f64,
    /// Times the amplitude at full fatigue, 0 to 100. Default 2.5.
    pub fatigue_factor: f64,
    /// Seconds of running to full fatigue, above 0 and at most 600. Default 6.
    pub fatigue_build_s: f64,
    /// Seconds for full fatigue to fade, above 0 and at most 600. Default 10.
    pub fatigue_fade_s: f64,
    /// Longest the breath can be held, seconds, 0 to 600. Default 4.
    pub breath_hold_s: f64,
    /// Times the amplitude while the breath is held, 0 to 100. Default 0.2.
    pub breath_steady_factor: f64,
    /// Seconds of a worse sway after holding the breath, 0 to 600. Default 3.
    pub breath_recover_s: f64,
    /// Times the amplitude in those seconds, 0 to 100. Default 1.5.
    pub breath_recover_factor: f64,
    /// The period of the slowest part of the drift (breathing), seconds, above 0 and at most
    /// 600. Default 4.
    pub period_s: f64,
}

impl Default for Sway {
    fn default() -> Self {
        Sway {
            standing_mrad: 0.0,
            crouched_mrad: 0.0,
            prone_mrad: 0.0,
            moving_factor: 2.0,
            fatigue_factor: 2.5,
            fatigue_build_s: 6.0,
            fatigue_fade_s: 10.0,
            breath_hold_s: 4.0,
            breath_steady_factor: 0.2,
            breath_recover_s: 3.0,
            breath_recover_factor: 1.5,
            period_s: 4.0,
        }
    }
}

impl Sway {
    fn validate(&self, ctx: Ctx) -> Result<(), WorldError> {
        for (field, v) in [
            ("launcher.sway.standing_mrad", self.standing_mrad),
            ("launcher.sway.crouched_mrad", self.crouched_mrad),
            ("launcher.sway.prone_mrad", self.prone_mrad),
        ] {
            ctx.range(field, v, 0.0, RECOIL_KICK_MAX_MRAD, " milliradians")?;
        }
        for (field, v) in [
            ("launcher.sway.moving_factor", self.moving_factor),
            ("launcher.sway.fatigue_factor", self.fatigue_factor),
            (
                "launcher.sway.breath_steady_factor",
                self.breath_steady_factor,
            ),
            (
                "launcher.sway.breath_recover_factor",
                self.breath_recover_factor,
            ),
        ] {
            ctx.range(field, v, 0.0, SWAY_FACTOR_MAX, "")?;
        }
        for (field, v) in [
            ("launcher.sway.fatigue_build_s", self.fatigue_build_s),
            ("launcher.sway.fatigue_fade_s", self.fatigue_fade_s),
            ("launcher.sway.period_s", self.period_s),
        ] {
            above_zero(ctx, field, v, HANDLING_TIME_MAX_S, " seconds")?;
        }
        for (field, v) in [
            ("launcher.sway.breath_hold_s", self.breath_hold_s),
            ("launcher.sway.breath_recover_s", self.breath_recover_s),
        ] {
            ctx.range(field, v, 0.0, HANDLING_TIME_MAX_S, " seconds")?;
        }
        Ok(())
    }
}

// A document leaves the handling fields out while they are at their defaults, so a world
// written before they existed keeps its text (and so its state hashes).

/// The default of the burst, 3 shots.
const DEFAULT_BURST_COUNT: u32 = 3;
/// The default share of sway and dispersion left while aiming down the sights.
const DEFAULT_ADS_FACTOR: f64 = 0.5;

pub(crate) fn default_modes() -> Vec<FireMode> {
    vec![FireMode::Auto]
}

pub(crate) fn is_default_modes(modes: &[FireMode]) -> bool {
    modes == [FireMode::Auto]
}

pub(crate) fn is_default_burst(count: &u32) -> bool {
    *count == DEFAULT_BURST_COUNT
}

pub(crate) fn is_default_recoil(recoil: &Recoil) -> bool {
    *recoil == Recoil::default()
}

pub(crate) fn is_default_sway(sway: &Sway) -> bool {
    *sway == Sway::default()
}

pub(crate) fn is_default_ads_factor(factor: &f64) -> bool {
    *factor == DEFAULT_ADS_FACTOR
}

/// The defaults of the `launcher`'s handling fields as JSON (`modes`, `burst_count`,
/// `cyclic_rpm`, `reload_empty_s`, `spare_magazines`, `recoil`, `sway`, `ads_sway_factor`,
/// `ads_dispersion_factor`): a document leaves them out while they are at their defaults, so a
/// listing of the component's defaults adds them from here.
pub fn launcher_handling_defaults() -> serde_json::Value {
    let l = Launcher::default();
    serde_json::json!({
        "modes": l.modes,
        "burst_count": l.burst_count,
        "cyclic_rpm": l.cyclic_rpm,
        "reload_empty_s": l.reload_empty_s,
        "spare_magazines": l.spare_magazines,
        "recoil": l.recoil,
        "sway": l.sway,
        "ads_sway_factor": l.ads_sway_factor,
        "ads_dispersion_factor": l.ads_dispersion_factor,
    })
}

impl Launcher {
    /// The defaults of the handling fields (see [`launcher_handling_defaults`]).
    pub(crate) fn handling_defaults() -> (Vec<FireMode>, u32, f64) {
        (default_modes(), DEFAULT_BURST_COUNT, DEFAULT_ADS_FACTOR)
    }

    /// The fire modes, the burst, the cyclic rate, the reload from empty, recoil, sway and the
    /// factors of aiming down the sights.
    pub(crate) fn validate_handling(&self, ctx: Ctx) -> Result<(), WorldError> {
        let names = || {
            let names: Vec<&str> = self.modes.iter().map(|m| m.name()).collect();
            format!("[{}]", names.join(", "))
        };
        if self.modes.is_empty() || self.modes.len() > FireMode::NAMES.len() {
            return Err(ctx.invalid(
                "launcher.modes",
                names(),
                "it must list one to four of \"safe\", \"semi\", \"auto\" and \"burst\"",
            ));
        }
        for (i, m) in self.modes.iter().enumerate() {
            if self.modes.iter().take(i).any(|o| o == m) {
                return Err(ctx.invalid(
                    "launcher.modes",
                    names(),
                    format!("\"{}\" is listed twice; list each mode once", m.name()),
                ));
            }
        }
        if !(1..=BURST_COUNT_MAX).contains(&self.burst_count) {
            return Err(ctx.invalid(
                "launcher.burst_count",
                self.burst_count,
                format!("it must be from 1 to {BURST_COUNT_MAX}"),
            ));
        }
        if let Some(rpm) = self.cyclic_rpm {
            above_zero(
                ctx,
                "launcher.cyclic_rpm",
                rpm,
                CYCLIC_RPM_MAX,
                " rounds per minute",
            )?;
        }
        if let Some(s) = self.reload_empty_s {
            ctx.range(
                "launcher.reload_empty_s",
                s,
                0.0,
                LAUNCHER_TIME_MAX_S,
                " seconds",
            )?;
        }
        if let Some(n) = self.spare_magazines
            && n > MAGAZINE_MAX
        {
            return Err(ctx.invalid(
                "launcher.spare_magazines",
                n,
                format!("it must be from 0 to {MAGAZINE_MAX}, or null for unlimited"),
            ));
        }
        self.recoil.validate(ctx)?;
        self.sway.validate(ctx)?;
        ctx.range(
            "launcher.ads_sway_factor",
            self.ads_sway_factor,
            0.0,
            1.0,
            "",
        )?;
        ctx.range(
            "launcher.ads_dispersion_factor",
            self.ads_dispersion_factor,
            0.0,
            1.0,
            "",
        )
    }

    /// Seconds between two shots in `mode`: the cyclic rate's interval for `auto` and `burst`
    /// (`rate_s` without one), `rate_s` for `semi` and `safe`.
    pub fn interval_s(&self, mode: FireMode) -> f64 {
        match (mode, self.cyclic_rpm) {
            (FireMode::Auto | FireMode::Burst, Some(rpm)) if rpm > 0.0 => 60.0 / rpm,
            _ => self.rate_s,
        }
    }

    /// Seconds a reload takes from an empty magazine: `reload_empty_s`, else `reload_s`.
    pub fn empty_reload_s(&self) -> f64 {
        self.reload_empty_s.unwrap_or(self.reload_s)
    }

    /// The mode at the start: the first of `modes`.
    pub fn first_mode(&self) -> FireMode {
        self.modes.first().copied().unwrap_or_default()
    }

    /// The mode after `mode` in `modes`, round again after the last; the first mode when
    /// `mode` is not among them.
    pub fn next_mode(&self, mode: FireMode) -> FireMode {
        match self.modes.iter().position(|m| *m == mode) {
            Some(i) => self
                .modes
                .get(i + 1)
                .or_else(|| self.modes.first())
                .copied()
                .unwrap_or(mode),
            None => self.first_mode(),
        }
    }
}
