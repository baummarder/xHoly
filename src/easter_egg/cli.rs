//! Easter egg CLI mapping — `--easter-egg`, `--easter-egg-force`,
//! `--no-easter-egg`.

use super::config::EasterEggConfig;

/// What main() should do about the egg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EasterEggRequest {
    /// Show once per session (default behavior).
    Once,
    /// Show unconditionally, bypassing the once-guard.
    Force,
    /// Skip entirely.
    Off,
}

/// Resolve CLI flags into a request.
///
/// - `--no-easter-egg` wins over everything.
/// - `--easter-egg-force` bypasses the once-per-session guard.
/// - `--easter-egg` / default → once per session.
pub fn resolve(no_egg: bool, egg: bool, force: bool) -> EasterEggRequest {
    if no_egg {
        EasterEggRequest::Off
    } else if force {
        EasterEggRequest::Force
    } else {
        let _ = egg; // explicit flag == default behavior
        EasterEggRequest::Once
    }
}

/// Build the runtime config; `debug` downgrades to the minimal
/// text-only variant (shorter, no info probing).
pub fn config_for(debug: bool) -> EasterEggConfig {
    if debug {
        EasterEggConfig::minimal()
    } else {
        EasterEggConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_resolution() {
        assert_eq!(resolve(false, false, false), EasterEggRequest::Once);
        assert_eq!(resolve(false, true, false), EasterEggRequest::Once);
        assert_eq!(resolve(false, false, true), EasterEggRequest::Force);
        assert_eq!(resolve(true, false, true), EasterEggRequest::Off);
        assert_eq!(resolve(true, true, true), EasterEggRequest::Off);
    }

    #[test]
    fn debug_config_is_minimal() {
        assert_eq!(config_for(true).duration_ms, 1200);
        assert_eq!(config_for(false).duration_ms, 2500);
    }
}
