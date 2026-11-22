//! winit window + wgpu surface management for the windowed GUI.

use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

/// Create the winit window for the GUI.
pub fn create_window(
    event_loop: &ActiveEventLoop,
    title: &str,
    size: (u32, u32),
    fullscreen: bool,
) -> Result<Arc<Window>, String> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_inner_size(LogicalSize::new(size.0 as f64, size.1 as f64))
        .with_resizable(true)
        .with_min_inner_size(LogicalSize::new(640.0, 400.0));
    let window = event_loop
        .create_window(attrs)
        .map_err(|e| format!("window creation failed: {e}"))?;
    let window = Arc::new(window);
    if fullscreen {
        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
    }
    Ok(window)
}

/// Create the wgpu surface for `window`.
pub fn create_surface(
    instance: &wgpu::Instance,
    window: &Arc<Window>,
) -> Result<wgpu::Surface<'static>, String> {
    instance
        .create_surface(window.clone())
        .map_err(|e| format!("surface creation failed: {e}"))
}

/// Handle a resize event: returns the new physical size, or None if
/// minimized (skip rendering that frame).
pub fn resize_handler(window: &Window) -> Option<(u32, u32)> {
    let size = window.inner_size();
    if size.width == 0 || size.height == 0 {
        None
    } else {
        Some((size.width, size.height))
    }
}

/// Handle close: returns true when the window should shut down.
/// (Kept as a named hook so shutdown logic lives in one place.)
pub fn close_handler() -> bool {
    true
}
