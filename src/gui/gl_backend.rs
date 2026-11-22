//! OpenGL fallback backend — Phase 6.
//!
//! xHoly drives wgpu with the `GL` backend bit rather than pulling in a
//! separate `gl`/`glutin` stack: wgpu's GL backend uses the system's
//! OpenGL (or ANGLE on platforms that provide it), which is the same
//! code path real GL fallbacks take, minus a second windowing/context
//! implementation to maintain.
//!
//! This wrapper exists to (a) make the fallback explicit in the UI
//! ("Running on OpenGL fallback") and (b) give `--gpu-api opengl` a
//! named backend type.

use super::renderer::GuiFrameRenderer;
use super::windowed::renderer::WgpuRenderer;
use crate::gui::gpu_api_detector::GpuApi;

/// wgpu-over-OpenGL renderer with fallback warning surfaced.
pub struct GlBackend {
    inner: WgpuRenderer,
}

impl GlBackend {
    /// Wrap an already-initialized wgpu renderer (created with
    /// `wgpu::Backends::GL`).
    pub fn new(inner: WgpuRenderer) -> Self {
        Self { inner }
    }

    /// Access the inner wgpu renderer (e.g. for offscreen readback).
    pub fn inner(&self) -> &WgpuRenderer {
        &self.inner
    }

    /// Mutable access.
    pub fn inner_mut(&mut self) -> &mut WgpuRenderer {
        &mut self.inner
    }
}

impl GuiFrameRenderer for GlBackend {
    fn resize(&mut self, width: u32, height: u32) {
        self.inner.resize(width, height);
    }

    fn render_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        self.inner.render_rgba(rgba, width, height)
    }

    fn backend_name(&self) -> &'static str {
        "wgpu (OpenGL backend)"
    }

    fn gpu_api(&self) -> GpuApi {
        GpuApi::OpenGL
    }

    fn set_postfx(&mut self, config: crate::gui::postfx::PostFxConfig) {
        self.inner.set_postfx(config);
    }

    fn is_fallback(&self) -> bool {
        // GL via wgpu is a first-class backend, but we still flag it so
        // the UI can show the "OpenGL fallback" notice per spec.
        true
    }

    fn warning(&self) -> Option<&'static str> {
        Some("Running on OpenGL fallback")
    }
}
