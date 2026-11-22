//! Abstract GUI frame renderer — Phase 6.
//!
//! The windowed GUI composites its UI into an RGBA8 framebuffer in
//! software ([`crate::gui::windowed::ui`]); a [`GuiFrameRenderer`]
//! implementation decides how those pixels reach the screen:
//!
//! - [`crate::gui::windowed::renderer::WgpuRenderer`] — Vulkan/OpenGL
//!   via wgpu (surface or offscreen).
//! - [`crate::gui::gl_backend::GlBackend`] — wgpu GL backend wrapper
//!   with an explicit OpenGL-fallback warning.
//! - [`crate::gui::software_renderer::SoftwareRenderer`] — pure CPU
//!   framebuffer (headless/tests, last-resort fallback).

use crate::gui::gpu_api_detector::GpuApi;

/// A renderer that accepts a composited RGBA8 frame.
pub trait GuiFrameRenderer {
    /// Resize the render target.
    fn resize(&mut self, width: u32, height: u32);
    /// Upload `rgba` (width×height×4 bytes) and draw one frame.
    fn render_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), String>;
    /// Human-readable backend name for the info panel.
    fn backend_name(&self) -> &'static str;
    /// The GPU API this backend uses.
    fn gpu_api(&self) -> GpuApi;
    /// True when running on a fallback path (software/GL-emulation).
    fn is_fallback(&self) -> bool {
        false
    }
    /// Warning text to show in the UI overlay, if any.
    fn warning(&self) -> Option<&'static str> {
        self.is_fallback().then_some("fallback renderer active")
    }
}

impl GuiFrameRenderer for crate::gui::windowed::renderer::WgpuRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        crate::gui::windowed::renderer::WgpuRenderer::resize(self, width, height);
    }

    fn render_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        crate::gui::windowed::renderer::WgpuRenderer::render_rgba(self, rgba, width, height)
    }

    fn backend_name(&self) -> &'static str {
        "wgpu"
    }

    fn gpu_api(&self) -> GpuApi {
        GpuApi::Vulkan
    }
}
