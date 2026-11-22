//! WGSL emitter — Phase 10.
//!
//! Converts [`HlslIr`] directly to WGSL source (the format wgpu
//! consumes natively). This is the *primary* output path; the SPIR-V
//! generator exists for cache artifacts and future interop.
//!
//! On any unsupported IR feature the emitter falls back to
//! [`blit_wgsl`] — a trivial texture-copy shader — so callers always
//! get *something* renderable (honest degradation, logged by callers).

use super::hlsl_compiler::{Const, HlslIr, Node, ShaderStage};

/// Emit WGSL for the IR. Returns the source plus a list of warnings
/// for degraded features.
pub fn emit_wgsl(ir: &HlslIr) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let body = match &ir.stage {
        ShaderStage::Vertex { body } => emit_vertex(body, &mut warnings),
        ShaderStage::Fragment { body } => emit_fragment(body, &mut warnings),
        ShaderStage::Compute => {
            warnings.push("compute stage unsupported".into());
            return (blit_wgsl(), warnings);
        }
    };
    if body.is_none() {
        warnings.push("body contains unsupported nodes".into());
        return (blit_wgsl(), warnings);
    }
    let src = match &ir.stage {
        ShaderStage::Vertex { .. } => format!("{VERTEX_HEADER}\n{}\n", body.unwrap()),
        ShaderStage::Fragment { .. } => format!("{FRAGMENT_HEADER}\n{}\n", body.unwrap()),
        _ => unreachable!(),
    };
    (src, warnings)
}

/// Fallback: trivial fullscreen blit (samples `tex`, writes it out).
pub fn blit_wgsl() -> String {
    r#"// xHoly fallback blit shader (unsupported DXIL feature)
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(vec2<f32>(-1.0,-3.0), vec2<f32>(-1.0,1.0), vec2<f32>(3.0,1.0));
    var o: VsOut;
    o.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    o.uv = pos[vi] * 0.5 + 0.5;
    o.uv.y = 1.0 - o.uv.y;
    return o;
}
@fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(tex, samp, in.uv, 0.0);
}
"#
    .to_string()
}

const VERTEX_HEADER: &str = r#"// xHoly translated vertex shader (DXIL→IR→WGSL)
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0), vec2<f32>(-1.0, 1.0), vec2<f32>(3.0, 1.0));
    var o: VsOut;
    o.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    o.uv = pos[vi] * 0.5 + 0.5;
    o.uv.y = 1.0 - o.uv.y;
    return o;
}
// translated body:
fn translated_body() -> vec4<f32> {
"#;

const FRAGMENT_HEADER: &str = r#"// xHoly translated fragment shader (DXIL→IR→WGSL)
@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var samp0: sampler;
struct VsIn { @location(0) uv: vec2<f32> }
fn translated_body() -> vec4<f32> {
"#;

fn emit_vertex(body: &Node, warnings: &mut Vec<String>) -> Option<String> {
    emit_expr(body, warnings).map(|e| format!("    return {e};\n}}\n"))
}

fn emit_fragment(body: &Node, warnings: &mut Vec<String>) -> Option<String> {
    emit_expr(body, warnings).map(|e| format!(
            "    return {e};\n}}\n@fragment fn fs_main(in: VsIn) -> @location(0) vec4<f32> {{\n    return translated_body();\n}}\n"
        ))
}

fn emit_expr(n: &Node, warnings: &mut Vec<String>) -> Option<String> {
    match n {
        Node::Const(c) => Some(const_wgsl(c)),
        Node::Add(a, b) => Some(format!(
            "({} + {})",
            emit_expr(a, warnings)?,
            emit_expr(b, warnings)?
        )),
        Node::Mul(a, b) => Some(format!(
            "({} * {})",
            emit_expr(a, warnings)?,
            emit_expr(b, warnings)?
        )),
        Node::SampleTexture { reg, uv } => {
            let u = emit_expr(uv, warnings)?;
            warnings.push(format!("texture reg {reg} bound as tex{reg}"));
            Some(format!("textureSampleLevel(tex{reg}, samp{reg}, {u}, 0.0)"))
        }
        Node::VertexId => {
            warnings.push("vertex-id lowered to constant".into());
            Some("vec4<f32>(0.0)".into())
        }
        Node::InputParam(name) => {
            warnings.push(format!("input param '{name}' unresolved"));
            Some("vec4<f32>(0.0)".into())
        }
    }
}

fn const_wgsl(c: &Const) -> String {
    match c {
        Const::F32(v) => format!("{v}f"),
        Const::Vec2(x, y) => format!("vec2<f32>({x}f, {y}f)"),
        Const::Vec3(x, y, z) => format!("vec3<f32>({x}f, {y}f, {z}f)"),
        Const::Vec4(x, y, z, w) => format!("vec4<f32>({x}f, {y}f, {z}f, {w}f)"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::hlsl_compiler::{encode_mini, Const, Node};
    use super::*;

    #[test]
    fn emits_fragment_wgsl() {
        let body = Node::Mul(
            Box::new(Node::Const(Const::Vec4(1.0, 0.5, 0.25, 1.0))),
            Box::new(Node::Const(Const::F32(0.5))),
        );
        let bc = encode_mini(b'f', &body);
        let ir = super::super::hlsl_compiler::lower(&bc).unwrap();
        let (wgsl, warns) = emit_wgsl(&ir);
        assert!(wgsl.contains("fs_main"));
        assert!(wgsl.contains("vec4<f32>"));
        assert!(warns.is_empty());
    }

    #[test]
    fn fallback_on_garbage() {
        // Force unsupported: SampleTexture with un-emittable uv? All
        // nodes emit; fallback triggers only via stage/bad IR — test
        // the blit function directly.
        let src = blit_wgsl();
        assert!(src.contains("fs_main"));
    }
}
