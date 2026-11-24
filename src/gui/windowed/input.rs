//! Input handling for the windowed GUI — keyboard, mouse, touch.

use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase};
use winit::keyboard::Key;

/// High-level GUI actions produced from raw input events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiAction {
    /// ESC — pause / open menu.
    Pause,
    /// F1 — screenshot.
    Screenshot,
    /// F5 — save state.
    SaveState,
    /// F9 — load state.
    LoadState,
    /// Eject disc.
    Eject,
    /// Toggle cheat engine.
    ToggleCheats,
    /// Launch selected game.
    Launch,
    /// Move selection down.
    NextGame,
    /// Move selection up.
    PrevGame,
    /// Toggle fullscreen.
    ToggleFullscreen,
    /// Toggle the settings panel.
    ToggleSettings,
    /// Ctrl+Alt+T: open or close the emulation menu.
    EmulationMenu,
    /// Quit the app.
    Quit,
}

/// Aggregated input state for one frame.
#[derive(Debug, Default)]
pub struct InputState {
    /// Current keyboard modifiers (Ctrl/Alt/Shift).
    modifiers: winit::keyboard::ModifiersState,
    /// Actions triggered since the last frame.
    pub actions: Vec<GuiAction>,
    /// Mouse position (physical pixels).
    pub mouse: (f64, f64),
    /// Left button currently held.
    pub mouse_down: bool,
    /// Buttons currently held for the guest (Phase 22): bit0 left,
    /// bit1 right, bit2 up, bit3 down, bit4 A.
    pub guest_buttons: u32,
}

impl InputState {
    /// Create an empty input state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Guest-facing button bits for a keyboard key (0 = none).
    fn guest_bit(code: &winit::keyboard::Key) -> u32 {
        match code {
            Key::Named(winit::keyboard::NamedKey::ArrowLeft) => 1 << 0,
            Key::Named(winit::keyboard::NamedKey::ArrowRight) => 1 << 1,
            Key::Named(winit::keyboard::NamedKey::ArrowUp) => 1 << 2,
            Key::Named(winit::keyboard::NamedKey::ArrowDown) => 1 << 3,
            Key::Character(s) if s.eq_ignore_ascii_case("a") => 1 << 4,
            _ => 0,
        }
    }

    /// Map a winit keyboard event to GUI actions and guest buttons.
    /// Track Ctrl/Alt/Shift so combinations can be recognised.
    pub fn modifiers_changed(&mut self, m: winit::keyboard::ModifiersState) {
        self.modifiers = m;
    }

    pub fn keyboard_input(&mut self, event: &KeyEvent) {
        // Ctrl+Alt+T opens the emulation menu (return to main menu).
        if event.state == ElementState::Pressed
            && self.modifiers.control_key()
            && self.modifiers.alt_key()
            && matches!(&event.logical_key, Key::Character(s) if s.eq_ignore_ascii_case("t"))
        {
            self.actions.push(GuiAction::EmulationMenu);
            return;
        }
        let bit = Self::guest_bit(&event.logical_key);
        if bit != 0 {
            if event.state == ElementState::Pressed {
                self.guest_buttons |= bit;
            } else {
                self.guest_buttons &= !bit;
            }
        }
        if event.state != ElementState::Pressed {
            return;
        }
        let code = event.logical_key.clone();
        let action = match &code {
            Key::Named(winit::keyboard::NamedKey::Escape) => Some(GuiAction::Pause),
            Key::Character(s) if s.eq_ignore_ascii_case("p") => Some(GuiAction::Pause),
            Key::Character(s) if s.eq_ignore_ascii_case("f") => None, // handled below via F-keys
            Key::Character(s) if s.eq_ignore_ascii_case("e") => Some(GuiAction::Eject),
            Key::Character(s) if s.eq_ignore_ascii_case("c") => Some(GuiAction::ToggleCheats),
            Key::Character(s) if s.eq_ignore_ascii_case("q") => Some(GuiAction::Quit),
            Key::Named(winit::keyboard::NamedKey::Enter) => Some(GuiAction::Launch),
            Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | Key::Named(winit::keyboard::NamedKey::ArrowRight) => Some(GuiAction::NextGame),
            Key::Named(winit::keyboard::NamedKey::ArrowUp)
            | Key::Named(winit::keyboard::NamedKey::ArrowLeft) => Some(GuiAction::PrevGame),
            Key::Named(winit::keyboard::NamedKey::F1) => Some(GuiAction::Screenshot),
            Key::Named(winit::keyboard::NamedKey::F5) => Some(GuiAction::SaveState),
            Key::Named(winit::keyboard::NamedKey::F9) => Some(GuiAction::LoadState),
            Key::Named(winit::keyboard::NamedKey::F11) => Some(GuiAction::ToggleFullscreen),
            Key::Character(s) if s.eq_ignore_ascii_case("o") => Some(GuiAction::ToggleSettings),
            _ => None,
        };
        if let Some(a) = action {
            self.actions.push(a);
        }
    }

    /// Map a mouse button event.
    pub fn mouse_input(&mut self, button: MouseButton, state: ElementState) {
        if button == MouseButton::Left {
            self.mouse_down = state == ElementState::Pressed;
        }
    }

    /// Map a mouse-motion event (position in physical pixels).
    pub fn mouse_motion(&mut self, x: f64, y: f64) {
        self.mouse = (x, y);
    }

    /// Map a scroll event — wheel up/down moves game selection.
    pub fn mouse_scroll(&mut self, delta: MouseScrollDelta) {
        let dy = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
        };
        if dy < -0.5 {
            self.actions.push(GuiAction::NextGame);
        } else if dy > 0.5 {
            self.actions.push(GuiAction::PrevGame);
        }
    }

    /// Map a touch event (basic: tap = click at position).
    pub fn touch_input(&mut self, phase: TouchPhase, x: f64, y: f64) {
        match phase {
            TouchPhase::Started | TouchPhase::Moved => {
                self.mouse = (x, y);
                self.mouse_down = phase == TouchPhase::Started;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.mouse_down = false;
            }
        }
    }

    /// Drain accumulated actions (call after processing a frame).
    pub fn drain_actions(&mut self) -> Vec<GuiAction> {
        std::mem::take(&mut self.actions)
    }
}
