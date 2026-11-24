//! Infrastructure-wide config sections (Phase 11.5).
//!
//! Complements [`super::settings::Config`] (which owns video/audio/
//! input/emulation) with the non-emulation subsystems: save, mods,
//! cheats, benchmark, debug, cloud. All serde-friendly so configs
//! round-trip through TOML/JSON.

use crate::presence::DiscordSettings;
use serde::{Deserialize, Serialize};

/// Root infra config — every subsystem gets a section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct InfraConfig {
    pub save: SaveSection,
    pub mod_system: ModSection,
    pub cheat_engine: CheatSection,
    pub benchmark: BenchmarkSection,
    pub debug: DebugSection,
    pub cloud: CloudSection,
    /// Discord Rich Presence (disabled entirely when application_id is empty).
    pub discord: DiscordSettings,
}

/// Save-system settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveSection {
    /// Directory for save files (default: platform data dir / xholy/saves).
    pub dir: Option<String>,
    /// Enable periodic auto-save.
    pub auto_save: bool,
    /// Auto-save interval in seconds (0 = off).
    pub auto_save_interval_secs: u64,
    /// Compress save files.
    pub compress: bool,
}
impl Default for SaveSection {
    fn default() -> Self {
        Self {
            dir: None,
            auto_save: true,
            auto_save_interval_secs: 300,
            compress: true,
        }
    }
}

/// Mod-system settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ModSection {
    /// Directories scanned for mods.
    pub dirs: Vec<String>,
    /// Enable hot-reload of mod manifests.
    pub hot_reload: bool,
    /// Load order override (mod ids, earlier = higher priority).
    pub load_order: Vec<String>,
}

/// Cheat-engine settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheatSection {
    /// Enable the cheat manager.
    pub enabled: bool,
    /// Show the cheat overlay hotkey hint.
    pub show_overlay: bool,
}
impl Default for CheatSection {
    fn default() -> Self {
        Self {
            enabled: false,
            show_overlay: true,
        }
    }
}

/// Benchmark settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchmarkSection {
    /// Default benchmark duration in ms.
    pub default_duration_ms: u64,
    /// Write reports to this directory (None = temp).
    pub report_dir: Option<String>,
}
impl Default for BenchmarkSection {
    fn default() -> Self {
        Self {
            default_duration_ms: 5000,
            report_dir: None,
        }
    }
}

/// Debug-menu settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugSection {
    /// Debug overlay hotkey (F1..F12).
    pub hotkey: String,
    /// Show FPS counter in overlay.
    pub show_fps: bool,
    /// Show memory stats.
    pub show_memory: bool,
}
impl Default for DebugSection {
    fn default() -> Self {
        Self {
            hotkey: "F1".into(),
            show_fps: true,
            show_memory: true,
        }
    }
}

/// Cloud settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudSection {
    /// Enable cloud sync.
    pub enabled: bool,
    /// Cloud endpoint base URL (http only for now).
    pub endpoint: String,
    /// Auth token (loaded from file/env, never committed).
    pub token_env: String,
}
impl Default for CloudSection {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            token_env: "XHOLY_CLOUD_TOKEN".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let c = InfraConfig::default();
        let s = toml::to_string(&c).unwrap();
        let back: InfraConfig = toml::from_str(&s).unwrap();
        assert!(back.save.auto_save);
        assert!(!back.cheat_engine.enabled);
        assert_eq!(back.debug.hotkey, "F1");
    }

    #[test]
    fn defaults_are_sane() {
        let c = InfraConfig::default();
        assert_eq!(c.benchmark.default_duration_ms, 5000);
        assert!(!c.cloud.enabled);
    }
}
