//! GUI settings: theme, overlay toggles, persistence.

use super::{GuiConfig, GuiTheme};
use serde::{Deserialize, Serialize};

/// Persisted GUI preferences (subset of [`GuiConfig`] saved to TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    /// Selected theme name: "dark" | "gold" | "light".
    pub theme: String,
    /// Show FPS overlay.
    pub show_fps: bool,
    /// Show CPU usage overlay.
    pub show_cpu_usage: bool,
    /// FPS limit.
    pub fps_limit: Option<u32>,
    /// Software rendering fallback.
    pub software_rendering: bool,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            show_fps: true,
            show_cpu_usage: true,
            fps_limit: Some(60),
            software_rendering: false,
        }
    }
}

impl GuiSettings {
    /// Apply these settings onto a [`GuiConfig`].
    pub fn apply(&self, config: &mut GuiConfig) {
        config.theme = match self.theme.as_str() {
            "gold" => GuiTheme::Gold,
            "light" => GuiTheme::Light,
            _ => GuiTheme::Dark,
        };
        config.show_fps = self.show_fps;
        config.show_cpu_usage = self.show_cpu_usage;
        config.fps_limit = self.fps_limit;
        config.software_rendering = self.software_rendering;
    }

    /// Extract settings from a [`GuiConfig`].
    pub fn from_config(config: &GuiConfig) -> Self {
        Self {
            theme: match config.theme {
                GuiTheme::Dark => "dark",
                GuiTheme::Gold => "gold",
                GuiTheme::Light => "light",
            }
            .into(),
            show_fps: config.show_fps,
            show_cpu_usage: config.show_cpu_usage,
            fps_limit: config.fps_limit,
            software_rendering: config.software_rendering,
        }
    }

    /// Load GUI settings from `~/.config/xholy/gui.toml` (defaults if absent).
    pub fn load_default() -> Self {
        let path = crate::config::Config::default_path()
            .parent()
            .unwrap()
            .join("gui.toml");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Save settings next to the main config file.
    pub fn save_default(&self) -> std::io::Result<()> {
        let path = crate::config::Config::default_path()
            .parent()
            .unwrap()
            .join("gui.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self).unwrap_or_default())
    }
}
