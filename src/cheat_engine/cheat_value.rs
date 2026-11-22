//! Cheat values — what a cheat writes where.

/// A guest memory region a cheat can target (simulated addresses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegion {
    /// Main guest RAM.
    Ram,
    /// VRAM / framebuffer region.
    Vram,
    /// Kernel space.
    Kernel,
}

/// A cheat's payload value.
#[derive(Debug, Clone, PartialEq)]
pub enum CheatValue {
    /// Single byte.
    U8(u8),
    /// 16-bit.
    U16(u16),
    /// 32-bit (most common: health, ammo, money).
    U32(u32),
    /// 64-bit.
    U64(u64),
    /// Float.
    F32(f32),
    /// Byte array (struct patch).
    Bytes(Vec<u8>),
}

impl CheatValue {
    /// Serialized little-endian bytes for patching.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::U8(v) => vec![*v],
            Self::U16(v) => v.to_le_bytes().to_vec(),
            Self::U32(v) => v.to_le_bytes().to_vec(),
            Self::U64(v) => v.to_le_bytes().to_vec(),
            Self::F32(v) => v.to_bits().to_le_bytes().to_vec(),
            Self::Bytes(b) => b.clone(),
        }
    }

    /// Size in bytes when patched.
    pub fn len(&self) -> usize {
        self.to_bytes().len()
    }

    /// Empty payload?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Parse "0x..." or decimal into U32/U64 by width.
    pub fn parse_int(s: &str) -> Option<Self> {
        let v = if let Some(h) = s.strip_prefix("0x") {
            u64::from_str_radix(h, 16).ok()?
        } else {
            s.parse().ok()?
        };
        Some(if v <= u32::MAX as u64 {
            Self::U32(v as u32)
        } else {
            Self::U64(v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_bytes_roundtrip() {
        assert_eq!(
            CheatValue::U32(0xDEAD_BEEF).to_bytes(),
            [0xEF, 0xBE, 0xAD, 0xDE]
        );
        assert_eq!(CheatValue::U8(7).to_bytes(), [7]);
        assert_eq!(CheatValue::F32(1.5).len(), 4);
    }

    #[test]
    fn parse_int_hex() {
        assert_eq!(CheatValue::parse_int("0x10"), Some(CheatValue::U32(16)));
        assert_eq!(
            CheatValue::parse_int("4294967296"),
            Some(CheatValue::U64(4294967296))
        );
        assert_eq!(CheatValue::parse_int("xyz"), None);
    }
}
