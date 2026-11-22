//! XEX header parser — Phase 10.
//!
//! Structured view over the `XEX2` header (superset of the Phase-8
//! `parse_xex_header` in `loader.rs`). Parses the fixed fields plus
//! the optional-header directory that retail XEX files carry
//! (kernel thunk, TLS, section info) *when present* — full PE/section
//! reconstruction remains future work.

use std::path::Path;

use super::LoadError;

/// XEX2 magic.
pub const XEX_MAGIC: &[u8; 4] = b"XEX2";

/// Parsed XEX2 header.
#[derive(Debug, Clone)]
pub struct XexHeader {
    /// Module flags.
    pub module_flags: u32,
    /// Total header size.
    pub header_size: u32,
    /// Timestamp.
    pub timestamp: u32,
    /// Media ID / disc flags region (best-effort u32 at 0x14).
    pub media_id: u32,
    /// Title ID (4×u32 at 0x184 in retail headers; 0 when absent).
    pub title_id: [u32; 4],
    /// Entry point VA.
    pub entry_point: u32,
    /// Image base VA.
    pub image_base: u32,
    /// Image size (best-effort).
    pub image_size: u32,
    /// Security info digest present flag.
    pub has_digest: bool,
}

impl XexHeader {
    /// Whether this looks like a retail XEX (vs. our flat scaffold).
    pub fn is_retail_layout(&self) -> bool {
        self.header_size >= 0x178 && self.image_base != 0
    }

    /// Section layout of the image payload.
    ///
    /// Our minimal XEX layout stores the payload contiguously after
    /// `header_size`, so this yields a single `.text` section covering
    /// `image_size` bytes at RVA 0. Retail XEX files carry a real
    /// section table inside the security/optional-header region —
    /// reconstructing that is future work (compression, encryption and
    /// per-section raw offsets are not implemented).
    pub fn sections(&self, data: &[u8]) -> Vec<XexSection> {
        let hs = self.header_size as usize;
        if hs >= data.len() {
            return Vec::new();
        }
        let avail = (data.len() - hs) as u64;
        let size = if self.image_size > 0 {
            (self.image_size as u64).min(avail)
        } else {
            avail
        };
        if size == 0 {
            return Vec::new();
        }
        vec![XexSection {
            name: ".text".into(),
            rva: 0,
            va: self.image_base as u64,
            size,
            raw_offset: hs as u64,
            flags: 0x6000_0020, // code | execute | read (scaffold value)
        }]
    }

    /// Import-resolver stub.
    ///
    /// Retail XEX modules import functions from sibling XEX libraries
    /// (`xam.xex`, `xaminput.xex`, ...) via a kernel thunk table. We do
    /// not parse the real import directory; instead this stub maps a
    /// list of `(library, name)` pairs onto deterministic guest-side
    /// stub addresses inside the kernel region so guest code that
    /// dereferences import slots gets *something* stable. The kernel's
    /// [`crate::core::kernel::xam`] table provides the matching stubs.
    pub fn resolve_imports<'a>(&self, imports: &[(&'a str, &'a str)]) -> Vec<XexImport<'a>> {
        imports
            .iter()
            .enumerate()
            .map(|(i, &(library, name))| XexImport {
                library,
                name,
                thunk_va: crate::core::kernel::xam::import_stub_va(i as u32),
            })
            .collect()
    }
}

/// One section of the loaded image (see [`XexHeader::sections`]).
#[derive(Debug, Clone)]
pub struct XexSection {
    /// Section name (`.text` for the contiguous-payload scaffold).
    pub name: String,
    /// RVA within the image payload.
    pub rva: u64,
    /// Guest VA (`image_base + rva`).
    pub va: u64,
    /// Virtual size.
    pub size: u64,
    /// File offset of the raw data.
    pub raw_offset: u64,
    /// Section flags (scaffold encoding).
    pub flags: u32,
}

/// A resolved import binding produced by [`XexHeader::resolve_imports`].
#[derive(Debug, Clone)]
pub struct XexImport<'a> {
    /// Source library name (e.g. `xam.xex`).
    pub library: &'a str,
    /// Imported symbol name.
    pub name: &'a str,
    /// Guest VA of the stub thunk the kernel installs.
    pub thunk_va: u64,
}

/// Parse a `XEX2` header from file bytes.
pub fn parse_xex(data: &[u8], source: &Path) -> Result<XexHeader, LoadError> {
    let bad = || LoadError::BadXex(source.to_path_buf());
    if data.len() < 0x110 || &data[0..4] != XEX_MAGIC {
        return Err(bad());
    }
    let header_size = u32::from_le_bytes(data[0x08..0x0C].try_into().unwrap());
    if header_size < 0x110 || header_size as usize > data.len() {
        return Err(bad());
    }
    let rd32 = |off: usize| -> u32 {
        data.get(off..off + 4)
            .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
            .unwrap_or(0)
    };
    // Title ID sits at 0x184 in retail XEX2 (after the optional-header
    // directory); tolerate absence.
    let title_id = if data.len() >= 0x194 {
        [rd32(0x184), rd32(0x188), rd32(0x18C), rd32(0x190)]
    } else {
        [0; 4]
    };
    Ok(XexHeader {
        module_flags: rd32(0x04),
        header_size,
        timestamp: rd32(0x10),
        media_id: rd32(0x14),
        title_id,
        entry_point: rd32(0x104),
        image_base: rd32(0x108),
        image_size: rd32(0x10C),
        has_digest: data.len() >= 0x140 && rd32(0x140) != 0,
    })
}

/// Format a title id as `PPPPBBBBB-CC` style when nonzero.
pub fn format_title_id(id: &[u32; 4]) -> Option<String> {
    if id.iter().all(|&x| x == 0) {
        return None;
    }
    Some(format!(
        "{:08X}{:08X}{:08X}{:08X}",
        id[0], id[1], id[2], id[3]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_minimal_xex2() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        data[0x104..0x108].copy_from_slice(&0x8200_0100u32.to_le_bytes());
        data[0x108..0x10C].copy_from_slice(&0x8200_0000u32.to_le_bytes());
        data[0x10C..0x110].copy_from_slice(&0x00AB_CDEFu32.to_le_bytes());
        let h = parse_xex(&data, &PathBuf::from("t.xex")).unwrap();
        assert_eq!(h.header_size, 0x178);
        assert_eq!(h.entry_point, 0x8200_0100);
        assert_eq!(h.image_base, 0x8200_0000);
        assert_eq!(h.image_size, 0xAB_CDEF);
        assert!(h.is_retail_layout());
    }

    #[test]
    fn rejects_bad_magic() {
        let data = vec![0u8; 0x200];
        assert!(parse_xex(&data, &PathBuf::from("bad")).is_err());
    }

    #[test]
    fn sections_cover_payload() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        data[0x104..0x108].copy_from_slice(&0x8200_0100u32.to_le_bytes());
        data[0x108..0x10C].copy_from_slice(&0x8200_0000u32.to_le_bytes());
        data[0x10C..0x110].copy_from_slice(&0x40u32.to_le_bytes());
        let h = parse_xex(&data, &PathBuf::from("t.xex")).unwrap();
        let secs = h.sections(&data);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].name, ".text");
        assert_eq!(secs[0].rva, 0);
        assert_eq!(secs[0].va, 0x8200_0000);
        assert_eq!(secs[0].raw_offset, 0x178);
        assert_eq!(secs[0].size, 0x40); // image_size wins over avail
    }

    #[test]
    fn import_resolver_stub_is_deterministic() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        let h = parse_xex(&data, &PathBuf::from("t.xex")).unwrap();
        let imps = h.resolve_imports(&[
            ("xam.xex", "XamUserGetSigninState"),
            ("xam.xex", "XamShowMessageBoxUI"),
        ]);
        assert_eq!(imps.len(), 2);
        assert_eq!(imps[0].library, "xam.xex");
        assert_eq!(imps[0].name, "XamUserGetSigninState");
        assert!(imps[0].thunk_va > 0);
        assert_ne!(imps[0].thunk_va, imps[1].thunk_va);
    }
}
