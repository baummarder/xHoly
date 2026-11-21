//! GPU rendering subsystem — Phase 2 (wgpu backend + software fallback).
//!
//! Architecture:
//! ```text
//! CPU (arm64.rs) ──output bytes──► bit-texture ──► WgpuBackend ──► RGBA frame
//!                                       │
//!                                       └──► SoftwareRenderer (fallback)
//! ```
//!
//! The CPU's console output is expanded into an R8 bitmap texture (each
//! byte → 8 horizontal pixels, each bit → one row) which the WGSL shader
//! displays as gold-on-dark — a visual echo of "Hello, xHoly!".

pub mod shader;
#[cfg(not(target_arch = "wasm32"))]
pub mod wgpu_backend;

use std::borrow::Cow;

/// Texture geometry for the bit-expanded text display.
pub const TEX_HEIGHT: u32 = 8;
/// Bytes per row of console output that fit before wrapping (none — we
/// size the texture exactly to the output length × 8 px).
pub const MAX_OUTPUT_BYTES: usize = 256;

/// Expand a byte string into an R8 bitmap: byte `i` occupies columns
/// `[i*8, i*8+8)`; bit `b` of the byte lights the whole column when set.
/// Deterministic and trivially testable (a bit-column display, not text).
pub fn bytes_to_bitmap(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let width = (bytes.len() as u32) * 8;
    let height = TEX_HEIGHT;
    let mut tex = vec![0u8; (width * height) as usize];
    for (i, &byte) in bytes.iter().enumerate() {
        for bit in 0..8u32 {
            let x = i as u32 * 8 + bit;
            if (byte >> bit) & 1 == 1 {
                for y in 0..height {
                    tex[(y * width + x) as usize] = 255;
                }
            }
        }
    }
    (width, height, tex)
}

/// A single GPU frame request.
#[derive(Debug, Clone)]
pub struct GpuFrame {
    /// RGBA clear color.
    pub clear_color: [f32; 4],
    /// Optional text bytes to display (expanded via [`bytes_to_bitmap`]).
    pub text: Vec<u8>,
}

impl Default for GpuFrame {
    fn default() -> Self {
        Self {
            clear_color: [0.10, 0.10, 0.10, 1.0],
            text: Vec::new(),
        }
    }
}

/// The GPU backend trait — mirrors the spec's `GpuBackend`.
pub trait GpuBackend {
    /// Initialize the backend (device + pipeline). Headless-friendly.
    fn init(width: u32, height: u32) -> Result<Self, String>
    where
        Self: Sized;
    /// Render `frame` and return the RGBA8 pixels (readback).
    fn render(&mut self, frame: &GpuFrame) -> Result<Vec<u8>, String>;
    /// Release GPU resources.
    fn dispose(&mut self);
}

/// WGSL shader source, embedded at compile time.
pub const SHADER_WGSL: &str = include_str!("../../../shaders/render.wgsl");

/// Fallback used when no GPU adapter is available: expands the text bits
/// into an RGBA buffer on the CPU. Deterministic — safe for CI.
pub mod software {
    use super::{bytes_to_bitmap, GpuFrame};

    /// Render `frame.text` into an RGBA8 buffer (gold-on-dark).
    pub fn render_frame(frame: &GpuFrame) -> Vec<u8> {
        let (w, h, bits) = bytes_to_bitmap(&frame.text);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for &b in &bits {
            if b > 0 {
                out.extend_from_slice(&[255, 214, 0, 255]); // gold #FFD700
            } else {
                out.extend_from_slice(&[26, 26, 26, 255]); // #1a1a1a
            }
        }
        let _ = h;
        out
    }

    /// Render to an arbitrary target size by nearest-neighbor scaling.
    pub fn render_scaled(frame: &GpuFrame, out_w: u32, out_h: u32) -> Vec<u8> {
        let src = render_frame(frame);
        let (sw, sh, _) = bytes_to_bitmap(&frame.text);
        let (ow, oh) = (out_w as usize, out_h as usize);
        let (sw, sh) = (sw as usize, sh as usize);
        let mut out = vec![0u8; ow * oh * 4];
        for y in 0..oh {
            for x in 0..ow {
                let sx = x * sw / ow.max(1);
                let sy = y * sh / oh.max(1);
                let si = (sy * sw + sx) * 4;
                let di = (y * ow + x) * 4;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        out
    }
}

/// Try to create the hardware backend; falls back to software pixels.
///
/// This is the entry point used by tests and by `arm64::run_hello_world_gpu`.
pub fn render_text(text: &[u8], target_w: u32, target_h: u32) -> Result<Vec<u8>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match wgpu_backend::WgpuBackend::init(target_w, target_h) {
            Ok(mut gpu) => {
                let frame = GpuFrame {
                    text: text.to_vec(),
                    ..Default::default()
                };
                let px = gpu.render(&frame)?;
                gpu.dispose();
                return Ok(px);
            }
            Err(e) => {
                log::debug!("xHoly: no GPU adapter ({e}); using software rasterizer");
            }
        }
    }
    let _ = (target_w, target_h);
    Ok(software::render_scaled(
        &GpuFrame {
            text: text.to_vec(),
            ..Default::default()
        },
        target_w.max(1),
        target_h.max(1),
    ))
}

/// Re-export for shader-translation consumers.
pub use crate::gpu::shader::ShaderTarget;

/// Convenience: WGSL source for the embedded render shader as Cow.
pub fn shader_source() -> Cow<'static, str> {
    Cow::Borrowed(SHADER_WGSL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cpu::arm64;

    /// 'H' = 0x48 → bits 3 and 6 set → columns 3 and 6 lit.
    #[test]
    fn bitmap_expansion() {
        let (w, h, tex) = bytes_to_bitmap(b"H");
        assert_eq!((w, h), (8, 8));
        // Column 3 (bit 3) is lit on every row.
        assert_eq!(tex[3], 255);
        assert_eq!(tex[8 + 3], 255);
        // Column 0 (bit 0 = 0) is dark.
        assert_eq!(tex[0], 0);
    }

    /// Software rasterizer produces gold/dark pixels deterministically.
    #[test]
    fn software_render_pixels() {
        let px = software::render_frame(&GpuFrame {
            text: b"H".to_vec(),
            ..Default::default()
        });
        // First pixel (column 0 = bit0 of 'H' = 0) → dark.
        assert_eq!(&px[0..4], &[26, 26, 26, 255]);
        // Column 3 → gold #FFD700.
        let i = (3 * 4) as usize;
        assert_eq!(&px[i..i + 4], &[255, 214, 0, 255]);
    }

    /// DXBC-magic shader blob translates to WGSL.
    #[test]
    fn shader_translation_stub() {
        let dxbc = b"DXBC\x00\x00\x00\x00payload";
        let prog = shader::translate_dx12_to_wgsl(dxbc).expect("translate");
        assert_eq!(prog.target, shader::ShaderTarget::Wgsl);
        assert!(prog.source.contains("@fragment"));
        // Bad magic rejected.
        assert!(shader::translate_dx12_to_wgsl(b"NOPE").is_err());
    }

    /// End-to-end: CPU Hello World → GPU/software render → gold pixels.
    #[test]
    fn render_helloworld() {
        let cpu = arm64::run_hello_world();
        let px = render_text(&cpu.output, 256, 16).expect("render");
        assert_eq!(px.len(), (256 * 16 * 4) as usize);
        // 'H' bit 3 → some gold pixel must exist in the frame.
        let gold = px
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] == 255 && p[1] == 214 && p[2] == 0);
        assert!(gold, "expected gold pixels in rendered frame");
    }

    /// wgpu device/pipeline creation (skips gracefully without adapter).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_pipeline_creation() {
        match wgpu_backend::WgpuBackend::init(64, 64) {
            Ok(mut gpu) => {
                let px = gpu
                    .render(&GpuFrame {
                        text: b"Hi".to_vec(),
                        ..Default::default()
                    })
                    .expect("render");
                assert_eq!(px.len(), 64 * 64 * 4);
                gpu.dispose();
            }
            Err(e) => {
                eprintln!("skipping GPU test (no adapter): {e}");
            }
        }
    }
}
