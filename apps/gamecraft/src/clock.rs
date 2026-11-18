//! The fixed simulation step: frame time in, whole ticks out, and how far the picture is
//! between the last tick and the next.

/// The most ticks one frame runs. A slower frame drops the rest of its time, so a slow
/// machine plays slower instead of falling ever further behind (no spiral of death).
pub const MAX_TICKS_PER_FRAME: u32 = 4;
/// The longest frame counted, seconds (a window dragged or a tab in the background).
const MAX_FRAME_S: f64 = 0.25;
/// Rounding room when frame times add up to whole ticks, in ticks.
const TICK_EPSILON: f64 = 1e-9;

/// Frame time gathered into ticks of `dt` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedStep {
    dt: f64,
    accumulated: f64,
}

impl FixedStep {
    /// A step of `dt` seconds (the simulation's `DT`), nothing gathered.
    pub fn new(dt: f64) -> FixedStep {
        FixedStep {
            dt: if dt.is_finite() && dt > 0.0 {
                dt
            } else {
                gamecraft_sim::DT
            },
            accumulated: 0.0,
        }
    }

    /// Adds a frame of `frame_s` seconds (a time that is not finite or below 0 counts as 0,
    /// one over 0.25 s as 0.25 s) and returns the ticks to run now: at most
    /// [`MAX_TICKS_PER_FRAME`]; time beyond that is dropped.
    pub fn advance(&mut self, frame_s: f64) -> u32 {
        let frame_s = if frame_s.is_finite() && frame_s > 0.0 {
            frame_s.min(MAX_FRAME_S)
        } else {
            0.0
        };
        self.accumulated += frame_s;
        let due = (self.accumulated / self.dt + TICK_EPSILON).floor().max(0.0);
        self.accumulated = (self.accumulated - due * self.dt).clamp(0.0, self.dt);
        if due >= f64::from(MAX_TICKS_PER_FRAME) {
            MAX_TICKS_PER_FRAME
        } else {
            due as u32
        }
    }

    /// How far the present is between the last tick and the next, 0 to below 1.
    pub fn alpha(&self) -> f64 {
        (self.accumulated / self.dt).clamp(0.0, 1.0 - f64::EPSILON)
    }

    /// Forgets the gathered time (after a restart or a pause).
    pub fn reset(&mut self) {
        self.accumulated = 0.0;
    }
}
