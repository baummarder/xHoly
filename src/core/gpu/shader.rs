//! Shader translation stub: DX12 bytecode → SPIR-V → WGSL → GLSL.
//!
//! Phase-2 scope: recognize container headers, emit WGSL for the built-in
//! text-bitmap shader, and provide the software-rasterizer fallback used
//! when no GPU adapter exists. Real DXBC/DXIL decoding is future work.

use crate::core::gpu::SHADER_WGSL;

/// Shader stage labels for translated programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage {
    /// Vertex stage.
    Vertex,
    /// Pixel/fragment stage.
    Pixel,
    /// Compute stage.
    Compute,
}

/// Output shading language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderTarget {
    /// WGSL (wgpu/WebGPU).
    Wgsl,
    /// GLSL (OpenGL).
    Glsl,
    /// SPIR-V binary (Vulkan).
    SpirV,
}

/// A translated shader program.
#[derive(Debug, Clone)]
pub struct ShaderProgram {
    /// Debug name.
    pub name: String,
    /// Source stage it was translated from.
    pub stage: ShaderStage,
    /// Target language of `source`.
    pub target: ShaderTarget,
    /// Translated source text (WGSL/GLSL) — empty for SpirV binary.
    pub source: String,
}

/// Magic bytes for the xHoly stub DXBC container (`"xHO1"`).
pub const XHOLY_SHADER_MAGIC: u32 = 0x314f4858;

/// Errors from the translation stub.
#[derive(Debug, thiserror::Error)]
pub enum ShaderError {
    /// Input too short to contain a header.
    #[error("shader bytecode too short")]
    TooShort,
    /// Unrecognized container magic.
    #[error("unrecognized shader magic: 0x{0:08x}")]
    BadMagic(u32),
    /// Requested target unsupported by the stub.
    #[error("target {:?} not supported by stub", .0)]
    UnsupportedTarget(ShaderTarget),
}

/// Parse a (stub) DX12 shader container and translate to `target`.
///
/// The stub accepts either:
/// - the xHoly magic `0x314f4858` ("xHO1"), or
/// - any blob whose first four bytes are `"DXBC"` (real DXBC container),
///   which we pass through to the WGSL emitter unchanged.
///
/// Real DXBC opcode decoding is Phase 3+ work.
pub fn translate_dx12_to_wgsl(bytecode: &[u8]) -> Result<ShaderProgram, ShaderError> {
    translate(bytecode, ShaderStage::Pixel, ShaderTarget::Wgsl)
}

/// Translate `bytecode` to `target`, detecting the container format.
pub fn translate(
    bytecode: &[u8],
    stage: ShaderStage,
    target: ShaderTarget,
) -> Result<ShaderProgram, ShaderError> {
    if bytecode.len() < 4 {
        return Err(ShaderError::TooShort);
    }
    let magic = u32::from_le_bytes([bytecode[0], bytecode[1], bytecode[2], bytecode[3]]);
    match magic {
        XHOLY_SHADER_MAGIC => {}
        0x43425844 => {} // "DXBC" little-endian
        other => return Err(ShaderError::BadMagic(other)),
    }

    let source = match target {
        ShaderTarget::Wgsl => SHADER_WGSL.to_string(),
        ShaderTarget::Glsl => wgsl_to_glsl_stub(SHADER_WGSL),
        ShaderTarget::SpirV => {
            // No SPIR-V assembler in the stub; emit WGSL and let the
            // caller note the mismatch via `target`.
            return Err(ShaderError::UnsupportedTarget(target));
        }
    };

    Ok(ShaderProgram {
        name: format!("dx12_{:?}", stage).to_lowercase(),
        stage,
        target,
        source,
    })
}

/// Naive WGSL→GLSL transliteration for the stub shader (texture sampling
/// only — enough for smoke tests, not a real compiler).
fn wgsl_to_glsl_stub(wgsl: &str) -> String {
    let mut out = String::with_capacity(wgsl.len() + 256);
    out.push_str("#version 450\n");
    out.push_str("layout(set=0, binding=0) uniform sampler2D tex;\n");
    out.push_str("layout(location=0) out vec4 fs_color;\n");
    out.push_str("void main() { fs_color = vec4(1.0, 0.84, 0.0, 1.0); }\n");
    // Keep a marker so tests can verify the pass ran.
    out.push_str(&format!("// source-wgsl-len={}\n", wgsl.len()));
    out
}

/// Software rasterizer fallback — renders the bit-texture to RGBA.
///
/// Kept here for API compatibility with the spec; the canonical
/// implementation lives in [`crate::core::gpu::software`].
pub mod fallback {
    use crate::core::gpu::{bytes_to_bitmap, GpuFrame};

    /// Render `frame.text` into RGBA8 (trans blue on dark), unscaled.
    pub fn render_software(frame: &GpuFrame) -> Vec<u8> {
        let (w, _h, bits) = bytes_to_bitmap(&frame.text);
        let mut out = Vec::with_capacity((w * 8 * 4) as usize);
        for &b in &bits {
            if b > 0 {
                out.extend_from_slice(&[0x5B, 0xCE, 0xFA, 255]);
            } else {
                out.extend_from_slice(&[0x12, 0x16, 0x22, 255]);
            }
        }
        out
    }
}
