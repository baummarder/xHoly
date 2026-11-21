//! Configuration management.
//!
//! Hybrid schema: this Rust side uses `serde` + TOML; the Scala runtime
//! mirrors the same fields with case classes (see `scala/.../Settings.scala`).
//! Config is persisted to `~/.config/xholy/config.toml` and hot-reloaded
//! via filesystem notifications.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub video: VideoConfig,
    pub audio: AudioConfig,
    pub input: InputConfig,
    pub emulation: EmulationConfig,
    pub profiles: Vec<Profile>,
}


/// Video/output settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// Rendering backend: "hardware" (wgpu) or "software".
    pub backend: String,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Enable variable rate shading when supported.
    pub variable_rate_shading: bool,
    /// Enable ray-traced effects when supported.
    pub ray_tracing: bool,
    /// Dynamic resolution scaling factor (1.0 = native).
    pub resolution_scale: f32,
    /// Fullscreen mode.
    pub fullscreen: bool,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            backend: "hardware".into(),
            width: 2560,
            height: 1440,
            variable_rate_shading: true,
            ray_tracing: true,
            resolution_scale: 1.0,
            fullscreen: false,
        }
    }
}

/// Audio settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Audio backend: "wasapi" (Windows), "coreaudio" (macOS), "alsa"/"pulse" (Linux).
    pub backend: String,
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Output buffer size in samples.
    pub buffer_size: u32,
    /// Master volume 0.0–1.0.
    pub volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            backend: "wasapi".into(),
            sample_rate: 48_000,
            buffer_size: 512,
            volume: 1.0,
        }
    }
}

/// Input device settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    /// Controller API: "xinput", "sdl", "raw_hid".
    pub api: String,
    /// Polling rate in Hz.
    pub polling_rate: u32,
    /// Deadzone for analog sticks (0.0–1.0).
    pub deadzone: f32,
    /// Enable Kinect device emulation.
    pub kinect: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            api: "sdl".into(),
            polling_rate: 1000,
            deadzone: 0.1,
            kinect: false,
        }
    }
}

/// Emulation behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmulationConfig {
    /// Fast-forward multiplier (1.0 = normal speed).
    pub fast_forward: f32,
    /// Enable save states.
    pub save_states: bool,
    /// Enable the built-in cheat engine.
    pub cheats: bool,
    /// Enable cloud asset streaming.
    pub cloud_streaming: bool,
    /// Enable mods directory scanning.
    pub mods: bool,
}

impl Default for EmulationConfig {
    fn default() -> Self {
        Self {
            fast_forward: 1.0,
            save_states: true,
            cheats: true,
            cloud_streaming: false,
            mods: true,
        }
    }
}

/// A named user-defined configuration profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Glob pattern matching game IDs/titles this profile applies to.
    pub game_pattern: String,
    /// Per-field overrides serialized as a TOML table.
    pub overrides: toml::Value,
}

impl Config {
    /// Default config path: `~/.config/xholy/config.toml`.
    pub fn default_path() -> PathBuf {
        dirs_home()
            .join(".config")
            .join("xholy")
            .join("config.toml")
    }

    /// Load config from the default location (or defaults if absent).
    pub fn load_default() -> Self {
        Self::load(&Self::default_path())
    }

    /// Load config from `path`; falls back to defaults on any error.
    pub fn load(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("xHoly: failed to parse {:?}: {}", path, e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Persist config to `path`, creating parent directories as needed.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, text)
    }

    /// Save to the default location.
    pub fn save_default(&self) -> std::io::Result<()> {
        self.save(&Self::default_path())
    }

    /// Watch `path` for changes and invoke `on_change` on modification.
    /// Returns a guard that stops watching when dropped.
    pub fn watch<F>(path: PathBuf, on_change: F) -> notify::RecommendedWatcher
    where
        F: Fn() + Send + 'static,
    {
        use notify::{RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .expect("failed to create config watcher");
        watcher.watch(&path, RecursiveMode::NonRecursive).ok();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                on_change();
                // Debounce: drain pending events.
                while rx.try_recv().is_ok() {}
            }
        });
        watcher
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
