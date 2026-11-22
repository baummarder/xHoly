//! Window/surface manager for the GUI.
//!
//! Tracks logical size, title, HiDPI scale, and fullscreen state. The
//! ratatui backend renders into the terminal "window"; a future wgpu
//! windowed shell will consume the same state.

/// Managed window state.
#[derive(Debug, Clone)]
pub struct WindowManager {
    /// Window title.
    pub title: String,
    /// Logical size in pixels.
    pub size: (u32, u32),
    /// HiDPI scale factor (1.0 = 100%).
    pub scale_factor: f64,
    /// Whether fullscreen is active.
    pub fullscreen: bool,
    /// Whether vsync is enabled.
    pub vsync: bool,
}

impl WindowManager {
    /// Create a window manager with the given title and size.
    pub fn new(title: impl Into<String>, size: (u32, u32)) -> Self {
        Self {
            title: title.into(),
            size,
            scale_factor: 1.0,
            fullscreen: false,
            vsync: true,
        }
    }

    /// Update the scale factor from the OS.
    pub fn set_scale_factor(&mut self, factor: f64) {
        self.scale_factor = factor.max(1.0);
    }

    /// Resize the window.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
    }

    /// Toggle fullscreen.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
    }

    /// Terminal cell dimensions approximating the pixel size at the
    /// current scale factor (rough 8×16 cell metrics for ratatui).
    pub fn cell_size(&self) -> (u16, u16) {
        let scale = self.scale_factor.max(1.0) as u32;
        (
            ((self.size.0 / (8 * scale)) as u16).max(1),
            ((self.size.1 / (16 * scale)) as u16).max(1),
        )
    }
}
