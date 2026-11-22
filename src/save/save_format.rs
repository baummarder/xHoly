//! Save file format — versioned header + checksummed payload.
//!
//! Layout (little-endian):
//! ```text
//! magic "XHSV" (4) | version u32 | flags u32 | payload_len u64 |
//! payload_crc32 u32 | payload bytes...
//! ```
//! Payload itself is caller-defined (usually bincode-ish/serde_json
//! bytes); compression is applied *before* this layer by
//! [`super::save_compression`].

use std::fmt;

/// Current format version.
pub const FORMAT_VERSION: u32 = 1;
/// File magic.
pub const MAGIC: &[u8; 4] = b"XHSV";
/// Header size in bytes.
pub const HEADER_LEN: usize = 4 + 4 + 4 + 8 + 4;

/// Save format errors.
#[derive(Debug)]
pub enum SaveError {
    /// Bad magic / truncated header.
    BadHeader,
    /// Unsupported format version.
    UnsupportedVersion(u32),
    /// CRC mismatch (corrupt file).
    Checksum { expected: u32, actual: u32 },
    /// IO error.
    Io(std::io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadHeader => write!(f, "bad save header"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported save version {v}"),
            Self::Checksum { expected, actual } => {
                write!(
                    f,
                    "checksum mismatch: expected {expected:#x}, got {actual:#x}"
                )
            }
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for SaveError {}
impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Parsed header.
#[derive(Debug, Clone)]
pub struct SaveHeader {
    pub version: u32,
    /// Bit0 = compressed payload.
    pub flags: u32,
    pub payload_len: u64,
    pub crc32: u32,
}

/// What a save file carries: metadata + opaque payload.
#[derive(Debug, Clone)]
pub struct SavePayload {
    pub meta: super::save_slot::SaveMeta,
    /// Serialized game state bytes.
    pub state: Vec<u8>,
}

/// CRC-32 (IEEE) — small table-less impl is fine at our sizes.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Serialize `payload` into a versioned save file image.
pub fn encode(payload: &SavePayload) -> Vec<u8> {
    // Inner doc: JSON meta + raw state (simple, debuggable).
    let mut doc = serde_json::json!({
        "meta": payload.meta,
        "state_b64": base64_encode(&payload.state),
    });
    let body = serde_json::to_vec(&doc).unwrap_or_default();
    let crc = crc32(&body);

    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&body);
    let _ = &mut doc;
    out
}

/// Parse a save file image back into `SavePayload`.
pub fn decode(bytes: &[u8]) -> Result<SavePayload, SaveError> {
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return Err(SaveError::BadHeader);
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(SaveError::UnsupportedVersion(version));
    }
    let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    if bytes.len() < HEADER_LEN + len {
        return Err(SaveError::BadHeader);
    }
    let body = &bytes[HEADER_LEN..HEADER_LEN + len];
    if crc32(body) != crc {
        return Err(SaveError::Checksum {
            expected: crc,
            actual: crc32(body),
        });
    }
    let doc: serde_json::Value = serde_json::from_slice(body).map_err(|_| SaveError::BadHeader)?;
    let meta: super::save_slot::SaveMeta =
        serde_json::from_value(doc["meta"].clone()).unwrap_or_default();
    let state = base64_decode(doc["state_b64"].as_str().unwrap_or(""));
    let _ = flags;
    Ok(SavePayload { meta, state })
}

/// Minimal base64 (no dep): encode/decode.
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// Decode base64 produced by [`base64_encode`].
pub fn base64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let clean: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::save_slot::SaveMeta;

    #[test]
    fn encode_decode_roundtrip() {
        let p = SavePayload {
            meta: SaveMeta {
                name: "slot1".into(),
                title_id: "T1".into(),
                ..Default::default()
            },
            state: vec![1, 2, 3, 250, 251, 252],
        };
        let bytes = encode(&p);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.meta.name, "slot1");
        assert_eq!(back.state, p.state);
    }

    #[test]
    fn detects_corruption() {
        let p = SavePayload {
            meta: SaveMeta::default(),
            state: vec![42],
        };
        let mut bytes = encode(&p);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(matches!(decode(&bytes), Err(SaveError::Checksum { .. })));
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(matches!(decode(b"NOPE"), Err(SaveError::BadHeader)));
    }

    #[test]
    fn base64_roundtrip() {
        let data: Vec<u8> = (0..=255).collect();
        let enc = base64_encode(&data);
        assert_eq!(base64_decode(&enc), data);
    }
}
