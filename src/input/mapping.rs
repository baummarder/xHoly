//! Action mapping — bind logical actions to physical inputs.

use std::collections::HashMap;

/// A logical action the game/GUI cares about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Named custom action.
    Custom(String),
    /// Common emulator actions.
    Confirm,
    Cancel,
    Menu,
    Screenshot,
    SaveState,
    LoadState,
    FastForward,
    Pause,
}

/// What a physical input is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Binding {
    /// Keyboard key name (e.g. "Space", "F5").
    Key(String),
    /// Mouse button.
    Mouse(u8),
    /// Gamepad button (index into GamepadButton list order).
    Pad(u8),
}

/// Bidirectional-ish map: action → set of bindings.
#[derive(Debug, Default, Clone)]
pub struct ActionMap {
    map: HashMap<Action, Vec<Binding>>,
}

impl ActionMap {
    /// New empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `binding` to `action` (replaces existing bindings).
    pub fn bind(&mut self, action: Action, binding: Binding) {
        self.map.insert(action, vec![binding]);
    }

    /// Add an additional binding (multi-bind).
    pub fn add(&mut self, action: Action, binding: Binding) {
        self.map.entry(action).or_default().push(binding);
    }

    /// All bindings for an action.
    pub fn bindings(&self, action: &Action) -> &[Binding] {
        self.map.get(action).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Resolve: is `binding` currently triggering `action`?
    /// (Caller checks held-state; this just does lookup.)
    pub fn action_for(&self, binding: &Binding) -> Vec<Action> {
        self.map
            .iter()
            .filter(|(_, bs)| bs.contains(binding))
            .map(|(a, _)| a.clone())
            .collect()
    }

    /// Default emulator bindings.
    pub fn default_emulator() -> Self {
        let mut m = Self::new();
        m.bind(Action::Confirm, Binding::Key("Enter".into()));
        m.bind(Action::Confirm, Binding::Pad(0)); // A
        m.bind(Action::Cancel, Binding::Key("Escape".into()));
        m.bind(Action::Cancel, Binding::Pad(1)); // B
        m.bind(Action::Menu, Binding::Key("F1".into()));
        m.bind(Action::Screenshot, Binding::Key("F12".into()));
        m.bind(Action::SaveState, Binding::Key("F5".into()));
        m.bind(Action::LoadState, Binding::Key("F9".into()));
        m.bind(Action::FastForward, Binding::Key("Tab".into()));
        m.bind(Action::Pause, Binding::Key("P".into()));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_and_lookup() {
        let mut m = ActionMap::new();
        m.bind(Action::Confirm, Binding::Key("Enter".into()));
        m.add(Action::Confirm, Binding::Pad(0));
        assert_eq!(m.bindings(&Action::Confirm).len(), 2);
        let acts = m.action_for(&Binding::Pad(0));
        assert!(acts.contains(&Action::Confirm));
    }

    #[test]
    fn defaults_have_core_actions() {
        let m = ActionMap::default_emulator();
        for a in [
            Action::Confirm,
            Action::Cancel,
            Action::SaveState,
            Action::LoadState,
        ] {
            assert!(!m.bindings(&a).is_empty(), "missing {a:?}");
        }
    }
}
