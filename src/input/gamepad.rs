//! Gamepad state — buttons + analog axes with deadzone applied.

use super::deadzone::apply_deadzone;

/// Standard gamepad buttons (Xbox-layout naming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadButton {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    Back,
    Start,
    LStick,
    RStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// Analog axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

/// One gamepad's live state.
#[derive(Debug, Clone)]
pub struct GamepadState {
    /// Backend-assigned id (0 = first pad).
    pub id: u32,
    /// Held buttons.
    pub buttons: Vec<GamepadButton>,
    /// Raw axis values (-1..=1 for sticks, 0..=1 for triggers).
    pub axes: Vec<(GamepadAxis, f32)>,
    /// Deadzone applied to stick axes (0.0..0.5 typical).
    pub deadzone: f32,
}

impl GamepadState {
    /// New state for pad `id`.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            buttons: Vec::new(),
            axes: Vec::new(),
            deadzone: 0.15,
        }
    }

    /// Set an axis raw value.
    pub fn set_axis(&mut self, axis: GamepadAxis, value: f32) {
        if let Some(e) = self.axes.iter_mut().find(|(a, _)| *a == axis) {
            e.1 = value;
        } else {
            self.axes.push((axis, value));
        }
    }

    /// Read an axis with deadzone applied.
    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        let raw = self
            .axes
            .iter()
            .find(|(a, _)| *a == axis)
            .map(|(_, v)| *v)
            .unwrap_or(0.0);
        apply_deadzone(raw, self.deadzone)
    }

    /// Is a button held?
    pub fn is_down(&self, b: GamepadButton) -> bool {
        self.buttons.contains(&b)
    }

    /// Press/release helpers.
    pub fn press(&mut self, b: GamepadButton) {
        if !self.buttons.contains(&b) {
            self.buttons.push(b);
        }
    }
    pub fn release(&mut self, b: GamepadButton) {
        self.buttons.retain(|x| *x != b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_applied_on_read() {
        let mut g = GamepadState::new(0);
        g.deadzone = 0.2;
        g.set_axis(GamepadAxis::LeftX, 0.1); // inside deadzone
        assert_eq!(g.axis(GamepadAxis::LeftX), 0.0);
        g.set_axis(GamepadAxis::LeftX, 0.6);
        assert!(g.axis(GamepadAxis::LeftX) > 0.4);
    }

    #[test]
    fn buttons_toggle() {
        let mut g = GamepadState::new(0);
        g.press(GamepadButton::A);
        assert!(g.is_down(GamepadButton::A));
        g.release(GamepadButton::A);
        assert!(!g.is_down(GamepadButton::A));
    }
}
