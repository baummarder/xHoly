//! Keyboard state — pressed-key set with edge detection.

use std::collections::HashSet;

/// A keyboard event (backend-agnostic; `key` is a logical name like
/// "Space", "KeyA", "F1").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// Logical key name.
    pub key: String,
    /// Physical position when known (e.g. "KeyW").
    pub physical: Option<String>,
    /// True = pressed, false = released.
    pub pressed: bool,
    /// True if this is an OS auto-repeat.
    pub repeat: bool,
}

impl KeyEvent {
    /// Convenience constructor.
    pub fn new(key: impl Into<String>, pressed: bool) -> Self {
        Self {
            key: key.into(),
            physical: None,
            pressed,
            repeat: false,
        }
    }
}

/// Live keyboard state.
#[derive(Debug, Default, Clone)]
pub struct KeyboardState {
    down: HashSet<String>,
}

impl KeyboardState {
    /// Apply an event.
    pub fn apply(&mut self, ev: &KeyEvent) {
        if ev.pressed {
            self.down.insert(ev.key.clone());
        } else {
            self.down.remove(&ev.key);
        }
    }

    /// Is `key` currently held?
    pub fn is_down(&self, key: &str) -> bool {
        self.down.contains(key)
    }

    /// Press a key directly (tests / synthetic input).
    pub fn press(&mut self, key: &str) {
        self.down.insert(key.to_string());
    }

    /// Release a key directly.
    pub fn release(&mut self, key: &str) {
        self.down.remove(key);
    }

    /// Clear all state (focus loss).
    pub fn clear(&mut self) {
        self.down.clear();
    }

    /// Count of held keys.
    pub fn len(&self) -> usize {
        self.down.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.down.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_release_edges() {
        let mut kb = KeyboardState::default();
        kb.apply(&KeyEvent::new("A", true));
        assert!(kb.is_down("A"));
        kb.apply(&KeyEvent::new("A", false));
        assert!(!kb.is_down("A"));
    }

    #[test]
    fn clear_on_focus_loss() {
        let mut kb = KeyboardState::default();
        kb.press("B");
        kb.clear();
        assert!(kb.is_empty());
    }
}
