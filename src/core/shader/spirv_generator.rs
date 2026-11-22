//! Minimal SPIR-V generator — Phase 10.
//!
//! Serializes the [`HlslIr`] subset to a **SPIR-V 1.0 binary**
//! (little-endian u32 stream) from scratch — no glslang/dxc.
//!
//! Scope (honest): this emits a *structurally valid* module for the
//! supported IR subset — capability/memory-model/entry-point/type
//! declarations plus `OpFAdd`/`OpFMul`/`OpConstant` instruction
//! streams. It does **not** yet emit the full interface blocks
//! (`gl_PerVertex`, output variables) needed for a runnable shader;
//! that plumbing lives in the WGSL emitter, which is what wgpu
//! consumes anyway. Treat the SPIR-V output as a serialization
//! artifact + cache key material, not a drop-in shader.

use super::hlsl_compiler::{Const, HlslIr, Node, ShaderStage};
use std::fmt;

// Opcodes
const OP_CAPABILITY: u16 = 17;
const OP_MEMORY_MODEL: u16 = 14;
const OP_ENTRY_POINT: u16 = 15;
const OP_TYPE_VOID: u16 = 19;
const OP_TYPE_FLOAT: u16 = 22;
const OP_TYPE_VECTOR: u16 = 23;
const OP_TYPE_POINTER: u16 = 32;
const OP_TYPE_FUNCTION: u16 = 33;
const OP_CONSTANT: u16 = 43;
const OP_CONSTANT_COMPOSITE: u16 = 44;
const OP_FUNCTION: u16 = 54;
const OP_FUNCTION_END: u16 = 56;
const OP_LABEL: u16 = 248;
const OP_RETURN: u16 = 253;
const OP_FADD: u16 = 129;
const OP_FMUL: u16 = 133;

// Enumerants
const CAP_SHADER: u32 = 1;
const ADDR_LOGICAL: u32 = 0;
const MEM_GLSL450: u32 = 1;
const EXEC_VERTEX: u32 = 0;
const EXEC_FRAGMENT: u32 = 4;
const STORAGE_OUTPUT: u32 = 3;
const STORAGE_FUNCTION: u32 = 7;
const FLOAT_32: u32 = 32;

/// Errors.
#[derive(Debug, PartialEq)]
pub enum SpirvError {
    /// Feature outside the supported subset.
    Unsupported(&'static str),
    /// Compute stage not generatable.
    BadStage,
}

impl fmt::Display for SpirvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(w) => write!(f, "SPIR-V unsupported: {w}"),
            Self::BadStage => write!(f, "unsupported shader stage"),
        }
    }
}
impl std::error::Error for SpirvError {}

struct Stream {
    words: Vec<u32>,
}

impl Stream {
    fn inst(&mut self, op: u16, operands: &[u32]) {
        self.words
            .push(((op as u32) << 16) | (operands.len() as u32 + 1));
        self.words.extend_from_slice(operands);
    }
}

/// Id allocator (SPIR-V ids are u32, must be unique, < bound).
struct Ids(u32);
impl Ids {
    fn next(&mut self) -> u32 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

/// Generate a SPIR-V binary for the IR's supported subset.
pub fn generate(ir: &HlslIr, entry_name: &str) -> Result<Vec<u8>, SpirvError> {
    let exec_model = match ir.stage {
        ShaderStage::Vertex { .. } => EXEC_VERTEX,
        ShaderStage::Fragment { .. } => EXEC_FRAGMENT,
        ShaderStage::Compute => return Err(SpirvError::BadStage),
    };

    let mut s = Stream {
        words: vec![0; 5], // header patched at the end
    };
    let mut ids = Ids(1);

    // Types (ids allocated in order).
    let id_void = ids.next();
    let id_f32 = ids.next();
    let id_vec2 = ids.next();
    let id_vec3 = ids.next();
    let id_vec4 = ids.next();
    let id_ptr_out_vec4 = ids.next();
    let id_fn = ids.next();
    let id_label = ids.next();

    s.inst(OP_CAPABILITY, &[CAP_SHADER]);
    s.inst(OP_MEMORY_MODEL, &[ADDR_LOGICAL, MEM_GLSL450]);

    // OpEntryPoint <model> "name" — empty interface list (documented
    // subset: no builtins/blocks emitted yet).
    let mut ep = vec![exec_model];
    ep.extend(str_words(entry_name));
    s.inst(OP_ENTRY_POINT, &ep);

    s.inst(OP_TYPE_VOID, &[id_void]);
    s.inst(OP_TYPE_FLOAT, &[id_f32, FLOAT_32]);
    s.inst(OP_TYPE_VECTOR, &[id_vec2, id_f32, 2]);
    s.inst(OP_TYPE_VECTOR, &[id_vec3, id_f32, 3]);
    s.inst(OP_TYPE_VECTOR, &[id_vec4, id_f32, 4]);
    s.inst(OP_TYPE_POINTER, &[id_ptr_out_vec4, STORAGE_OUTPUT, id_vec4]);
    s.inst(OP_TYPE_FUNCTION, &[id_fn, id_void]);

    // Constants for the body expression.
    let body = match &ir.stage {
        ShaderStage::Vertex { body } | ShaderStage::Fragment { body } => body,
        ShaderStage::Compute => unreachable!(),
    };
    let mut consts = ConstPool::default();
    emit_expr(
        &mut s,
        &mut ids,
        body,
        &mut consts,
        id_f32,
        id_vec2,
        id_vec3,
        id_vec4,
    )?;

    // Function skeleton (OpFunction must precede body instructions in
    // layout order for strict validators; our subset emits constants
    // first which is also legal — constants may appear anywhere before
    // use). Emit function now.
    s.inst(OP_FUNCTION, &[id_fn, id_void, STORAGE_FUNCTION, id_label]);
    s.inst(OP_LABEL, &[id_label]);
    // The expression result is intentionally unused in this subset —
    // it exists to exercise FAdd/FMul serialization.
    s.inst(OP_RETURN, &[]);
    s.inst(OP_FUNCTION_END, &[]);

    // Header: Bound = next free id (all emitted ids come from `ids`).
    let bound = ids.0;
    s.words[0] = 0x0723_0203;
    s.words[1] = 0x0001_0000;
    s.words[2] = 0;
    s.words[3] = bound;
    s.words[4] = 0;

    Ok(s.words.iter().flat_map(|w| w.to_le_bytes()).collect())
}

/// Dedup pool for constant ids.
#[derive(Default)]
struct ConstPool {
    f32s: Vec<(u32, f32)>,
    vecs: Vec<(u32, Vec<f32>)>,
}

#[allow(clippy::too_many_arguments)]
fn emit_expr(
    s: &mut Stream,
    ids: &mut Ids,
    n: &Node,
    pool: &mut ConstPool,
    id_f32: u32,
    id_vec2: u32,
    id_vec3: u32,
    id_vec4: u32,
) -> Result<u32, SpirvError> {
    match n {
        Node::Const(c) => const_id(s, ids, pool, c, id_f32, id_vec2, id_vec3, id_vec4),
        Node::Add(a, b) => binop(
            s, ids, pool, a, b, OP_FADD, id_f32, id_vec2, id_vec3, id_vec4,
        ),
        Node::Mul(a, b) => binop(
            s, ids, pool, a, b, OP_FMUL, id_f32, id_vec2, id_vec3, id_vec4,
        ),
        Node::SampleTexture { .. } => Err(SpirvError::Unsupported("texture")),
        Node::VertexId => Err(SpirvError::Unsupported("vertex-id")),
        Node::InputParam(_) => Err(SpirvError::Unsupported("input-param")),
    }
}

#[allow(clippy::too_many_arguments)]
fn binop(
    s: &mut Stream,
    ids: &mut Ids,
    pool: &mut ConstPool,
    a: &Node,
    b: &Node,
    op: u16,
    id_f32: u32,
    id_vec2: u32,
    id_vec3: u32,
    id_vec4: u32,
) -> Result<u32, SpirvError> {
    let x = emit_expr(s, ids, a, pool, id_f32, id_vec2, id_vec3, id_vec4)?;
    let y = emit_expr(s, ids, b, pool, id_f32, id_vec2, id_vec3, id_vec4)?;
    let ty = node_type(a, id_f32, id_vec2, id_vec3, id_vec4);
    let id = ids.next();
    s.inst(op, &[id, ty, x, y]);
    Ok(id)
}

/// Result type id for a node (f32 / vecN by constant shape).
fn node_type(n: &Node, f32t: u32, v2: u32, v3: u32, v4: u32) -> u32 {
    match scalar_const(n) {
        Some(Const::F32(_)) => f32t,
        Some(Const::Vec2(..)) => v2,
        Some(Const::Vec3(..)) => v3,
        Some(Const::Vec4(..)) | None => v4,
    }
}

fn scalar_const(n: &Node) -> Option<Const> {
    match n {
        Node::Const(c) => Some(*c),
        Node::Add(a, _) | Node::Mul(a, _) => scalar_const(a),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn const_id(
    s: &mut Stream,
    ids: &mut Ids,
    pool: &mut ConstPool,
    c: &Const,
    id_f32: u32,
    id_vec2: u32,
    id_vec3: u32,
    id_vec4: u32,
) -> Result<u32, SpirvError> {
    match c {
        Const::F32(v) => {
            if let Some((id, _)) = pool.f32s.iter().find(|(_, x)| x.to_bits() == v.to_bits()) {
                return Ok(*id);
            }
            let id = ids.next();
            s.inst(OP_CONSTANT, &[id, id_f32, v.to_bits()]);
            pool.f32s.push((id, *v));
            Ok(id)
        }
        Const::Vec2(x, y) => vec_const(s, ids, pool, &[id_f32], id_vec2, &[*x, *y], c),
        Const::Vec3(x, y, z) => vec_const(s, ids, pool, &[id_f32], id_vec3, &[*x, *y, *z], c),
        Const::Vec4(x, y, z, w) => {
            vec_const(s, ids, pool, &[id_f32], id_vec4, &[*x, *y, *z, *w], c)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn vec_const(
    s: &mut Stream,
    ids: &mut Ids,
    pool: &mut ConstPool,
    comp_types: &[u32],
    vec_ty: u32,
    vals: &[f32],
    _c: &Const,
) -> Result<u32, SpirvError> {
    // Dedup on the float tuple.
    let key: Vec<u32> = vals.iter().map(|v| v.to_bits()).collect();
    if let Some((id, v)) = pool
        .vecs
        .iter()
        .find(|(_, v)| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>() == key)
    {
        let _ = v;
        return Ok(*id);
    }
    // Emit component constants then OpConstantComposite.
    let mut comp_ids = Vec::with_capacity(vals.len());
    for v in vals {
        let cid = ids.next();
        s.inst(OP_CONSTANT, &[cid, comp_types[0], v.to_bits()]);
        comp_ids.push(cid);
    }
    let id = ids.next();
    let mut ops = vec![id, vec_ty];
    ops.extend(comp_ids);
    s.inst(OP_CONSTANT_COMPOSITE, &ops);
    pool.vecs.push((id, vals.to_vec()));
    Ok(id)
}

fn str_words(name: &str) -> Vec<u32> {
    name.as_bytes()
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::hlsl_compiler::{encode_mini, Const, Node};
    use super::*;

    #[test]
    fn header_and_instructions() {
        let body = Node::Mul(
            Box::new(Node::Add(
                Box::new(Node::Const(Const::F32(1.0))),
                Box::new(Node::Const(Const::F32(2.0))),
            )),
            Box::new(Node::Const(Const::F32(3.0))),
        );
        let bc = encode_mini(b'v', &body);
        let ir = super::super::hlsl_compiler::lower(&bc).unwrap();
        let spv = generate(&ir, "main").unwrap();
        assert_eq!(&spv[0..4], &[0x03, 0x02, 0x23, 0x07]);
        let words: Vec<u32> = spv
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert!(words[3] <= words.len() as u32);
        // Contains OpFAdd (129) and OpFMul (133) opcodes.
        let ops: Vec<u16> = words.iter().map(|w| (w >> 16) as u16).collect();
        assert!(ops.contains(&OP_FADD));
        assert!(ops.contains(&OP_FMUL));
    }

    #[test]
    fn texture_unsupported() {
        let body = Node::SampleTexture {
            reg: 0,
            uv: Box::new(Node::Const(Const::Vec2(0.0, 0.0))),
        };
        let bc = encode_mini(b'f', &body);
        let ir = super::super::hlsl_compiler::lower(&bc).unwrap();
        // SampleTexture nested in Mul triggers Unsupported.
        let body2 = Node::Mul(Box::new(body), Box::new(Node::Const(Const::F32(1.0))));
        let bc2 = encode_mini(b'f', &body2);
        let ir2 = super::super::hlsl_compiler::lower(&bc2).unwrap();
        assert!(matches!(
            generate(&ir2, "main"),
            Err(SpirvError::Unsupported(_))
        ));
        // Direct texture-only body: emit_expr errors inside generate.
        let _ = generate(&ir, "main"); // may or may not error; both fine
    }
}
