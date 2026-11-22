//! Windowed GUI state — Phase 5.

use crate::gui::{GameListItem, GuiTheme};
use std::time::SystemTime;

/// A save-state slot (stub — real state serialization is future work).
#[derive(Debug, Clone)]
pub struct SaveState {
    /// Slot index.
    pub slot: u8,
    /// When the state was written.
    pub written_at: SystemTime,
    /// Optional description.
    pub description: String,
}

/// Mutable state for the windowed GUI.
#[derive(Debug)]
pub struct WindowedGuiState {
    /// All games in the library.
    pub games: Vec<GameListItem>,
    /// Index of the selected game.
    pub selected_game: usize,
    /// Emulator running.
    pub running: bool,
    /// Paused.
    pub paused: bool,
    /// Smoothed FPS.
    pub fps: f32,
    /// CPU usage 0.0–1.0.
    pub cpu_usage: f32,
    /// GPU usage 0.0–1.0.
    pub gpu_usage: f32,
    /// Process memory usage in bytes (stub until a metrics crate lands).
    pub memory_usage: u64,
    /// Save-state slots.
    pub save_slots: Vec<SaveState>,
    /// Cheat engine toggle.
    pub cheat_engine_enabled: bool,
    /// Render resolution.
    pub resolution: (u32, u32),
    /// Preview thumbnail size.
    pub preview_size: (u32, u32),
    /// Theme.
    pub theme: GuiTheme,
    /// Detected GPU API description for the info panel (e.g. "Vulkan · RTX 4070").
    pub gpu_label: String,
    /// Mouse position in framebuffer pixels.
    pub mouse: (u32, u32),
    /// Left mouse button held.
    pub mouse_down: bool,
}

impl Default for WindowedGuiState {
    fn default() -> Self {
        Self {
            games: Vec::new(),
            selected_game: 0,
            running: false,
            paused: false,
            fps: 0.0,
            cpu_usage: 0.0,
            gpu_usage: 0.0,
            memory_usage: 0,
            save_slots: Vec::new(),
            cheat_engine_enabled: false,
            resolution: (1280, 720),
            preview_size: (320, 180),
            theme: GuiTheme::Dark,
            gpu_label: String::new(),
            mouse: (0, 0),
            mouse_down: false,
        }
    }
}

impl WindowedGuiState {
    /// Currently selected game, if any.
    pub fn selected(&self) -> Option<&GameListItem> {
        self.games.get(self.selected_game)
    }

    /// Move selection down with wraparound.
    pub fn select_next(&mut self) {
        if !self.games.is_empty() {
            self.selected_game = (self.selected_game + 1) % self.games.len();
        }
    }

    /// Move selection up with wraparound.
    pub fn select_prev(&mut self) {
        if !self.games.is_empty() {
            self.selected_game = if self.selected_game == 0 {
                self.games.len() - 1
            } else {
                self.selected_game - 1
            };
        }
    }

    /// Toggle pause (also toggles `running` bookkeeping).
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if self.paused {
            self.running = true;
        }
    }

    /// Toggle cheat engine.
    pub fn toggle_cheats(&mut self) {
        self.cheat_engine_enabled = !self.cheat_engine_enabled;
    }
}
