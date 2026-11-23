//! GUI configuration types — Phase 5/6/Final.
//!
//! Holds [`GuiConfig`], [`GuiTheme`], [`GuiBackend`], [`GpuApi`] and
//! [`GpuApiPreference`]. These types deliberately avoid wgpu/winit so
//! they compile on every target (incl. wasm and tests).

use crate::gui::frame_pacing::FramePacingMode;
use crate::gui::postfx::PostFxConfig;
use serde::{Deserialize, Serialize};

/// Detected/selected GPU API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuApi {
    /// Direct3D 12 (Windows primary; wgpu DX12 backend in this scaffold).
    D3D12,
    /// Vulkan (Linux primary; MoltenVK on macOS via wgpu).
    Vulkan,
    /// Metal (macOS primary; wgpu Metal backend in this scaffold).
    Metal,
    /// OpenGL 4.x / GLES (wgpu GL backend or ANGLE).
    OpenGL,
    /// No hardware API — CPU/software rendering only.
    Software,
}

impl GpuApi {
    /// CLI-friendly name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::D3D12 => "d3d12",
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
            Self::OpenGL => "opengl",
            Self::Software => "software",
        }
    }
}

/// User preference: auto-detect or force an API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuApiPreference {
    /// Probe and pick the best available API (default).
    #[default]
    Auto,
    D3D12,
    Vulkan,
    Metal,
    OpenGL,
    Software,
}

impl GpuApiPreference {
    /// Parse a CLI/config string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "d3d12" | "dx12" | "direct3d12" => Some(Self::D3D12),
            "vulkan" => Some(Self::Vulkan),
            "metal" => Some(Self::Metal),
            "opengl" | "gl" => Some(Self::OpenGL),
            "software" | "sw" | "cpu" => Some(Self::Software),
            _ => None,
        }
    }

    /// Resolve to a concrete API using a detected result.
    /// `detected` is what auto-probing found; forced prefs win.
    pub fn resolve(self, detected: GpuApi) -> GpuApi {
        match self {
            Self::Auto => detected,
            Self::D3D12 => GpuApi::D3D12,
            Self::Vulkan => GpuApi::Vulkan,
            Self::Metal => GpuApi::Metal,
            Self::OpenGL => GpuApi::OpenGL,
            Self::Software => GpuApi::Software,
        }
    }
}

/// GUI backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiBackend {
    /// Terminal UI via ratatui/crossterm.
    Ratatui,
    /// wgpu-rendered preview (falls back to ratatui when no adapter).
    Wgpu,
}

/// Color theme for the GUI.
///
/// Every variant uses the pride palette from [`crate::theme`]; the
/// legacy names (`dark`/`gold`/`light`) stay valid for CLI/config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiTheme {
    /// Deep blue-black surface with trans-flag accents (default).
    Dark,
    /// Blue-black surface with pink-forward trans accents.
    Gold,
    /// Light surface with trans-flag accents and dark text.
    Light,
}

impl GuiTheme {
    /// Parse a CLI string (`dark`|`gold`|`light`|`trans`|`pride`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "dark" | "trans" | "pride" => Some(Self::Dark),
            "gold" | "pink" => Some(Self::Gold),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    /// Background RGB.
    pub fn bg(&self) -> (u8, u8, u8) {
        use crate::theme::{BG, TRANS_WHITE};
        let c = match self {
            Self::Dark | Self::Gold => BG,
            Self::Light => TRANS_WHITE,
        };
        (c[0], c[1], c[2])
    }

    /// Foreground/accent RGB (trans blue for dark themes).
    pub fn fg(&self) -> (u8, u8, u8) {
        use crate::theme::{TRANS_BLUE, TRANS_PINK};
        let c = match self {
            Self::Dark | Self::Light => TRANS_BLUE,
            Self::Gold => TRANS_PINK,
        };
        (c[0], c[1], c[2])
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
    /// Frame pacing policy: `Auto` (VSync→Fifo, else manual), `Fixed`,
    /// or `Uncapped`.
    pub frame_pacing_mode: FramePacingMode,
    /// Headless mode: render loop without a window.
    pub headless: bool,
    /// Color theme.
    pub theme: GuiTheme,
    /// Preferred GPU API (`Auto` = platform order: D3D12 on Windows,
    /// Vulkan on Linux, Metal on macOS, then wgpu/software fallbacks).
    pub preferred_api: GpuApiPreference,
    /// Post-processing configuration (Phase 9): bloom, tone mapping,
    /// anti-aliasing. Ignored by the software renderer.
    pub postfx: PostFxConfig,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            backend: GuiBackend::Ratatui,
            window_title: "xHoly — Xbox Emulator".into(),
            window_size: (1920, 1080),
            high_dpi: true,
            fps_limit: Some(120),
            software_rendering: false,
            show_fps: true,
            show_cpu_usage: true,
            show_gpu_usage: true,
            fullscreen: false,
            vsync: true,
            frame_pacing_mode: FramePacingMode::Auto,
            headless: false,
            theme: GuiTheme::Dark,
            preferred_api: GpuApiPreference::Auto,
            postfx: PostFxConfig::default(),
        }
    }
}
