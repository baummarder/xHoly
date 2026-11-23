//! Easter egg rendering — ANSI console output.
//!
//! Trans-flag blue (#5BCEFA → ANSI 111) on deep blue-black. Frames are
//! plain text lines; [`render_frame`] is pure and unit-testable.

use super::config::EasterEggConfig;

/// ASCII-art logo (small, corner-sized).
pub const LOGO: &str = r#"
   __  ___  _  _ ____ ___ ____
  /  \(  _ \/ )( ( ___/ ___)
  \ / | |_) )__| |  _ \___ \
   X  |____/\__/|_|_|_|____/
      xHoly — research scaffold
"#;

/// ANSI trans-flag blue foreground (legacy name kept for API compatibility).
pub const GOLD: &str = "\x1b[38;5;111m";
/// ANSI trans-flag pink foreground.
pub const PINK: &str = "\x1b[38;5;213m";
/// ANSI deep blue-black background.
pub const BG: &str = "\x1b[48;5;233m";
/// Reset.
pub const RESET: &str = "\x1b[0m";
/// Bold.
pub const BOLD: &str = "\x1b[1m";

/// Emoji that bobs vertically at ~2 Hz.
pub const EGG_EMOJI: char = '🎉';

/// Info line collected lazily by the engine (GPU name, commit).
#[derive(Debug, Default, Clone)]
pub struct EggInfo {
    /// GPU adapter name when detection succeeded.
    pub gpu_name: Option<String>,
    /// Short commit hash when `git` is available.
    pub commit: Option<String>,
}

/// Approximate sine via a cheap triangle wave is fine for a bob —
/// we use real `f64::sin` anyway (2 Hz over ~30 frames is cheap).
fn bob_offset(t_norm: f64) -> i32 {
    // ±1 row bob at 2 Hz.
    (t_norm * std::f64::consts::TAU * 2.0).sin().round() as i32
}

/// Fade factor 0.0→1.0→0.0 across the whole duration (first/last 15%).
pub fn fade(t_norm: f64) -> f64 {
    let edge = 0.15;
    if t_norm < edge {
        t_norm / edge
    } else if t_norm > 1.0 - edge {
        (1.0 - t_norm) / edge
    } else {
        1.0
    }
}

/// Render one frame of the egg as text lines (without ANSI colors —
/// colors are applied by the engine so tests can assert on content).
pub fn render_frame(config: &EasterEggConfig, info: &EggInfo, t_norm: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let _ = fade(t_norm); // fade is applied at the ANSI layer

    if config.show_logo {
        lines.push(String::new());
        for l in LOGO.lines() {
            lines.push(format!("  {l}"));
        }
        lines.push(String::new());
    }

    lines.push(format!("  {}", config.message));

    if config.show_dev_mode {
        lines.push("  xHoly v0.1.0-dev — Phase 10: Shader Translation + Kernel".to_string());
    }

    if config.show_gpu_info {
        if let Some(g) = &info.gpu_name {
            lines.push(format!("  GPU: {g}"));
        }
    }
    if config.show_commit_hash {
        if let Some(c) = &info.commit {
            lines.push(format!("  commit: {c}"));
        }
    }

    if config.show_emoji {
        // Bob the emoji one row up/down; render as its own line after
        // a spacer so the motion is visible in a static frame dump.
        let off = bob_offset(t_norm);
        let pad = if off < 0 {
            format!("{}{}", " ".repeat((-off) as usize * 4), EGG_EMOJI)
        } else {
            format!("{}{}", " ".repeat(off as usize * 4), EGG_EMOJI)
        };
        lines.push(String::new());
        lines.push(format!("  {pad}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_contains_message() {
        let cfg = EasterEggConfig::default();
        let info = EggInfo::default();
        let frame = render_frame(&cfg, &info, 0.5);
        assert!(frame
            .iter()
            .any(|l| l.contains("Oh shit here we go again!")));
    }

    #[test]
    fn minimal_config_has_no_logo_or_emoji() {
        let cfg = EasterEggConfig::minimal();
        let info = EggInfo::default();
        let frame = render_frame(&cfg, &info, 0.5);
        assert!(!frame.iter().any(|l| l.contains('🎉')));
        assert!(frame
            .iter()
            .all(|l| !l.contains("xHoly — research scaffold")));
    }

    #[test]
    fn fade_ramps() {
        assert!(fade(0.0) < 0.01);
        assert!((fade(0.5) - 1.0).abs() < 0.01);
        assert!(fade(1.0) < 0.01);
    }

    #[test]
    fn bob_moves() {
        assert_ne!(bob_offset(0.0), bob_offset(0.125));
    }
}
