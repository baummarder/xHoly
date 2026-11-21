//! GPU context: owns the active rendering backend and framebuffer.
//!
//! Backend selection order:
//! 1. Hardware: WGPU/Vulkan/Metal/DX12 via the [`wgpu`] crate.
//! 2. Software: a built-in rasterizer ([`crate::gpu::translation::SoftwareRasterizer`]).

use crate::config::Config;
use crate::core::memory::Memory;

/// Width of the emulated display (Series X native target is 4K; default 1440p).
pub const DEFAULT_WIDTH: u32 = 2560;
pub const DEFAULT_HEIGHT: u32 = 1440;

/// Active GPU backend.
pub enum Backend {
    /// Hardware-accelerated backend via wgpu.
    Hardware,
    /// Software rasterizer fallback.
    Software,
}

/// Rendering context shared across the emulation loop.
pub struct GpuContext {
    backend: Backend,
    width: u32,
    height: u32,
    /// RGBA8 framebuffer in host memory (always maintained for readback).
    framebuffer: Vec<u8>,
    frame_index: u64,
}

impl GpuContext {
    /// Create a GPU context, honoring `config.video.backend` preference.
    pub fn new(config: &Config) -> Self {
        let backend = match config.video.backend.as_str() {
            "software" => Backend::Software,
            _ => Backend::Hardware, // attempt hardware; falls back at present time
        };
        let (w, h) = (config.video.width, config.video.height);
        Self {
            backend,
            width: w,
            height: h,
            framebuffer: vec![0u8; (w * h * 4) as usize],
            frame_index: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Copy the current framebuffer out as RGBA8.
    pub fn readback_rgba(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Submit a frame of guest rendering commands.
    ///
    /// The scaffold paints a deterministic test pattern so the pipeline
    /// (CPU → memory → GPU → framebuffer) is exercisable end-to-end.
    pub fn submit_frame(&mut self, _mem: &Memory) {
        match self.backend {
            Backend::Hardware => self.present_hardware(),
            Backend::Software => self.present_software(),
        }
        self.frame_index += 1;
    }

    fn present_hardware(&mut self) {
        // In a full implementation this encodes wgpu render passes from the
        // translated command buffer. The scaffold fills the host framebuffer
        // directly; wgpu swapchain integration is future work.
        self.fill_pattern();
    }

    fn present_software(&mut self) {
        crate::gpu::translation::SoftwareRasterizer::new(self.width, self.height)
            .rasterize(&mut self.framebuffer);
    }

    /// Deterministic gradient pattern for smoke-testing the pipeline.
    fn fill_pattern(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let t = (self.frame_index % 256) as u8;
                self.framebuffer[i] = (x as u32 % 256) as u8;
                self.framebuffer[i + 1] = (y as u32 % 256) as u8;
                self.framebuffer[i + 2] = t;
                self.framebuffer[i + 3] = 255;
            }
        }
    }
}
