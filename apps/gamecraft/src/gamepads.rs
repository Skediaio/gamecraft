//! Reads the connected gamepad with `gilrs` (natively and, through the browser's Gamepad API,
//! on the web) into a [`PadState`]. The mapping from there on is the pure [`crate::pad`].

use crate::pad::PadState;
#[cfg(any(target_arch = "wasm32", feature = "native-gamepad"))]
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};

/// The gamepads of the machine: the one that was touched last is the one that plays.
pub struct Pads {
    #[cfg(any(target_arch = "wasm32", feature = "native-gamepad"))]
    gilrs: Option<Gilrs>,
    #[cfg(any(target_arch = "wasm32", feature = "native-gamepad"))]
    active: Option<GamepadId>,
}

/// Natively without the `native-gamepad` feature there is no backend: no pad ever reports.
#[cfg(not(any(target_arch = "wasm32", feature = "native-gamepad")))]
impl Pads {
    /// No backend.
    pub fn new() -> Pads {
        Pads {}
    }

    /// Always the neutral state.
    pub fn poll(&mut self) -> PadState {
        PadState::default()
    }
}

#[cfg(any(target_arch = "wasm32", feature = "native-gamepad"))]
impl Pads {
    /// Opens the gamepad backend; without one (an unsupported platform) no pad ever reports.
    pub fn new() -> Pads {
        let gilrs = Gilrs::new().ok();
        let active = gilrs.as_ref().and_then(|g| {
            g.gamepads()
                .find(|(_, pad)| pad.is_connected())
                .map(|(id, _)| id)
        });
        Pads { gilrs, active }
    }

    /// Takes in the events since the last call and reads the active pad; the neutral state
    /// when no pad is connected.
    pub fn poll(&mut self) -> PadState {
        let Some(gilrs) = &mut self.gilrs else {
            return PadState::default();
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Disconnected if self.active == Some(event.id) => self.active = None,
                EventType::Disconnected | EventType::Dropped => {}
                _ => self.active = Some(event.id),
            }
        }
        if self.active.is_none() {
            self.active = gilrs
                .gamepads()
                .find(|(_, pad)| pad.is_connected())
                .map(|(id, _)| id);
        }
        let Some(pad) = self.active.map(|id| gilrs.gamepad(id)) else {
            return PadState::default();
        };
        if !pad.is_connected() {
            return PadState::default();
        }
        let axis = |a: Axis| f64::from(pad.value(a));
        let trigger = |b: Button| {
            pad.button_data(b).map_or_else(
                || f64::from(u8::from(pad.is_pressed(b))),
                |d| f64::from(d.value()),
            )
        };
        PadState {
            left_x: axis(Axis::LeftStickX),
            left_y: axis(Axis::LeftStickY),
            right_x: axis(Axis::RightStickX),
            right_y: axis(Axis::RightStickY),
            left_trigger: trigger(Button::LeftTrigger2),
            right_trigger: trigger(Button::RightTrigger2),
            left_thumb: pad.is_pressed(Button::LeftThumb),
            south: pad.is_pressed(Button::South),
            west: pad.is_pressed(Button::West),
            north: pad.is_pressed(Button::North),
            east: pad.is_pressed(Button::East),
            start: pad.is_pressed(Button::Start),
            select: pad.is_pressed(Button::Select),
        }
    }
}

impl Default for Pads {
    fn default() -> Pads {
        Pads::new()
    }
}
