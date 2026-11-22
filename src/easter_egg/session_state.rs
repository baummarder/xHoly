//! Per-session "already shown" state.
//!
//! No persistence — a fresh process always starts with `shown = false`,
//! so the egg appears once per session by design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Session-scoped flag guarding the once-per-session display.
#[derive(Debug, Default)]
pub struct SessionState {
    shown: AtomicBool,
}

impl SessionState {
    /// Process-global session state.
    pub fn global() -> &'static Self {
        static STATE: OnceLock<SessionState> = OnceLock::new();
        STATE.get_or_init(SessionState::default)
    }

    /// Has the egg already been shown this session?
    pub fn was_shown(&self) -> bool {
        self.shown.load(Ordering::Relaxed)
    }

    /// Mark as shown; returns `true` if this call won the race
    /// (i.e. the egg should actually be displayed).
    pub fn mark_shown(&self) -> bool {
        !self.shown.swap(true, Ordering::Relaxed)
    }

    /// Test hook: clear the flag (documented; not for app use).
    pub fn reset(&self) {
        self.shown.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_once() {
        let s = SessionState::default();
        assert!(!s.was_shown());
        assert!(s.mark_shown());
        assert!(s.was_shown());
        assert!(!s.mark_shown());
    }
}
