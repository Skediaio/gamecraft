//! A launcher in a character's hands: the trigger and the selector, recoil and sway.
//!
//! Every tick, for every character with a launcher (after the characters moved, before it
//! fires; see [`crate::combat`]):
//!
//! - **Selector and reload.** The input's `switch_mode` going from false to true moves the
//!   selector to the next of the launcher's `modes`; `reload` going true starts a reload of
//!   `reload_s` when the magazine is not full and not reloading already.
//! - **Recoil.** The kick left of earlier shots decays: it is multiplied by
//!   `exp(-DT * RECOVERY_TIME_CONSTANTS / recovery_s)` (0 when `recovery_s` is 0), so after
//!   `recovery_s` about 5 % of it is left.
//! - **Sway.** The amplitude asked for is the stance's (`standing_mrad`; characters have no
//!   other stance yet, [`stance_of`] is where one will come from), times `moving_factor` while
//!   the body moves faster than [`MOVING_SPEED_MPS`], times `1 + (fatigue_factor - 1) * f` for
//!   the fatigue `f` (0 to 1, up by `DT / fatigue_build_s` per tick of running, down by
//!   `DT / fatigue_fade_s` otherwise), times `ads_sway_factor` while the input holds `ads`,
//!   times the breath's factor. The amplitude follows that over [`SWAY_SMOOTHING_S`] (an
//!   exponential approach), so nothing jumps. The drift is `amplitude * d(t)` on each axis, with
//!   `d(t)` the weighted sum of [`SWAY_PARTS`] sines of `2 pi t / (period_s * share) + phase`,
//!   whose weights sum to 1 (so `|d| <= 1`), the six phases drawn from the world's seed forked
//!   by [`LABEL_SWAY`] and the character's id: the same world gives the same path.
//! - **Breath.** Holding `hold_breath` steadies the sway to `breath_steady_factor` for at most
//!   `breath_hold_s`; letting go, or running out, gives `breath_recover_s` of
//!   `breath_recover_factor` before the breath can be held again.
//! - **Trigger.** A shot leaves along the input's aim plus the recoil plus the sway, with the
//!   dispersion times `ads_dispersion_factor` while aiming down the sights. `safe` never fires;
//!   `semi` fires once per pull (holding the trigger through the wait for `rate_s` fires once
//!   when ready); `burst` fires up to `burst_count` per pull at the cyclic rate; `auto` while
//!   held. Letting go of the trigger, or moving the selector, ends a pull.
//! - **Kick.** Each shot adds `kick_up_mrad` up and `kick_side_mrad * (2u - 1)` to the side,
//!   `u` drawn from the world's seed forked by [`LABEL_RECOIL`], the shooter's id and the shot's
//!   number. The offset is capped at [`MAX_RECOIL_MRAD`] on each axis.
//!
//! Angles here are milliradians, `[left, up]`: left as the yaw turns (counter-clockwise seen
//! from above), up as the pitch tilts.

use gamecraft_math::{Portable, Rng};
use gamecraft_world::{Entity, EntityId, FireMode, Launcher};
use serde::Serialize;

use crate::{DT, Input};

/// The label of the recoil's random stream (forked from the world seed).
pub const LABEL_RECOIL: u64 = 0x2EC0_11C1;
/// The label of the sway's phases (forked from the world seed).
pub const LABEL_SWAY: u64 = 0x05A7_5A7E;
/// How many time constants of the recoil's decay make its `recovery_s`: three leave 5 %.
pub const RECOVERY_TIME_CONSTANTS: f64 = 3.0;
/// The time constant with which the sway's amplitude follows what is asked of it, seconds.
pub const SWAY_SMOOTHING_S: f64 = 0.2;
/// A body faster than this sways as moving, metres per second.
pub const MOVING_SPEED_MPS: f64 = 0.1;
/// The largest recoil offset on each axis, milliradians.
pub const MAX_RECOIL_MRAD: f64 = 1000.0;
/// The parts of the sway's drift: (share of `period_s`, weight). The weights sum to 1.
pub const SWAY_PARTS: [(f64, f64); 3] = [(1.0, 0.5), (0.53, 0.3), (0.29, 0.2)];

/// How a character stands, for its sway.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stance {
    /// On its feet (every character, for now).
    #[default]
    Standing,
    /// Crouched.
    Crouched,
    /// Lying down.
    Prone,
}

/// The stance of a character. Characters have no stances yet, so every one stands; this is the
/// one place a stance will come from.
pub fn stance_of(_entity: &Entity) -> Stance {
    Stance::Standing
}

/// Where a character's breath is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Breath {
    /// Breathing.
    #[default]
    Normal,
    /// Held: the sway is steadier.
    Holding,
    /// After holding it: the sway is worse for a while.
    Recovering,
}

impl Breath {
    /// The name as in JSON.
    pub fn name(self) -> &'static str {
        match self {
            Breath::Normal => "normal",
            Breath::Holding => "holding",
            Breath::Recovering => "recovering",
        }
    }
}

/// The state of a launcher in a character's hands that is not in the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Handling {
    /// Shots of `semi` or `burst` fired since the trigger was pulled (0 while it is not held,
    /// after a switch of the selector, and in `auto`).
    pub pulled: u32,
    /// The input held `switch_mode` last tick.
    pub switch_held: bool,
    /// The input held `reload` last tick.
    pub reload_held: bool,
    /// The recoil's offset of the aim, `[left, up]`, milliradians.
    pub recoil_mrad: [f64; 2],
    /// The sway's offset of the aim, `[left, up]`, milliradians.
    pub sway_mrad: [f64; 2],
    /// The sway's amplitude now, milliradians.
    pub sway_amplitude_mrad: f64,
    /// Fatigue from running, 0 to 1.
    pub fatigue: f64,
    /// Where the breath is.
    pub breath: Breath,
    /// Seconds the breath has been held (holding), or seconds left of recovering.
    pub breath_s: f64,
    /// Aiming down the sights.
    pub ads: bool,
    /// The stance the sway was taken for.
    pub stance: Stance,
}

/// What a tick of handling needs to know of the character.
pub(crate) struct Moment<'a> {
    /// Its stored input; `None` without one (or while it is down).
    pub(crate) input: Option<&'a Input>,
    /// Its body's horizontal speed, metres per second.
    pub(crate) speed_mps: f64,
    /// Its stance.
    pub(crate) stance: Stance,
    /// The simulated time at the end of the tick, seconds.
    pub(crate) time_s: f64,
    /// The phases of its sway ([`sway_phases`]).
    pub(crate) phases: &'a [f64; 6],
}

/// The six phases of a character's sway, radians: three for the left drift, three for the up.
pub fn sway_phases(seed: u64, id: EntityId) -> [f64; 6] {
    let mut rng = Rng::new(seed).fork(LABEL_SWAY).fork(id.0);
    [(); 6].map(|()| 2.0 * core::f64::consts::PI * rng.unit())
}

/// The sway's unit drift at `time_s`: the weighted sum of [`SWAY_PARTS`] sines with `phases`,
/// within -1 to 1.
pub fn drift(time_s: f64, period_s: f64, phases: &[f64]) -> f64 {
    if !(period_s.is_finite() && period_s > 0.0 && time_s.is_finite()) {
        return 0.0;
    }
    let mut sum = 0.0;
    for ((share, weight), phase) in SWAY_PARTS.iter().zip(phases) {
        let angle = 2.0 * core::f64::consts::PI * time_s / (period_s * share) + phase;
        sum += weight * angle.psin();
    }
    if sum.is_finite() {
        sum.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The share of a kick left after one tick of decay.
pub fn recoil_decay(recovery_s: f64) -> f64 {
    if recovery_s.is_finite() && recovery_s > 0.0 {
        (-DT * RECOVERY_TIME_CONSTANTS / recovery_s).pexp()
    } else {
        0.0
    }
}

/// Whether `launcher` sways at all (some stance has an amplitude).
pub fn sways(launcher: &Launcher) -> bool {
    let s = &launcher.sway;
    s.standing_mrad > 0.0 || s.crouched_mrad > 0.0 || s.prone_mrad > 0.0
}

/// `value` when it is finite, else 0.
fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

impl Handling {
    /// The offset of the aim, recoil plus sway, `[left, up]`, milliradians.
    pub fn offset_mrad(&self) -> [f64; 2] {
        [
            self.recoil_mrad[0] + self.sway_mrad[0],
            self.recoil_mrad[1] + self.sway_mrad[1],
        ]
    }

    /// An offset in milliradians as (yaw, pitch) degrees.
    pub fn to_deg([left, up]: [f64; 2]) -> (f64, f64) {
        (
            finite_or_zero((left * 1e-3).to_degrees()),
            finite_or_zero((up * 1e-3).to_degrees()),
        )
    }

    /// Counts a shot of the current pull: in `semi` and `burst`, which fire a number of shots
    /// per pull (`auto` does not count them).
    pub(crate) fn count_shot(&mut self, mode: FireMode) {
        if matches!(mode, FireMode::Semi | FireMode::Burst) {
            self.pulled = self.pulled.saturating_add(1);
        }
    }

    /// Whether the trigger may fire now in `mode`, after `pulled` shots of this pull.
    pub fn allows(&self, mode: FireMode, burst_count: u32) -> bool {
        match mode {
            FireMode::Safe => false,
            FireMode::Semi => self.pulled == 0,
            FireMode::Burst => self.pulled < burst_count,
            FireMode::Auto => true,
        }
    }

    /// One tick: the recoil decays, the fatigue and the breath move on, the sway follows.
    pub(crate) fn tick(&mut self, launcher: &Launcher, now: &Moment<'_>) {
        let decay = recoil_decay(launcher.recoil.recovery_s);
        self.recoil_mrad = self.recoil_mrad.map(|v| finite_or_zero(v * decay));
        let input = now.input.copied().unwrap_or_default();
        if !input.fire {
            self.pulled = 0;
        }
        self.ads = input.ads;
        self.stance = now.stance;
        let sway = &launcher.sway;
        let moving = now.speed_mps.is_finite() && now.speed_mps > MOVING_SPEED_MPS;
        let running = moving && input.run;
        if sways(launcher) {
            let fatigue = if running {
                self.fatigue + DT / sway.fatigue_build_s
            } else {
                self.fatigue - DT / sway.fatigue_fade_s
            };
            self.fatigue = finite_or_zero(fatigue).clamp(0.0, 1.0);
            self.breathe(input.hold_breath, launcher);
        } else {
            // Without a sway there is nothing for fatigue or the breath to change.
            self.fatigue = 0.0;
            self.breath = Breath::Normal;
            self.breath_s = 0.0;
        }
        let stance_mrad = match self.stance {
            Stance::Standing => sway.standing_mrad,
            Stance::Crouched => sway.crouched_mrad,
            Stance::Prone => sway.prone_mrad,
        };
        let breath = match self.breath {
            Breath::Normal => 1.0,
            Breath::Holding => sway.breath_steady_factor,
            Breath::Recovering => sway.breath_recover_factor,
        };
        let wanted = stance_mrad
            * if moving { sway.moving_factor } else { 1.0 }
            * (1.0 + (sway.fatigue_factor - 1.0) * self.fatigue)
            * if self.ads {
                launcher.ads_sway_factor
            } else {
                1.0
            }
            * breath;
        let follow = 1.0 - (-DT / SWAY_SMOOTHING_S).pexp();
        let amplitude =
            self.sway_amplitude_mrad + (wanted.max(0.0) - self.sway_amplitude_mrad) * follow;
        self.sway_amplitude_mrad = finite_or_zero(amplitude).max(0.0);
        let (left, up) = now.phases.split_at(SWAY_PARTS.len());
        self.sway_mrad = [
            self.sway_amplitude_mrad * drift(now.time_s, sway.period_s, left),
            self.sway_amplitude_mrad * drift(now.time_s, sway.period_s, up),
        ];
    }

    /// One tick of the breath.
    fn breathe(&mut self, hold: bool, launcher: &Launcher) {
        let sway = &launcher.sway;
        match self.breath {
            Breath::Normal => {
                if hold {
                    self.breath = Breath::Holding;
                    self.breath_s = 0.0;
                }
            }
            Breath::Holding => {
                self.breath_s += DT;
                if !hold || self.breath_s >= sway.breath_hold_s {
                    if sway.breath_recover_s > 0.0 {
                        self.breath = Breath::Recovering;
                        self.breath_s = sway.breath_recover_s;
                    } else {
                        self.breath = Breath::Normal;
                        self.breath_s = 0.0;
                    }
                }
            }
            Breath::Recovering => {
                self.breath_s -= DT;
                if self.breath_s <= 0.0 {
                    self.breath = Breath::Normal;
                    self.breath_s = 0.0;
                }
            }
        }
        self.breath_s = finite_or_zero(self.breath_s);
    }

    /// The kick of shot number `shot` of the launcher of `shooter` in the world of `seed`.
    pub(crate) fn kick(&mut self, launcher: &Launcher, seed: u64, shooter: EntityId, shot: u64) {
        let recoil = &launcher.recoil;
        let mut rng = Rng::new(seed).fork(LABEL_RECOIL).fork(shooter.0).fork(shot);
        let side = recoil.kick_side_mrad * (2.0 * rng.unit() - 1.0);
        let [left, up] = self.recoil_mrad;
        self.recoil_mrad = [
            finite_or_zero(left + side).clamp(-MAX_RECOIL_MRAD, MAX_RECOIL_MRAD),
            finite_or_zero(up + recoil.kick_up_mrad).clamp(-MAX_RECOIL_MRAD, MAX_RECOIL_MRAD),
        ];
    }
}
