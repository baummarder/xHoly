//! Input device handling: controllers and hotkeys.

use serde::{Deserialize, Serialize};

/// Snapshot of a standard gamepad state (Xbox-layout buttons + sticks).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct GamepadState {
    pub buttons: u16,
    pub left_stick: (f32, f32),
    pub right_stick: (f32, f32),
    pub left_trigger: f32,
    pub right_trigger: f32,
}

/// Button bit positions (subset of the Xbox pad layout).
pub mod buttons {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const X: u16 = 1 << 2;
    pub const Y: u16 = 1 << 3;
    pub const DPAD_UP: u16 = 1 << 4;
    pub const DPAD_DOWN: u16 = 1 << 5;
    pub const DPAD_LEFT: u16 = 1 << 6;
    pub const DPAD_RIGHT: u16 = 1 << 7;
    pub const START: u16 = 1 << 8;
    pub const BACK: u16 = 1 << 9;
    pub const LEFT_SHOULDER: u16 = 1 << 10;
    pub const RIGHT_SHOULDER: u16 = 1 << 11;
    pub const LEFT_THUMB: u16 = 1 << 12;
    pub const RIGHT_THUMB: u16 = 1 << 13;
}

/// Hotkey actions handled by the emulator shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    Pause,
    Screenshot,
    QuickSave,
    QuickLoad,
    ToggleFullscreen,
    FastForward,
}

/// Poll input devices and return the current gamepad snapshot.
///
/// Scaffold: returns a neutral state. The real implementation polls
/// SDL2/XInput/RawHID per `InputConfig.api` at `polling_rate` Hz.
pub fn poll_gamepad() -> GamepadState {
    GamepadState::default()
}
