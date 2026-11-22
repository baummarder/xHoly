//! Easter egg configuration.

/// Runtime configuration for the start screen.
#[derive(Debug, Clone)]
pub struct EasterEggConfig {
    /// Total visible time in milliseconds (default 2500).
    pub duration_ms: u64,
    /// Main message line.
    pub message: &'static str,
    /// Show the bobbing emoji row.
    pub show_emoji: bool,
    /// Show the dev-mode subtitle.
    pub show_dev_mode: bool,
    /// Show the ASCII logo box.
    pub show_logo: bool,
    /// Show detected GPU adapter name (best effort, wgpu).
    pub show_gpu_info: bool,
    /// Show the git commit hash (best effort).
    pub show_commit_hash: bool,
}

impl Default for EasterEggConfig {
    fn default() -> Self {
        Self {
            duration_ms: 2500,
            message: "Oh shit here we go again!",
            show_emoji: true,
            show_dev_mode: true,
            show_logo: true,
            show_gpu_info: true,
            show_commit_hash: true,
        }
    }
}

impl EasterEggConfig {
    /// Text-only fallback config (no logo/emoji/info lines).
    pub fn minimal() -> Self {
        Self {
            duration_ms: 1200,
            show_emoji: false,
            show_dev_mode: false,
            show_logo: false,
            show_gpu_info: false,
            show_commit_hash: false,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = EasterEggConfig::default();
        assert_eq!(c.duration_ms, 2500);
        assert_eq!(c.message, "Oh shit here we go again!");
        assert!(c.show_emoji && c.show_dev_mode && c.show_logo);
    }
}
