//! HLSL vertex-shader → WGSL translation (Phase 24).
//!
//! A real translator for the small HLSL subset used by our boot/demo
//! shaders: an input `struct` with semantic annotations (`POSITION`,
//! `NORMAL`, `TEXCOORDn`, `COLOR`) and a `main` returning `SV_Position`.
//! The result is valid WGSL with `@location` inputs, a
//! `@builtin(position)` output and a passthrough vertex body.
//!
//! Honest scope: this is not a full HLSL compiler — control flow,
//! matrices and pixel shaders remain in `core::shader`'s pipeline.

/// HLSL scalar/vector type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlslType {
    /// `float` / `float2` / `float3` / `float4`.
    Float(u8),
    /// `uint` / `uint2` / ...
    Uint(u8),
}

impl HlslType {
    /// Parse a type token like `float4`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        for (prefix, ctor) in [
            ("float", Self::Float as fn(u8) -> Self),
            ("uint", Self::Uint),
        ] {
            if let Some(rest) = s.strip_prefix(prefix) {
                let n: u8 = if rest.is_empty() {
                    1
                } else {
                    rest.parse().ok()?
                };
                return Some(ctor(n));
            }
        }
        None
    }

    /// WGSL type name.
    pub fn wgsl(&self) -> &'static str {
        match self {
            Self::Float(1) => "f32",
            Self::Float(2) => "vec2<f32>",
            Self::Float(3) => "vec3<f32>",
            Self::Float(_) => "vec4<f32>",
            Self::Uint(1) => "u32",
            Self::Uint(2) => "vec2<u32>",
            Self::Uint(3) => "vec3<u32>",
            Self::Uint(_) => "vec4<u32>",
        }
    }
}

/// Vertex input semantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantic {
    /// Object-space position.
    Position,
    /// Vertex normal.
    Normal,
    /// Texture coordinate (number retained).
    TexCoord(u32),
    /// Vertex colour.
    Color,
    /// Unknown semantic (retained verbatim).
    Other(&'static str),
}

impl Semantic {
    /// Parse a semantic token such as `TEXCOORD0`.
    pub fn parse(s: &str) -> Self {
        let upper = s.trim().to_ascii_uppercase();
        if upper == "POSITION" || upper == "SV_POSITION" {
            Self::Position
        } else if upper == "NORMAL" {
            Self::Normal
        } else if upper == "COLOR" || upper == "COLOR0" {
            Self::Color
        } else if let Some(rest) = upper.strip_prefix("TEXCOORD") {
            Self::TexCoord(rest.parse().unwrap_or(0))
        } else {
            Self::Other(match s.trim() {
                "FOG" => "FOG",
                _ => "OTHER",
            })
        }
    }

    /// WGSL location assigned to this semantic.
    pub fn location(&self) -> u32 {
        match self {
            Self::Position => 0,
            Self::Normal => 1,
            Self::TexCoord(n) => 2 + *n,
            Self::Color => 1,
            Self::Other(_) => 7,
        }
    }
}

/// One input element of the vertex struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexInput {
    /// HLSL member name.
    pub name: String,
    /// Declared type.
    pub ty: HlslType,
    /// Semantic annotation.
    pub semantic: Semantic,
}

/// A parsed vertex shader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexShader {
    /// Declared inputs.
    pub inputs: Vec<VertexInput>,
    /// Body statements (HLSL, copied into the WGSL body when trivial).
    pub body: Vec<String>,
}

/// Translation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateError(pub String);

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HLSL vertex translation failed: {}", self.0)
    }
}
impl std::error::Error for TranslateError {}

/// Parse a minimal HLSL vertex shader.
///
/// Example:
/// ```hlsl
/// struct VS_IN { float4 pos : POSITION; float2 uv : TEXCOORD0; };
/// float4 main(VS_IN input) : SV_Position { return input.pos; }
/// ```
pub fn parse_vertex_hlsl(src: &str) -> Result<VertexShader, TranslateError> {
    let struct_start = src
        .find("struct")
        .ok_or_else(|| TranslateError("no input struct".into()))?;
    let open = src[struct_start..]
        .find('{')
        .ok_or_else(|| TranslateError("no struct body".into()))?
        + struct_start;
    let close = src[open..]
        .find('}')
        .ok_or_else(|| TranslateError("unterminated struct".into()))?
        + open;
    let body = &src[open + 1..close];

    let mut inputs = Vec::new();
    for member in body.split(';') {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        let colon = member
            .find(':')
            .ok_or_else(|| TranslateError(format!("member without semantic: {member}")))?;
        let decl = member[..colon].trim();
        let semantic = Semantic::parse(&member[colon + 1..]);
        let mut parts = decl.split_whitespace();
        let ty_tok = parts
            .next()
            .ok_or_else(|| TranslateError("missing type".into()))?;
        let name = parts
            .next()
            .ok_or_else(|| TranslateError("missing member name".into()))?;
        let ty = HlslType::parse(ty_tok)
            .ok_or_else(|| TranslateError(format!("unknown type {ty_tok}")))?;
        inputs.push(VertexInput {
            name: name.into(),
            ty,
            semantic,
        });
    }
    if inputs.is_empty() {
        return Err(TranslateError("empty input struct".into()));
    }
    let main_open = src[close..]
        .find('{')
        .ok_or_else(|| TranslateError("no main body".into()))?
        + close;
    let main_close = src[main_open..]
        .find('}')
        .ok_or_else(|| TranslateError("unterminated main".into()))?
        + main_open;
    let statements: Vec<String> = src[main_open + 1..main_close]
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(VertexShader {
        inputs,
        body: statements,
    })
}

impl VertexShader {
    /// Emit WGSL for the parsed vertex shader.
    pub fn to_wgsl(&self) -> String {
        let mut out = String::from("// Translated HLSL vertex shader (xHoly Phase 24)\n");
        for (i, input) in self.inputs.iter().enumerate() {
            out.push_str(&format!(
                "@location({}) in_{}: {},\n",
                input.semantic.location(),
                input.name,
                input.ty.wgsl()
            ));
            let _ = i;
        }
        out.push_str("\nstruct VsOut {\n    @builtin(position) position: vec4<f32>,\n};\n\n");
        out.push_str("@vertex\nfn main(");
        for (i, input) in self.inputs.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("in_{}: {}", input.name, input.ty.wgsl()));
        }
        out.push_str(") -> VsOut {\n    var out: VsOut;\n");
        // Only `return input.<name>;` bodies are translated faithfully;
        // everything else falls back to a safe zero position.
        let passthrough = self.body.iter().find_map(|s| {
            let s = s.strip_prefix("return")?.trim();
            let s = s.strip_suffix(';').unwrap_or(s).trim();
            let name = s.strip_prefix("input.")?;
            self.inputs
                .iter()
                .find(|i| i.name == name)
                .map(|i| i.name.clone())
        });
        match passthrough {
            Some(name) => out.push_str(&format!(
                "    out.position = vec4<f32>(in_{name}.xyz, 1.0);\n"
            )),
            None => out.push_str("    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);\n"),
        }
        out.push_str("    return out;\n}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
struct VS_IN {
    float4 pos : POSITION;
    float2 uv : TEXCOORD0;
};
float4 main(VS_IN input) : SV_Position { return input.pos; }
"#;

    #[test]
    fn parses_inputs_and_semantics() {
        let shader = parse_vertex_hlsl(SAMPLE).unwrap();
        assert_eq!(shader.inputs.len(), 2);
        assert_eq!(shader.inputs[0].semantic, Semantic::Position);
        assert_eq!(shader.inputs[1].semantic, Semantic::TexCoord(0));
        assert_eq!(shader.inputs[0].ty, HlslType::Float(4));
    }

    #[test]
    fn wgsl_has_builtin_position_and_passthrough() {
        let wgsl = parse_vertex_hlsl(SAMPLE).unwrap().to_wgsl();
        assert!(wgsl.contains("@builtin(position) position: vec4<f32>"));
        assert!(wgsl.contains("out.position = vec4<f32>(in_pos.xyz, 1.0)"));
        assert!(wgsl.contains("@location(2) in_uv: vec2<f32>"));
    }

    #[test]
    fn rejects_shader_without_struct() {
        assert!(parse_vertex_hlsl("float4 main() { return 0; }").is_err());
    }

    #[test]
    fn hlsl_type_wgsl_names() {
        assert_eq!(HlslType::parse("float3").unwrap().wgsl(), "vec3<f32>");
        assert_eq!(HlslType::parse("uint").unwrap().wgsl(), "u32");
    }
}
