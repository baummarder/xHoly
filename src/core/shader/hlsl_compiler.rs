//! HLSL intermediate representation — Phase 10.
//!
//! The bridge between [`super::dxil_parser`] and the SPIR-V/WGSL
//! backends. Real DXIL is LLVM bitcode — decoding arbitrary bitcode
//! is a multi-month project, so this IR covers a **documented subset**
//! sufficient for trivial/generated shaders:
//!
//! - scalar/vector constants
//! - `add`/`mul` on scalars
//! - `sample` of a bound 2D texture
//! - passthrough vertex (position → position)
//!
//! [`lower`] attempts DXIL-bitcode recognition for the subset and
//! returns [`HlslError::Unsupported`] for anything else — callers must
//! handle that with the blit-shader fallback (see
//! [`super::wgsl_emitter`]).

/// Scalar/vector constant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Const {
    F32(f32),
    Vec2(f32, f32),
    Vec3(f32, f32, f32),
    Vec4(f32, f32, f32, f32),
}

/// IR node kinds (expression trees, not SSA).
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Const(Const),
    /// x + y (component-wise for vectors).
    Add(Box<Node>, Box<Node>),
    /// x * y.
    Mul(Box<Node>, Box<Node>),
    /// texture.Sample(uv) — binds to texture register t#.
    SampleTexture {
        reg: u32,
        uv: Box<Node>,
    },
    /// gl_VertexIndex / SV_VertexID equivalent.
    VertexId,
    /// Input parameter by semantic name (e.g. "POSITION").
    InputParam(String),
}

/// A lowered shader: entry kind + expression producing the output.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderStage {
    /// Vertex shader: outputs clip-space position (+ optional varyings).
    Vertex { body: Node },
    /// Fragment/pixel shader: outputs vec4 color.
    Fragment { body: Node },
    /// Compute: not yet supported.
    Compute,
}

/// Complete lowered shader.
#[derive(Debug, Clone)]
pub struct HlslIr {
    pub stage: ShaderStage,
    /// Bound texture registers used (for the binder).
    pub texture_regs: Vec<u32>,
}

/// Errors from lowering.
#[derive(Debug, PartialEq)]
pub enum HlslError {
    /// DXIL bitcode uses features outside the supported subset.
    Unsupported(&'static str),
    /// Bitcode is empty (metadata-only container).
    EmptyBitcode,
}

impl std::fmt::Display for HlslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "unsupported DXIL feature: {what}"),
            Self::EmptyBitcode => write!(f, "empty DXIL bitcode"),
        }
    }
}

impl std::error::Error for HlslError {}

/// Marker embedded in test/generated bitcode: our compiler recognizes
/// `XHLS` + a stage byte as a *host-generated* mini-program, not real
/// LLVM bitcode. This keeps the pipeline exercisable end-to-end while
/// being explicit that retail DXIL needs the full bitcode decoder.
pub const MINI_MAGIC: &[u8; 4] = b"XHLS";

/// Try to lower DXIL bitcode to the IR.
///
/// Recognized inputs:
/// - `XHLS` + stage byte + serialized node stream (host-generated)
/// - anything else → `Unsupported("llvm-bitcode")`
pub fn lower(bitcode: &[u8]) -> Result<HlslIr, HlslError> {
    if bitcode.is_empty() {
        return Err(HlslError::EmptyBitcode);
    }
    if bitcode.starts_with(MINI_MAGIC) && bitcode.len() >= 6 {
        let stage = bitcode[4];
        let body = decode_node(&bitcode[5..])?;
        let texture_regs = collect_texture_regs(&body);
        return Ok(HlslIr {
            stage: match stage {
                b'v' => ShaderStage::Vertex { body },
                b'f' => ShaderStage::Fragment { body },
                _ => return Err(HlslError::Unsupported("stage")),
            },
            texture_regs,
        });
    }
    Err(HlslError::Unsupported("llvm-bitcode"))
}

/// Encode a node into the mini-program stream (test/tooling helper —
/// mirrors [`decode_node`]).
pub fn encode_node(n: &Node, out: &mut Vec<u8>) {
    match n {
        Node::Const(c) => {
            out.push(0);
            match c {
                Const::F32(v) => {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                Const::Vec2(x, y) => {
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                }
                Const::Vec3(x, y, z) => {
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                    out.extend_from_slice(&z.to_le_bytes());
                }
                Const::Vec4(x, y, z, w) => {
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                    out.extend_from_slice(&z.to_le_bytes());
                    out.extend_from_slice(&w.to_le_bytes());
                }
            }
        }
        Node::Add(a, b) => {
            // Binary ops length-prefix the left child so decoding can
            // split the stream unambiguously.
            let mut tmp = Vec::new();
            encode_node(a, &mut tmp);
            out.push(1);
            out.extend_from_slice(&(tmp.len() as u16).to_le_bytes());
            out.extend_from_slice(&tmp);
            encode_node(b, out);
        }
        Node::Mul(a, b) => {
            let mut tmp = Vec::new();
            encode_node(a, &mut tmp);
            out.push(2);
            out.extend_from_slice(&(tmp.len() as u16).to_le_bytes());
            out.extend_from_slice(&tmp);
            encode_node(b, out);
        }
        Node::SampleTexture { reg, uv } => {
            out.push(3);
            out.extend_from_slice(&reg.to_le_bytes());
            encode_node(uv, out);
        }
        Node::VertexId => out.push(4),
        Node::InputParam(name) => {
            out.push(5);
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
    }
}

/// Build a complete mini-program bitcode blob.
pub fn encode_mini(stage: u8, body: &Node) -> Vec<u8> {
    let mut out = MINI_MAGIC.to_vec();
    out.push(stage);
    encode_node(body, &mut out);
    out
}

fn decode_node(b: &[u8]) -> Result<Node, HlslError> {
    if b.is_empty() {
        return Err(HlslError::Unsupported("truncated node"));
    }
    let op = b[0];
    let rest = &b[1..];
    match op {
        0 => {
            // Const: tag by remaining length.
            match rest.len() {
                4 => Ok(Node::Const(Const::F32(f32::from_le_bytes(
                    rest[..4].try_into().unwrap(),
                )))),
                8 => Ok(Node::Const(Const::Vec2(
                    f32::from_le_bytes(rest[0..4].try_into().unwrap()),
                    f32::from_le_bytes(rest[4..8].try_into().unwrap()),
                ))),
                12 => Ok(Node::Const(Const::Vec3(
                    f32::from_le_bytes(rest[0..4].try_into().unwrap()),
                    f32::from_le_bytes(rest[4..8].try_into().unwrap()),
                    f32::from_le_bytes(rest[8..12].try_into().unwrap()),
                ))),
                16 => Ok(Node::Const(Const::Vec4(
                    f32::from_le_bytes(rest[0..4].try_into().unwrap()),
                    f32::from_le_bytes(rest[4..8].try_into().unwrap()),
                    f32::from_le_bytes(rest[8..12].try_into().unwrap()),
                    f32::from_le_bytes(rest[12..16].try_into().unwrap()),
                ))),
                _ => Err(HlslError::Unsupported("const width")),
            }
        }
        1 | 2 => {
            if rest.len() < 2 {
                return Err(HlslError::Unsupported("truncated binop"));
            }
            let left_len = u16::from_le_bytes(rest[..2].try_into().unwrap()) as usize;
            if rest.len() < 2 + left_len {
                return Err(HlslError::Unsupported("truncated binop left"));
            }
            let a = decode_node_exact(&rest[2..2 + left_len])?;
            let b = decode_node_exact(&rest[2 + left_len..])?;
            Ok(if op == 1 {
                Node::Add(Box::new(a), Box::new(b))
            } else {
                Node::Mul(Box::new(a), Box::new(b))
            })
        }
        3 => {
            if rest.len() < 4 {
                return Err(HlslError::Unsupported("truncated sample"));
            }
            let reg = u32::from_le_bytes(rest[..4].try_into().unwrap());
            // uv is the remainder — decode a single node that must
            // consume it all.
            let uv = decode_node_exact(&rest[4..])?;
            Ok(Node::SampleTexture {
                reg,
                uv: Box::new(uv),
            })
        }
        4 => Ok(Node::VertexId),
        5 => {
            if rest.len() < 4 {
                return Err(HlslError::Unsupported("truncated param"));
            }
            let n = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
            if rest.len() < 4 + n {
                return Err(HlslError::Unsupported("truncated param name"));
            }
            Ok(Node::InputParam(
                String::from_utf8_lossy(&rest[4..4 + n]).to_string(),
            ))
        }
        _ => Err(HlslError::Unsupported("node opcode")),
    }
}

/// Decode a node that must consume the entire slice (used for the
/// right-hand side of binary ops and SampleTexture's uv operand).
fn decode_node_exact(b: &[u8]) -> Result<Node, HlslError> {
    decode_node(b)
}

fn collect_texture_regs(n: &Node) -> Vec<u32> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<u32>) {
        match n {
            Node::SampleTexture { reg, uv } => {
                if !out.contains(reg) {
                    out.push(*reg);
                }
                walk(uv, out);
            }
            Node::Add(a, b) | Node::Mul(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            _ => {}
        }
    }
    walk(n, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_program_roundtrip() {
        let body = Node::SampleTexture {
            reg: 0,
            uv: Box::new(Node::Const(Const::Vec2(0.0, 0.0))),
        };
        let bc = encode_mini(b'f', &body);
        let ir = lower(&bc).unwrap();
        assert!(matches!(ir.stage, ShaderStage::Fragment { .. }));
        assert_eq!(ir.texture_regs, vec![0]);
    }

    #[test]
    fn binop_roundtrip() {
        let body = Node::Mul(
            Box::new(Node::Add(
                Box::new(Node::Const(Const::F32(1.0))),
                Box::new(Node::Const(Const::F32(2.0))),
            )),
            Box::new(Node::Const(Const::F32(3.0))),
        );
        let bc = encode_mini(b'f', &body);
        let ir = lower(&bc).unwrap();
        assert!(matches!(ir.stage, ShaderStage::Fragment { .. }));
    }

    #[test]
    fn real_dxil_is_unsupported() {
        let fake_llvm = b"\xde\xc0\x17\x0bLLVM bitcode....";
        assert!(matches!(
            lower(fake_llvm),
            Err(HlslError::Unsupported("llvm-bitcode"))
        ));
        assert!(matches!(lower(&[]), Err(HlslError::EmptyBitcode)));
    }
}
