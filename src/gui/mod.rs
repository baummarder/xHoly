//! GUI subsystem for xHoly — Phase 4.
//!
//! Two backends:
//! - [`ratatui_backend`]: terminal UI (always available, headless-safe)
//! - [`wgpu_backend`]: GPU-accelerated preview via the Phase-2 renderer
//!
//! Theme: trans-flag accents (`#5BCEFA` / `#F5A9B8` / white) on a deep
//! blue-black surface; toolbar buttons carry other LGBTQ+ flag colours
//! (see [`crate::theme`]).

pub mod config;
pub mod frame_pacing;
#[cfg(not(target_arch = "wasm32"))]
pub mod gl_backend;
#[cfg(not(target_arch = "wasm32"))]
pub mod gpu_api_detector;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_gpu_backend;
#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
pub mod native_wm;
pub mod postfx;
pub mod preview;
pub mod ratatui_backend;
#[cfg(not(target_arch = "wasm32"))]
pub mod renderer;
pub mod scarletx;
pub mod settings;
pub mod shader_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod software_renderer;
pub mod terminal;
pub mod textures;
#[cfg(not(target_arch = "wasm32"))]
pub mod wgpu_backend;
pub mod widgets;
pub mod window;
#[cfg(not(target_arch = "wasm32"))]
pub mod windowed;

pub use config::{GpuApi, GpuApiPreference, GuiBackend, GuiConfig, GuiTheme};
pub use frame_pacing::{FramePacer, FramePacingMode};
pub use postfx::{AaMethod, PostFxConfig, ToneMapping};

use chrono::{NaiveDate, NaiveDateTime};
use std::path::PathBuf;

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
    /// Console output captured from the last launched game
    /// (loader/kernel/game stdout lines, newest last, capped).
    pub console_log: Vec<String>,
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
            console_log: Vec::new(),
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
