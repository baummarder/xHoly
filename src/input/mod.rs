//! Central input system — Phase 11.5.
//!
//! Backend-agnostic input model: event types + state tracking +
//! action mapping + deadzone + press-state machine. The windowed GUI
//! (`gui::windowed::input`) adapts winit events into these types.

pub mod deadzone;
pub mod gamepad;
pub mod keyboard;
pub mod mapping;
pub mod mouse;
pub mod state_machine;

pub use deadzone::apply_deadzone;
pub use gamepad::{GamepadAxis, GamepadButton, GamepadState};
pub use keyboard::{KeyEvent, KeyboardState};
pub use mapping::{Action, ActionMap};
pub use mouse::{MouseButton, MouseState};
pub use state_machine::{PressEvent, PressState};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_link() {
        let mut kb = KeyboardState::default();
        kb.press("Space");
        assert!(kb.is_down("Space"));
        kb.release("Space");
        assert!(!kb.is_down("Space"));
    }
}
