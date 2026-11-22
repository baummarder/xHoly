//! Mouse state — position, buttons, wheel, drag delta.

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u8),
}

/// Live mouse state.
#[derive(Debug, Default, Clone)]
pub struct MouseState {
    /// Cursor position (logical pixels).
    pub x: f64,
    pub y: f64,
    /// Per-frame delta.
    pub dx: f64,
    pub dy: f64,
    /// Wheel delta (lines or pixels, backend-defined).
    pub wheel: f64,
    /// Currently held buttons.
    pub buttons: Vec<MouseButton>,
    /// Button that started the current drag, if any.
    pub drag_button: Option<MouseButton>,
}

impl MouseState {
    /// Move cursor (absolute).
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.dx = x - self.x;
        self.dy = y - self.y;
        self.x = x;
        self.y = y;
    }

    /// Relative motion (some backends send deltas).
    pub fn move_by(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.dx += dx;
        self.dy += dy;
    }

    /// Press a button.
    pub fn press(&mut self, b: MouseButton) {
        if !self.buttons.contains(&b) {
            self.buttons.push(b);
        }
        if self.drag_button.is_none() {
            self.drag_button = Some(b);
        }
    }

    /// Release a button.
    pub fn release(&mut self, b: MouseButton) {
        self.buttons.retain(|x| *x != b);
        if self.drag_button == Some(b) {
            self.drag_button = None;
        }
    }

    /// Is a button held?
    pub fn is_down(&self, b: MouseButton) -> bool {
        self.buttons.contains(&b)
    }

    /// Is any drag in progress?
    pub fn is_dragging(&self) -> bool {
        self.drag_button.is_some() && !self.buttons.is_empty()
    }

    /// End-of-frame reset (deltas/wheel clear, buttons persist).
    pub fn end_frame(&mut self) {
        self.dx = 0.0;
        self.dy = 0.0;
        self.wheel = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_lifecycle() {
        let mut m = MouseState::default();
        m.move_to(10.0, 10.0);
        m.press(MouseButton::Left);
        assert!(m.is_dragging());
        m.move_to(30.0, 25.0);
        assert_eq!(m.dx, 20.0);
        assert_eq!(m.dy, 15.0);
        m.release(MouseButton::Left);
        assert!(!m.is_dragging());
    }

    #[test]
    fn end_frame_clears_deltas_only() {
        let mut m = MouseState::default();
        m.move_by(5.0, 5.0);
        m.press(MouseButton::Right);
        m.end_frame();
        assert_eq!(m.dx, 0.0);
        assert!(m.is_down(MouseButton::Right));
    }
}
