//! Easter egg — Phase 10 bonus.
//!
//! A short (~2.5 s), trans-blue-on-dark start screen ("Oh shit here we go
//! again!") shown once per session before the real app initializes.
//!
//! Scope (honest): console/ANSI rendering only — no GUI window, no
//! persistence, no i18n. See `docs/phase-10.md`.

pub mod cli;
pub mod config;
pub mod display;
pub mod engine;
pub mod session_state;

pub use cli::EasterEggRequest;
pub use config::EasterEggConfig;
pub use engine::EasterEgg;
pub use session_state::SessionState;
