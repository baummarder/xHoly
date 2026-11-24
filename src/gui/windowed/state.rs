//! Windowed GUI state — Phase 5.

use crate::core::execution::GuestFrame;
use crate::gui::scarletx::BootProgress;
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

/// Top-level navigation view of the redesigned xHoly UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuiView {
    /// Branding, recent games and quick access.
    #[default]
    Home,
    /// Searchable game library.
    Library,
    /// Live emulation status and logs.
    Emulation,
    /// Grouped settings.
    Settings,
    /// Structured diagnostics.
    Logs,
    /// Version and credits.
    About,
}

impl GuiView {
    /// All views in navigation order.
    pub const ALL: [Self; 6] = [
        Self::Home,
        Self::Library,
        Self::Emulation,
        Self::Settings,
        Self::Logs,
        Self::About,
    ];

    /// Navigation label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Library => "LIBRARY",
            Self::Emulation => "EMULATION",
            Self::Settings => "SETTINGS",
            Self::Logs => "LOGS",
            Self::About => "ABOUT",
        }
    }
}

/// Log level filter for the diagnostics view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFilter {
    /// Everything.
    #[default]
    All,
    /// Errors only.
    Error,
    /// Warnings and errors.
    Warning,
    /// Info and above.
    Info,
    /// Debug and above (everything).
    Debug,
}

impl LogFilter {
    /// Parse a CLI/config-style name.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "all" | "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warning" | "warn" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
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
    /// Whether the software (CPU) renderer is active.
    pub software_mode: bool,
    /// Warning message shown when on a fallback renderer.
    pub software_message: String,
    /// Settings panel visibility.
    pub settings_open: bool,
    /// Emulation overlay (Ctrl+Alt+T) with return-to-main-menu.
    pub emu_menu_open: bool,
    /// Configured FPS limit (mirrored for the settings panel).
    pub fps_limit: Option<u32>,
    /// Configured vsync flag.
    pub vsync: bool,
    /// Configured fullscreen flag.
    pub fullscreen: bool,
    /// Mouse position in framebuffer pixels.
    pub mouse: (u32, u32),
    /// Left mouse button held.
    pub mouse_down: bool,
    /// Last guest framebuffer frame blitted into the preview (Phase 18).
    /// Held across frames when the guest FB aperture stays empty.
    pub guest_frame: Option<GuestFrame>,
    /// Console output captured from game launches (loader/kernel/game
    /// stdout lines, newest last, capped at 200).
    pub console_log: Vec<String>,
    /// Retail-boot progress shown by the ScarletX shell (Phase 24).
    pub boot_progress: Option<BootProgress>,
    /// Active top-level navigation view.
    pub view: GuiView,
    /// Library search text (display only; filtering happens in the UI).
    pub search: String,
    /// Diagnostics level filter.
    pub log_filter: LogFilter,
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
            resolution: (1920, 1080),
            preview_size: (320, 180),
            theme: GuiTheme::Dark,
            gpu_label: String::new(),
            software_mode: false,
            software_message: String::new(),
            settings_open: false,
            emu_menu_open: false,
            fps_limit: None,
            vsync: true,
            fullscreen: false,
            mouse: (0, 0),
            mouse_down: false,
            guest_frame: None,
            console_log: Vec::new(),
            boot_progress: None,
            view: GuiView::Home,
            search: String::new(),
            log_filter: LogFilter::All,
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
