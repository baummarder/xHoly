//! Translation passes for GPU workloads.
//!
//! - [`translate_shader_bytecode`]: DXIL/DXBC → SPIR-V/GLSL/WGSL.
//! - [`SoftwareRasterizer`]: fallback renderer when no GPU API is available.

use crate::gpu::shader::{Shader, ShaderTarget, TranslationResult};

/// Translate raw Xbox shader bytecode to the requested target.
pub fn translate_shader_bytecode(
    bytecode: &[u8],
    stage: crate::gpu::shader::ShaderStage,
    target: ShaderTarget,
) -> TranslationResult {
    let mut shader = Shader::analyze(bytecode);
    shader.stage = stage;
    shader.translate(target)
}

/// A minimal software rasterizer used when hardware acceleration is
/// unavailable (e.g. headless CI, Raspberry Pi without GPU drivers).
///
/// Renders a deterministic gradient into an RGBA8 framebuffer.
pub struct SoftwareRasterizer {
    width: u32,
    height: u32,
}

impl SoftwareRasterizer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Rasterize a solid-color test triangle into `framebuffer`.
    pub fn rasterize(&self, framebuffer: &mut [u8]) {
        let (w, h) = (self.width as usize, self.height as usize);
        // Draw a centered triangle with per-pixel barycentric shading.
        let (x0, y0) = (w as f32 * 0.5, h as f32 * 0.15);
        let (x1, y1) = (w as f32 * 0.85, h as f32 * 0.85);
        let (x2, y2) = (w as f32 * 0.15, h as f32 * 0.85);
        let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        if area.abs() < f32::EPSILON {
            return;
        }
        for y in 0..h {
            for x in 0..w {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let w0 = ((x1 - px) * (y2 - py) - (x2 - px) * (y1 - py)) / area;
                let w1 = ((x2 - px) * (y0 - py) - (x0 - px) * (y2 - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let i = (y * w + x) * 4;
                    framebuffer[i] = (w0 * 255.0) as u8;
                    framebuffer[i + 1] = (w1 * 255.0) as u8;
                    framebuffer[i + 2] = (w2 * 255.0) as u8;
                    framebuffer[i + 3] = 255;
                }
            }
        }
    }
}
