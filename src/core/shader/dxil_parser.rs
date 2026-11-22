//! DXIL container parser — Phase 10.
//!
//! Parses the **DXBC container** that wraps DXIL bitcode in modern
//! DirectX shaders:
//!
//! ```text
//! "DXBC" | hash(16) | fourcc count | part offsets/sizes ... | parts
//! ```
//!
//! Each part has a 4-byte fourcc (`ISGN`/`OSGN` input/output signatures,
//! `SFI0` shader feature info, `HASH`, `DXIL` the LLVM bitcode blob).
//!
//! What we extract:
//! - container + part hashes (cache keys)
//! - ISGN/OSGN signature records (name, semantic, component type)
//! - the raw DXIL bitcode blob (handed to [`crate::core::shader::hlsl_compiler`])
//!
//! We do **not** decode the LLVM bitcode itself — that is the job of
//! the HLSL intermediate layer, which currently supports a documented
//! subset and reports `Unsupported` for arbitrary shaders.

use std::fmt;

/// FourCC part kinds found in a DXBC container.
pub mod parts {
    pub const HASH: u32 = u32::from_le_bytes(*b"HASH");
    pub const ISGN: u32 = u32::from_le_bytes(*b"ISGN");
    pub const OSGN: u32 = u32::from_le_bytes(*b"OSGN");
    pub const SFI0: u32 = u32::from_le_bytes(*b"SFI0");
    pub const DXIL: u32 = u32::from_le_bytes(*b"DXIL");
    pub const RDAT: u32 = u32::from_le_bytes(*b"RDAT");
    pub const STAT: u32 = u32::from_le_bytes(*b"STAT");
    pub const EOS: u32 = u32::from_le_bytes(*b"\0SOT"); // end-of-stream marker
}

/// One parsed signature parameter (vertex input / pixel output).
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureParam {
    /// Semantic name (e.g. `POSITION`, `TEXCOORD`, `SV_Target`).
    pub name: String,
    /// System-value or user semantic name.
    pub semantic: String,
    /// Component type enum value (as stored, e.g. 3 = float3).
    pub component_type: u32,
}

/// Parsed DXIL container.
#[derive(Debug, Clone)]
pub struct DxilAst {
    /// SHA-ish container hash part (first 16 bytes of HASH part).
    pub container_hash: [u8; 16],
    /// Raw DXIL (LLVM bitcode) blob.
    pub bitcode: Vec<u8>,
    /// ISGN signature params (may be empty for pure compute).
    pub input_signature: Vec<SignatureParam>,
    /// OSGN signature params.
    pub output_signature: Vec<SignatureParam>,
    /// Part names present (for diagnostics).
    pub part_names: Vec<String>,
}

/// Parse errors.
#[derive(Debug)]
pub enum DxilError {
    /// Not a DXBC container (bad magic or truncated header).
    NotDxbc,
    /// Container references data outside the buffer.
    Truncated,
    /// Malformed signature blob.
    BadSignature,
}

impl fmt::Display for DxilError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDxbc => write!(f, "not a DXBC/DXIL container"),
            Self::Truncated => write!(f, "truncated DXBC container"),
            Self::BadSignature => write!(f, "malformed ISGN/OSGN signature"),
        }
    }
}

impl std::error::Error for DxilError {}

fn u32_at(b: &[u8], off: usize) -> Result<u32, DxilError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or(DxilError::Truncated)
}

fn fourcc(v: u32) -> String {
    String::from_utf8_lossy(&v.to_le_bytes()).to_string()
}

/// Parse a DXBC-wrapped DXIL shader.
pub fn parse_dxbc(bytes: &[u8]) -> Result<DxilAst, DxilError> {
    // Header: "DXBC"(4) checksum(16) one(4) total_size(4) part_count(4)
    //         then part_count × {offset(4) size(4)}.
    if bytes.len() < 32 || &bytes[0..4] != b"DXBC" {
        return Err(DxilError::NotDxbc);
    }
    let checksum: [u8; 16] = bytes[4..20].try_into().unwrap();
    let _one = u32_at(bytes, 20)?;
    let total = u32_at(bytes, 24)? as usize;
    let count = u32_at(bytes, 28)? as usize;
    if total > bytes.len() || 32 + count * 8 > bytes.len() {
        return Err(DxilError::Truncated);
    }

    let mut hash = [0u8; 16];
    let mut bitcode = Vec::new();
    let mut input_sig = Vec::new();
    let mut output_sig = Vec::new();
    let mut part_names = Vec::new();

    for i in 0..count {
        let off = u32_at(bytes, 32 + i * 8)? as usize;
        let size = u32_at(bytes, 36 + i * 8)? as usize;
        if off + size > bytes.len() {
            return Err(DxilError::Truncated);
        }
        let part = &bytes[off..off + size];
        // Part header: fourcc(4) size(4) then payload.
        if part.len() < 8 {
            continue;
        }
        let kind = u32::from_le_bytes(part[0..4].try_into().unwrap());
        let payload = &part[8..];
        part_names.push(fourcc(kind));
        match kind {
            parts::HASH => {
                hash.copy_from_slice(&checksum);
            }
            parts::ISGN => input_sig = parse_signature(payload)?,
            parts::OSGN => output_sig = parse_signature(payload)?,
            parts::DXIL => bitcode = payload.to_vec(),
            _ => {}
        }
    }

    if bitcode.is_empty() && part_names.iter().any(|n| n == "DXIL") {
        return Err(DxilError::Truncated);
    }
    // Some containers omit the DXIL part for pure metadata shaders.
    Ok(DxilAst {
        container_hash: hash,
        bitcode,
        input_signature: input_sig,
        output_signature: output_sig,
        part_names,
    })
}

/// Parse an ISGN/OSGN signature blob.
///
/// Layout (little-endian):
///   param_count u32 | param_stride u32 | rasterized_stream u32 |
///   param_records[param_count] of {name[4] semantic_idx u32
///   component_type u32 rows u8 cols u8 ...}
///
/// The semantic index table follows the records; we resolve indices
/// back to names where possible.
pub fn parse_signature(blob: &[u8]) -> Result<Vec<SignatureParam>, DxilError> {
    if blob.len() < 12 {
        return Ok(Vec::new());
    }
    let count = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
    let stride = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
    if stride < 24 || 12 + count * stride > blob.len() {
        // Unknown layout — report empty rather than misparse.
        return Ok(Vec::new());
    }
    // Semantic string table sits after the param records.
    let table_off = 12 + count * stride;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rec = &blob[12 + i * stride..12 + (i + 1) * stride];
        let mut name = [0u8; 4];
        name.copy_from_slice(&rec[0..4]);
        let semantic_idx = u32::from_le_bytes(rec[4..8].try_into().unwrap()) as usize;
        let component_type = u32::from_le_bytes(rec[8..12].try_into().unwrap());
        let semantic = read_semantic(blob, table_off, semantic_idx);
        out.push(SignatureParam {
            name: String::from_utf8_lossy(&name)
                .trim_end_matches('\0')
                .to_string(),
            semantic,
            component_type,
        });
    }
    Ok(out)
}

/// Read a NUL-terminated semantic name at `idx` in the string table.
fn read_semantic(blob: &[u8], table_off: usize, idx: usize) -> String {
    // The table is a sequence of NUL-terminated strings; index selects
    // the idx-th string.
    let mut off = table_off;
    for _ in 0..idx {
        while off < blob.len() && blob[off] != 0 {
            off += 1;
        }
        off += 1; // skip NUL
    }
    let start = off;
    while off < blob.len() && blob[off] != 0 {
        off += 1;
    }
    String::from_utf8_lossy(&blob[start..off])
        .trim_end_matches('\0')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal DXBC container with ISGN + DXIL parts.
    fn build_dxbc() -> Vec<u8> {
        let isgn = {
            // 1 param: name="POS ", semantic_idx=0 ("POSITION"), type=3
            let mut b = Vec::new();
            b.extend_from_slice(&1u32.to_le_bytes()); // count
            b.extend_from_slice(&32u32.to_le_bytes()); // stride
            b.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // rasterized
            b.extend_from_slice(b"POS ");
            b.extend_from_slice(&0u32.to_le_bytes()); // semantic idx
            b.extend_from_slice(&3u32.to_le_bytes()); // component type
            b.extend_from_slice(&[3u8, 1, 0, 0]); // rows=3? cols=1 packed
            b.resize(12 + 32, 0);
            b.extend_from_slice(b"POSITION\0"); // semantic table
            b
        };
        let dxil = b"\x42\x43\xC0\xDEfake-llvm-bitcode".to_vec();
        let mut parts = Vec::new();
        let mut payload = Vec::new();
        for (kind, data) in [(parts::ISGN, isgn), (parts::DXIL, dxil)] {
            let off_pos = payload.len();
            let mut p = Vec::new();
            p.extend_from_slice(&kind.to_le_bytes());
            p.extend_from_slice(&(data.len() as u32 + 8).to_le_bytes());
            p.extend_from_slice(&data);
            payload.extend_from_slice(&p);
            // Payload base = 32-byte header + 2 part-table entries (16 B).
            parts.extend_from_slice(&((48 + off_pos) as u32).to_le_bytes());
            parts.extend_from_slice(&(p.len() as u32).to_le_bytes());
        }
        let total = 32 + parts.len() + payload.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"DXBC");
        out.extend_from_slice(&[0xAB; 16]); // checksum
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&parts);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn parses_dxbc_container() {
        let ast = parse_dxbc(&build_dxbc()).unwrap();
        assert_eq!(ast.bitcode, b"\x42\x43\xC0\xDEfake-llvm-bitcode");
        assert_eq!(ast.input_signature.len(), 1);
        assert_eq!(ast.input_signature[0].name.trim(), "POS");
        assert_eq!(ast.input_signature[0].semantic, "POSITION");
        assert!(ast.part_names.iter().any(|n| n == "ISGN"));
        assert!(ast.part_names.iter().any(|n| n == "DXIL"));
    }

    #[test]
    fn rejects_non_dxbc() {
        assert!(matches!(parse_dxbc(b"hello"), Err(DxilError::NotDxbc)));
        assert!(matches!(parse_dxbc(&[0u8; 64]), Err(DxilError::NotDxbc)));
    }
}
