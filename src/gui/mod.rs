//! GUI subsystem for xHoly — Phase 4.
//!
//! Two backends:
//! - [`ratatui_backend`]: terminal UI (always available, headless-safe)
//! - [`wgpu_backend`]: GPU-accelerated preview via the Phase-2 renderer
//!
//! Theme: gold `#FFD700` accents on dark `#1a1a1a`, matching the logo.

pub mod preview;
pub mod ratatui_backend;
pub mod settings;
pub mod terminal;
#[cfg(not(target_arch = "wasm32"))]
pub mod wgpu_backend;
pub mod window;
#[cfg(not(target_arch = "wasm32"))]
pub mod windowed;

use chrono::{NaiveDate, NaiveDateTime};
use std::path::PathBuf;

/// GUI backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiBackend {
    /// Terminal UI via ratatui/crossterm.
    Ratatui,
    /// wgpu-rendered preview (falls back to ratatui when no adapter).
    Wgpu,
}

/// Color theme for the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiTheme {
    /// Dark background, gold accents (default — matches logo).
    Dark,
    /// Gold background, dark text.
    Gold,
    /// Light background.
    Light,
}

impl GuiTheme {
    /// Background RGB.
    pub fn bg(&self) -> (u8, u8, u8) {
        match self {
            Self::Dark => (0x1A, 0x1A, 0x1A),
            Self::Gold => (0xFF, 0xD7, 0x00),
            Self::Light => (0xF5, 0xF5, 0xF5),
        }
    }

    /// Foreground/accent RGB.
    pub fn fg(&self) -> (u8, u8, u8) {
        match self {
            Self::Dark => (0xFF, 0xD7, 0x00),
            Self::Gold => (0x1A, 0x1A, 0x1A),
            Self::Light => (0x1A, 0x1A, 0x1A),
        }
    }
}

/// GUI configuration.
#[derive(Debug, Clone)]
pub struct GuiConfig {
    /// Active backend.
    pub backend: GuiBackend,
    /// Window title.
    pub window_title: String,
    /// Window size (width, height) in pixels.
    pub window_size: (u32, u32),
    /// HiDPI scaling support.
    pub high_dpi: bool,
    /// FPS limit (None = unlimited).
    pub fps_limit: Option<u32>,
    /// Force software rendering.
    pub software_rendering: bool,
    /// Show FPS overlay.
    pub show_fps: bool,
    /// Show CPU usage overlay.
    pub show_cpu_usage: bool,
    /// Show GPU usage overlay.
    pub show_gpu_usage: bool,
    /// Start in fullscreen (windowed backend only).
    pub fullscreen: bool,
    /// Enable vsync (windowed backend only).
    pub vsync: bool,
    /// Headless mode: render loop without a window.
    pub headless: bool,
    /// Color theme.
    pub theme: GuiTheme,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            backend: GuiBackend::Ratatui,
            window_title: "xHoly — Xbox Emulator".into(),
            window_size: (1920, 1080),
            high_dpi: true,
            fps_limit: Some(60),
            software_rendering: false,
            show_fps: true,
            show_cpu_usage: true,
            show_gpu_usage: true,
            fullscreen: false,
            vsync: true,
            headless: false,
            theme: GuiTheme::Dark,
        }
    }
}

/// A game entry in the library view.
#[derive(Debug, Clone)]
pub struct GameListItem {
    /// Display title.
    pub title: String,
    /// Path to the game payload / disc image.
    pub path: PathBuf,
    /// Optional cover art path.
    pub cover_art: Option<PathBuf>,
    /// Region string (NTSC-U / PAL / JPN / OTHER).
    pub region: String,
    /// Number of discs detected.
    pub discs: u8,
    /// Release date if known.
    pub release_date: Option<NaiveDate>,
    /// Whether this is a multi-disc title.
    pub is_multi_disc: bool,
    /// Last time the game was played.
    pub last_played: Option<NaiveDateTime>,
}

impl GameListItem {
    /// Build a list item from an optical [`ScannedGame`](crate::optical::game_scanner::ScannedGame).
    ///
    /// `discs` is the size of the multi-disc group this game belongs to.
    pub fn from_scanned(g: &crate::optical::game_scanner::ScannedGame, discs: u8) -> Self {
        Self {
            title: g
                .metadata
                .title_name
                .clone()
                .unwrap_or_else(|| "<untitled>".into()),
            path: PathBuf::from(&g.payload_path),
            cover_art: None,
            region: g.metadata.region.as_str().to_string(),
            discs,
            release_date: None,
            is_multi_disc: discs > 1,
            last_played: None,
        }
    }
}

/// Mutable GUI state shared across views.
#[derive(Debug)]
pub struct GuiState {
    /// All games in the library.
    pub games: Vec<GameListItem>,
    /// Index of the selected game.
    pub selected_game: usize,
    /// Whether the emulator is currently running a game.
    pub running: bool,
    /// Pause state.
    pub paused: bool,
    /// Current FPS (smoothed).
    pub fps: f32,
    /// CPU usage 0.0–1.0.
    pub cpu_usage: f32,
    /// GPU usage 0.0–1.0.
    pub gpu_usage: f32,
    /// Number of save slots.
    pub save_slots: u8,
    /// Cheat engine toggle.
    pub cheat_engine_enabled: bool,
    /// Active tab for multi-game view.
    pub active_tab: usize,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            games: Vec::new(),
            selected_game: 0,
            running: false,
            paused: false,
            fps: 0.0,
            cpu_usage: 0.0,
            gpu_usage: 0.0,
            save_slots: 10,
            cheat_engine_enabled: false,
            active_tab: 0,
        }
    }
}

impl GuiState {
    /// Currently selected game, if any.
    pub fn selected(&self) -> Option<&GameListItem> {
        self.games.get(self.selected_game)
    }

    /// Move selection down, wrapping at the end.
    pub fn select_next(&mut self) {
        if !self.games.is_empty() {
            self.selected_game = (self.selected_game + 1) % self.games.len();
        }
    }

    /// Move selection up, wrapping at the start.
    pub fn select_prev(&mut self) {
        if !self.games.is_empty() {
            self.selected_game = if self.selected_game == 0 {
                self.games.len() - 1
            } else {
                self.selected_game - 1
            };
        }
    }

    /// Switch to the next tab (multi-game support).
    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.wrapping_add(1);
    }

    /// Switch to the previous tab.
    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.wrapping_sub(1);
    }
}

/// Entry point: run the GUI with the given config.
///
/// Dispatches to the ratatui backend (the wgpu backend is used for
/// in-GUI previews; a full wgpu windowed shell is future work).
pub fn run(config: GuiConfig) -> std::io::Result<()> {
    ratatui_backend::run(config)
}
