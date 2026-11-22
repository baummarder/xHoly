//! Cheat compiler — tiny text format → patches.
//!
//! Format (one cheat per line, `#` comments):
//! ```text
//! addr 0xADDR size N value 0xVV [0xVV ...]
//! addr 0xADDR type u32 value 1000
//! ```
//! This is *not* Lua — it's a declarative patch list. Lua→bytecode
//! compilation is documented future work (see docs/phase-11.5.md).

use super::cheat_value::CheatValue;

/// A compiled patch: write `bytes` at guest `address`.
#[derive(Debug, Clone, PartialEq)]
pub struct CheatPatch {
    /// Guest virtual address.
    pub address: u64,
    /// Bytes to write.
    pub bytes: Vec<u8>,
}

/// Compile errors.
#[derive(Debug, PartialEq)]
pub enum CompileError {
    /// Empty/whitespace line handled silently; real errors:
    BadAddress,
    BadValue,
    UnknownType(String),
    Truncated,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadAddress => write!(f, "bad address"),
            Self::BadValue => write!(f, "bad value"),
            Self::UnknownType(t) => write!(f, "unknown type '{t}'"),
            Self::Truncated => write!(f, "truncated cheat line"),
        }
    }
}
impl std::error::Error for CompileError {}

/// Parse hex/dec integer.
fn parse_u64(s: &str) -> Option<u64> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Compile a cheat script into patches. Blank lines and `#` comments
/// are skipped; invalid lines produce an error with line number.
pub fn compile(script: &str) -> Result<Vec<CheatPatch>, (usize, CompileError)> {
    let mut out = Vec::new();
    for (idx, raw) in script.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        let res = compile_line(&toks);
        match res {
            Ok(p) => out.push(p),
            Err(e) => return Err((idx + 1, e)),
        }
    }
    Ok(out)
}

fn compile_line(t: &[&str]) -> Result<CheatPatch, CompileError> {
    // Expected: addr <A> [type <T> | size <N> value <hex...> | value <v>]
    if t.len() < 2 || t[0] != "addr" {
        return Err(CompileError::BadAddress);
    }
    let address = parse_u64(t[1]).ok_or(CompileError::BadAddress)?;
    if t.len() < 3 {
        return Err(CompileError::Truncated);
    }
    match t[2] {
        "value" => {
            // addr A value V  → infer type by magnitude
            if t.len() < 4 {
                return Err(CompileError::Truncated);
            }
            let v = CheatValue::parse_int(t[3]).ok_or(CompileError::BadValue)?;
            Ok(CheatPatch {
                address,
                bytes: v.to_bytes(),
            })
        }
        "type" => {
            // addr A type u32 value 1000
            if t.len() < 6 || t[4] != "value" {
                return Err(CompileError::Truncated);
            }
            let bytes = match t[3] {
                "u8" => vec![parse_u64(t[5]).ok_or(CompileError::BadValue)? as u8],
                "u16" => (parse_u64(t[5]).ok_or(CompileError::BadValue)? as u16)
                    .to_le_bytes()
                    .to_vec(),
                "u32" => (parse_u64(t[5]).ok_or(CompileError::BadValue)? as u32)
                    .to_le_bytes()
                    .to_vec(),
                "u64" => parse_u64(t[5])
                    .ok_or(CompileError::BadValue)?
                    .to_le_bytes()
                    .to_vec(),
                "f32" => (t[5].parse::<f32>().map_err(|_| CompileError::BadValue)?)
                    .to_bits()
                    .to_le_bytes()
                    .to_vec(),
                other => return Err(CompileError::UnknownType(other.into())),
            };
            Ok(CheatPatch { address, bytes })
        }
        "size" => {
            // addr A size N value B0 B1 ... BN-1
            if t.len() < 6 || t[4] != "value" {
                return Err(CompileError::Truncated);
            }
            let n: usize = t[3].parse().map_err(|_| CompileError::BadValue)?;
            if t.len() < 5 + n {
                return Err(CompileError::Truncated);
            }
            let mut bytes = Vec::with_capacity(n);
            for s in &t[5..5 + n] {
                bytes.push(parse_u64(s).ok_or(CompileError::BadValue)? as u8);
            }
            Ok(CheatPatch { address, bytes })
        }
        other => Err(CompileError::UnknownType(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_typed_values() {
        let patches = compile(
            r#"
# infinite health
addr 0x1000 type u32 value 9999
addr 0x2000 value 0xFF
addr 0x3000 size 4 value 1 2 3 4
"#,
        )
        .unwrap();
        assert_eq!(patches.len(), 3);
        assert_eq!(patches[0].address, 0x1000);
        assert_eq!(patches[0].bytes, 9999u32.to_le_bytes());
        // magnitude-inferred → U32 LE
        assert_eq!(patches[1].bytes, 255u32.to_le_bytes());
        assert_eq!(patches[2].bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn line_errors_report_number() {
        let err = compile("addr 0x1000 type u32 value 1\naddr bad\n").unwrap_err();
        assert_eq!(err.0, 2);
    }

    #[test]
    fn unknown_type_rejected() {
        assert!(compile("addr 0x1 type str value x").is_err());
    }
}
