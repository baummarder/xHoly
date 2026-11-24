//! LEGACY: XEX2 is the Xbox 360 (PowerPC) executable format. Kept only for the --game fixtures and until the PE32+ loader (phase 24D) replaces it.
//! XEX / XEX2 header parser — Phase 10 + Phase 19.
//!
//! Two on-disk layouts are understood:
//!
//! * **Real XEX2** (Phase 19): a 0x18-byte **big-endian** fixed header
//!   (`magic`, `module_flags`, `header_size`, `reserved`,
//!   `security_offset`, `header_count`) followed by `header_count`
//!   `(key, value)` directory entries. Entry point, image base and
//!   execution info come from that directory; the section layout comes
//!   from the page descriptors in the security block. Field/key IDs
//!   follow Xenia's `xex2_info.h`.
//! * **Scaffold layout**: xHoly's own minimal little-endian fixture
//!   (`games/minimal/default.xex`) with fixed offsets at 0x104–0x10C
//!   and a contiguous payload after `header_size`. It is kept working
//!   unchanged so the Phase-17/18 end-to-end path stays green.
//!
//! Still not implemented: decompression (basic/normal LZX), AES
//! decryption of the image, the embedded PowerPC PE section table and
//! the real import directory. `XexSection::raw_offset` is therefore an
//! *uncompressed-layout estimate* for retail files — see STATUS.md.

use std::path::Path;

use super::LoadError;

/// XEX2 magic.
pub const XEX_MAGIC: &[u8; 4] = b"XEX2";

/// Size of the fixed XEX2 header before the optional-header directory.
pub const XEX2_FIXED_HEADER_SIZE: usize = 0x18;

/// Optional-header key: entry point, inline u32.
pub const XEX_HEADER_ENTRY_POINT: u32 = 0x0001_0100;
/// Optional-header key: image base address, inline u32.
pub const XEX_HEADER_IMAGE_BASE_ADDRESS: u32 = 0x0001_0201;
/// Optional-header key: original image base address, inline u32.
pub const XEX_HEADER_ORIGINAL_BASE_ADDRESS: u32 = 0x0001_0001;
/// Optional-header key: execution info structure (0x18 bytes).
pub const XEX_HEADER_EXECUTION_INFO: u32 = 0x0004_0006;
/// Optional-header key: file format info (encryption/compression).
pub const XEX_HEADER_FILE_FORMAT_INFO: u32 = 0x0003_00FF;
/// Optional-header key: checksum timestamp, inline u32.
pub const XEX_HEADER_CHECKSUM_TIMESTAMP: u32 = 0x0001_8002;
/// Optional-header key: import library table (Phase-19 stub input).
pub const XEX_HEADER_IMPORT_LIBRARIES: u32 = 0x0001_03FF;

/// Security-info flag: 4 KiB pages instead of the default 64 KiB.
pub const XEX_IMAGE_PAGE_SIZE_4KB: u32 = 0x1000_0000;

/// Page descriptor type: code.
pub const XEX_SECTION_CODE: u32 = 1;
/// Page descriptor type: initialized data.
pub const XEX_SECTION_DATA: u32 = 2;
/// Page descriptor type: read-only data.
pub const XEX_SECTION_READONLY_DATA: u32 = 3;

/// PE-style flags for a code section (execute | read).
pub const SECTION_FLAGS_CODE: u32 = 0x6000_0020;
/// PE-style flags for a writable data section.
pub const SECTION_FLAGS_DATA: u32 = 0xC000_0040;
/// PE-style flags for a read-only data section.
pub const SECTION_FLAGS_RODATA: u32 = 0x4000_0040;

/// Keys we recognize when deciding whether a header carries a real
/// XEX2 optional-header directory (guards against false positives on
/// the scaffold's little-endian fields).
const KNOWN_OPTIONAL_KEYS: &[u32] = &[
    XEX_HEADER_ENTRY_POINT,
    XEX_HEADER_IMAGE_BASE_ADDRESS,
    XEX_HEADER_ORIGINAL_BASE_ADDRESS,
    XEX_HEADER_EXECUTION_INFO,
    XEX_HEADER_FILE_FORMAT_INFO,
    XEX_HEADER_CHECKSUM_TIMESTAMP,
    XEX_HEADER_IMPORT_LIBRARIES,
    0x0000_02FF, // resource info
    0x0000_05FF, // delta patch descriptor
    0x0000_80FF, // bounding path
    0x0000_8105, // device id
    0x0001_8102, // callcap
    0x0001_8200, // fastcap
    0x0001_83FF, // original PE name
    0x0002_00FF, // static libraries
    0x0002_0104, // TLS info
    0x0002_0200, // default stack size
    0x0002_0301, // filesystem cache size
    0x0002_0401, // default heap size
    0x0003_0000, // system flags
    0x00E1_0402, // exports by name
];

/// Which on-disk layout a header was decoded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XexFormat {
    /// xHoly's minimal little-endian fixture.
    Scaffold,
    /// Real XEX2: big-endian fixed header + optional-header directory.
    Xex2,
}

/// How an optional-header value is stored, encoded in the key's low
/// byte (Xenia `GetOptHeader`): 0x00 inline, 0x01 inline data ref,
/// any other value = byte offset from the start of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XexOptValueKind {
    /// `key & 0xff == 0x00`: the value field is a u32.
    Inline,
    /// `key & 0xff == 0x01`: the value field holds inline data.
    DataRef,
    /// Other: the value field is an offset into the header.
    Offset,
}

/// One entry of the optional-header directory.
#[derive(Debug, Clone)]
pub struct XexOptHeader {
    /// Directory key (see `XEX_HEADER_*` constants).
    pub key: u32,
    /// Raw big-endian value/offset field.
    pub value: u32,
    /// How `value` must be interpreted.
    pub kind: XexOptValueKind,
}

impl XexOptHeader {
    /// Value when the key stores it inline.
    pub fn inline(&self) -> Option<u32> {
        match self.kind {
            XexOptValueKind::Inline | XexOptValueKind::DataRef => Some(self.value),
            XexOptValueKind::Offset => None,
        }
    }

    /// Offset from the start of the header when the key points at a
    /// structure.
    pub fn offset(&self) -> Option<usize> {
        match self.kind {
            XexOptValueKind::Offset => Some(self.value as usize),
            _ => None,
        }
    }
}

/// Section kind from a security page descriptor (low 4 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XexPageKind {
    /// Executable code pages.
    Code,
    /// Writable data pages.
    Data,
    /// Read-only data pages.
    ReadOnly,
    /// Reserved/unknown descriptor type.
    Unknown(u8),
}

/// One XEX2 security page descriptor.
#[derive(Debug, Clone)]
pub struct XexPageDescriptor {
    /// Page contents.
    pub kind: XexPageKind,
    /// Number of consecutive pages of this kind.
    pub page_count: u32,
}

/// Big-endian security block found at `security_offset`.
#[derive(Debug, Clone)]
pub struct XexSecurityInfo {
    /// Header size recorded in the security block.
    pub header_size: u32,
    /// Uncompressed image size.
    pub image_size: u32,
    /// Image flags (page size, title/system flags).
    pub image_flags: u32,
    /// Preferred load address.
    pub load_address: u32,
    /// Bytes per page (0x1000 or 0x10000).
    pub page_size: u32,
    /// Page descriptors describing the section layout.
    pub page_descriptors: Vec<XexPageDescriptor>,
}

/// Parsed XEX2 header.
#[derive(Debug, Clone)]
pub struct XexHeader {
    /// Module flags.
    pub module_flags: u32,
    /// Total header size (offset of the image data).
    pub header_size: u32,
    /// Timestamp.
    pub timestamp: u32,
    /// Media ID / disc flags region (best-effort u32 at 0x14).
    pub media_id: u32,
    /// Title ID (4×u32 at 0x184 in retail headers; 0 when absent).
    /// For real XEX2 this is read from the execution-info structure.
    pub title_id: [u32; 4],
    /// Entry point VA.
    pub entry_point: u32,
    /// Image base VA.
    pub image_base: u32,
    /// Image size (security block for real XEX2, fixed field otherwise).
    pub image_size: u32,
    /// Security info digest present flag.
    pub has_digest: bool,
    /// Detected on-disk layout (Phase 19).
    pub format: XexFormat,
    /// Optional-header directory (empty for the scaffold fixture).
    pub directory: Vec<XexOptHeader>,
    /// Security block with page descriptors (real XEX2 only).
    pub security: Option<XexSecurityInfo>,
    /// File offset of the security block (0 for the scaffold fixture).
    /// The image decoder reads the encrypted session key from +0x150.
    pub security_offset: u32,
}

impl XexHeader {
    /// Whether this header looks like a retail XEX layout.
    ///
    /// The original heuristic (`header_size >= 0x178 && image_base != 0`)
    /// is retained for compatibility; a parsed optional-header directory
    /// is now also sufficient (Phase 19) so retail headers that do not
    /// match the old fixed-offset heuristic are no longer ignored.
    pub fn is_retail_layout(&self) -> bool {
        (self.header_size >= 0x178 && self.image_base != 0) || self.has_optional_directory()
    }

    /// Whether a real optional-header directory was decoded.
    pub fn has_optional_directory(&self) -> bool {
        !self.directory.is_empty()
    }

    /// Look up an optional-header entry by key.
    pub fn opt(&self, key: u32) -> Option<&XexOptHeader> {
        self.directory.iter().find(|e| e.key == key)
    }

    /// Look up an inline optional-header value by key.
    pub fn opt_inline(&self, key: u32) -> Option<u32> {
        self.opt(key).and_then(|e| e.inline())
    }

    /// Look up an optional-header structure offset by key.
    pub fn opt_offset(&self, key: u32) -> Option<usize> {
        self.opt(key).and_then(|e| e.offset())
    }

    /// Section layout of the image payload.
    ///
    /// For real XEX2 files this is reconstructed from the security
    /// block's page descriptors: one section per descriptor, named
    /// `.text` / `.data` / `.rdata` (with a numeric suffix for repeats).
    /// RVA = page index × page size, VA = image base + RVA, and
    /// `raw_offset` = header size + RVA (only exact for uncompressed,
    /// unencrypted images — compression/encryption is still future
    /// work). When no descriptors are available the old contiguous
    /// `.text` view over the payload is returned.
    pub fn sections(&self, data: &[u8]) -> Vec<XexSection> {
        if let Some(sec) = &self.security {
            if !sec.page_descriptors.is_empty() {
                let mut out = Vec::with_capacity(sec.page_descriptors.len());
                let mut page = 0u32;
                let mut code_n = 0usize;
                let mut data_n = 0usize;
                let mut ro_n = 0usize;
                for desc in &sec.page_descriptors {
                    let rva = page as u64 * sec.page_size as u64;
                    let size = desc.page_count as u64 * sec.page_size as u64;
                    let (base, flags, n) = match desc.kind {
                        XexPageKind::Code => {
                            code_n += 1;
                            (".text", SECTION_FLAGS_CODE, code_n)
                        }
                        XexPageKind::Data => {
                            data_n += 1;
                            (".data", SECTION_FLAGS_DATA, data_n)
                        }
                        XexPageKind::ReadOnly => {
                            ro_n += 1;
                            (".rdata", SECTION_FLAGS_RODATA, ro_n)
                        }
                        XexPageKind::Unknown(_) => {
                            data_n += 1;
                            (".data", SECTION_FLAGS_DATA, data_n)
                        }
                    };
                    out.push(XexSection {
                        name: if n == 1 {
                            base.to_string()
                        } else {
                            format!("{base}{}", n - 1)
                        },
                        rva,
                        va: self.image_base as u64 + rva,
                        size,
                        raw_offset: self.header_size as u64 + rva,
                        flags,
                    });
                    page = page.saturating_add(desc.page_count);
                }
                return out;
            }
        }

        // Fallback: scaffold / directory-only XEX, contiguous payload.
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
            flags: SECTION_FLAGS_CODE,
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
    /// Section name (`.text`, `.data`, `.rdata` for real XEX2).
    pub name: String,
    /// RVA within the image payload.
    pub rva: u64,
    /// Guest VA (`image_base + rva`).
    pub va: u64,
    /// Virtual size.
    pub size: u64,
    /// Estimated file offset of the raw data (uncompressed images).
    pub raw_offset: u64,
    /// Section flags (PE-style encoding).
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

/// Read a big-endian u32 at `off`, returning `None` when out of bounds.
fn get_be32(data: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    let s = data.get(off..end)?;
    Some(u32::from_be_bytes(s.try_into().ok()?))
}

/// Decode the fixed header + optional-header directory of a real XEX2.
///
/// Returns `(header_size, security_offset, directory)` when the fields
/// are self-consistent and at least one known key is present. The
/// little-endian scaffold fixture byte-swaps `header_size` and is
/// therefore rejected here.
fn parse_directory(data: &[u8]) -> Option<(u32, u32, Vec<XexOptHeader>)> {
    if data.len() < XEX2_FIXED_HEADER_SIZE {
        return None;
    }
    let header_size = get_be32(data, 0x08)?;
    let security_offset = get_be32(data, 0x10)?;
    let count = get_be32(data, 0x14)?;
    if header_size < XEX2_FIXED_HEADER_SIZE as u32 || header_size as usize > data.len() {
        return None;
    }
    if count == 0 || count > 256 {
        return None;
    }
    let end = XEX2_FIXED_HEADER_SIZE.checked_add(count as usize * 8)?;
    if end > header_size as usize || end > data.len() {
        return None;
    }
    let mut directory = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let off = XEX2_FIXED_HEADER_SIZE + i * 8;
        let key = get_be32(data, off)?;
        let value = get_be32(data, off + 4)?;
        if key == 0 {
            continue; // padding / end marker
        }
        let kind = match key & 0xff {
            0x00 => XexOptValueKind::Inline,
            0x01 => XexOptValueKind::DataRef,
            _ => XexOptValueKind::Offset,
        };
        directory.push(XexOptHeader { key, value, kind });
    }
    if !directory
        .iter()
        .any(|e| KNOWN_OPTIONAL_KEYS.contains(&e.key))
    {
        return None;
    }
    Some((header_size, security_offset, directory))
}

/// Decode the security block's page descriptors at `off`.
///
/// Layout per Xenia `xex2_security_info`: image_size at +0x4,
/// image_flags at +0x10C, load_address at +0x110,
/// page_descriptor_count at +0x180, descriptors at +0x184 (0x18 each:
/// 4-byte value + 0x14-byte digest). The value's low nibble is the
/// section type, the high 28 bits the page count.
fn parse_security(data: &[u8], off: u32) -> Option<XexSecurityInfo> {
    let base = off as usize;
    let header_size = get_be32(data, base)?;
    let image_size = get_be32(data, base + 0x04)?;
    let image_flags = get_be32(data, base + 0x10C)?;
    let load_address = get_be32(data, base + 0x110)?;
    let count = get_be32(data, base + 0x180)?;
    if count > 4096 {
        return None;
    }
    let descriptors_end = base
        .checked_add(0x184)?
        .checked_add(count as usize * 0x18)?;
    if descriptors_end > data.len() {
        return None;
    }
    let page_size = if image_flags & XEX_IMAGE_PAGE_SIZE_4KB != 0 || load_address > 0x9000_0000 {
        0x1000
    } else {
        0x10000
    };
    let mut page_descriptors = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let raw = get_be32(data, base + 0x184 + i * 0x18)?;
        let kind = match raw & 0xf {
            XEX_SECTION_CODE => XexPageKind::Code,
            XEX_SECTION_DATA => XexPageKind::Data,
            XEX_SECTION_READONLY_DATA => XexPageKind::ReadOnly,
            other => XexPageKind::Unknown(other as u8),
        };
        page_descriptors.push(XexPageDescriptor {
            kind,
            page_count: raw >> 4,
        });
    }
    Some(XexSecurityInfo {
        header_size,
        image_size,
        image_flags,
        load_address,
        page_size,
        page_descriptors,
    })
}

/// Parse a `XEX2` header from file bytes.
pub fn parse_xex(data: &[u8], source: &Path) -> Result<XexHeader, LoadError> {
    let bad = || LoadError::BadXex(source.to_path_buf());
    if data.len() < 4 || &data[0..4] != XEX_MAGIC {
        return Err(bad());
    }

    // Phase 19: real XEX2 fixed header + optional-header directory.
    if let Some((header_size, security_offset, directory)) = parse_directory(data) {
        let be = |off: usize| get_be32(data, off).unwrap_or(0);
        let security = parse_security(data, security_offset);
        let exec_off = directory
            .iter()
            .find(|e| e.key == XEX_HEADER_EXECUTION_INFO)
            .and_then(|e| e.offset());
        let title_id = if let Some(off) = exec_off {
            [get_be32(data, off + 0x0C).unwrap_or(0), 0, 0, 0]
        } else {
            [0; 4]
        };
        let media_id = exec_off.and_then(|off| get_be32(data, off)).unwrap_or(0);
        let entry_point = directory
            .iter()
            .find(|e| e.key == XEX_HEADER_ENTRY_POINT)
            .and_then(|e| e.inline())
            .unwrap_or_else(|| be(0x104));
        let image_base = directory
            .iter()
            .find(|e| e.key == XEX_HEADER_IMAGE_BASE_ADDRESS)
            .or_else(|| {
                directory
                    .iter()
                    .find(|e| e.key == XEX_HEADER_ORIGINAL_BASE_ADDRESS)
            })
            .and_then(|e| e.inline())
            .or_else(|| security.as_ref().map(|s| s.load_address))
            .unwrap_or(0);
        let timestamp = directory
            .iter()
            .find(|e| e.key == XEX_HEADER_CHECKSUM_TIMESTAMP)
            .and_then(|e| e.inline())
            .unwrap_or(0);
        let image_size = security
            .as_ref()
            .map(|s| s.image_size)
            .unwrap_or_else(|| be(0x10C));

        return Ok(XexHeader {
            module_flags: be(0x04),
            header_size,
            timestamp,
            media_id,
            title_id,
            entry_point,
            image_base,
            image_size,
            has_digest: security.is_some(),
            format: XexFormat::Xex2,
            directory,
            security,
            security_offset,
        });
    }

    // Scaffold fixture: minimal little-endian header (unchanged).
    if data.len() < 0x110 {
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
    let header_size = rd32(0x08);
    if header_size < 0x110 || header_size as usize > data.len() {
        return Err(bad());
    }
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
        format: XexFormat::Scaffold,
        directory: Vec::new(),
        security: None,
        security_offset: 0,
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

    fn path() -> PathBuf {
        PathBuf::from("t.xex")
    }

    /// Build a synthetic but structurally real XEX2 (big-endian fixed
    /// header, optional directory, execution info, security block with
    /// three page descriptors). Phase 19 test fixture.
    fn retail_xex2() -> Vec<u8> {
        let mut data = vec![0u8; 0x500];
        let put = |d: &mut [u8], off: usize, v: u32| {
            d[off..off + 4].copy_from_slice(&v.to_be_bytes());
        };
        data[0..4].copy_from_slice(b"XEX2");
        put(&mut data, 0x04, 0x0000_0001); // module flags: title
        put(&mut data, 0x08, 0x400); // header size (PE data offset)
        put(&mut data, 0x0C, 0); // reserved
        put(&mut data, 0x10, 0x200); // security offset
        put(&mut data, 0x14, 4); // optional header count

        // Directory at 0x18: (key, value) pairs, big-endian.
        put(&mut data, 0x18, XEX_HEADER_ENTRY_POINT); // key & 0xff == 0
        put(&mut data, 0x1C, 0x8200_0100); // inline entry point
        put(&mut data, 0x20, XEX_HEADER_IMAGE_BASE_ADDRESS); // key & 0xff == 1
        put(&mut data, 0x24, 0x8200_0000); // inline image base
        put(&mut data, 0x28, XEX_HEADER_EXECUTION_INFO); // offset key
        put(&mut data, 0x2C, 0x100); // struct at header+0x100
        put(&mut data, 0x30, XEX_HEADER_FILE_FORMAT_INFO); // offset key
        put(&mut data, 0x34, 0x120);

        // Execution info (0x18 bytes): title id at +0xC.
        put(&mut data, 0x100, 0xDEAD_BEEF); // media id
        put(&mut data, 0x10C, 0x1234_5678); // title id
                                            // File format info: no encryption, no compression.
        put(&mut data, 0x120, 0x08); // info size
        put(&mut data, 0x124, 0); // encryption + compression types

        // Security block at 0x200.
        put(&mut data, 0x200, 0x184); // security header size
        put(&mut data, 0x204, 0x0002_0000); // image size
        put(&mut data, 0x30C, 0); // image flags -> 64 KiB pages
        put(&mut data, 0x310, 0x8200_0000); // load address
        put(&mut data, 0x380, 3); // page descriptor count
        put(&mut data, 0x384, XEX_SECTION_CODE | (1 << 4)); // 1 code page
        put(&mut data, 0x39C, XEX_SECTION_DATA | (1 << 4)); // 1 data page
        put(&mut data, 0x3B4, XEX_SECTION_READONLY_DATA | (1 << 4)); // 1 RO page

        // Payload after header_size.
        data[0x400..0x404].copy_from_slice(&0xD503_201Fu32.to_le_bytes()); // NOP
        data
    }

    #[test]
    fn parses_minimal_xex2() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        data[0x104..0x108].copy_from_slice(&0x8200_0100u32.to_le_bytes());
        data[0x108..0x10C].copy_from_slice(&0x8200_0000u32.to_le_bytes());
        data[0x10C..0x110].copy_from_slice(&0x00AB_CDEFu32.to_le_bytes());
        let h = parse_xex(&data, &path()).unwrap();
        assert_eq!(h.header_size, 0x178);
        assert_eq!(h.entry_point, 0x8200_0100);
        assert_eq!(h.image_base, 0x8200_0000);
        assert_eq!(h.image_size, 0xAB_CDEF);
        assert_eq!(h.format, XexFormat::Scaffold);
        assert!(!h.has_optional_directory());
        assert!(h.is_retail_layout()); // retained heuristic
    }

    #[test]
    fn rejects_bad_magic() {
        let data = vec![0u8; 0x200];
        assert!(parse_xex(&data, &path()).is_err());
    }

    #[test]
    fn sections_cover_payload() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        data[0x104..0x108].copy_from_slice(&0x8200_0100u32.to_le_bytes());
        data[0x108..0x10C].copy_from_slice(&0x8200_0000u32.to_le_bytes());
        data[0x10C..0x110].copy_from_slice(&0x40u32.to_le_bytes());
        let h = parse_xex(&data, &path()).unwrap();
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
        let h = parse_xex(&data, &path()).unwrap();
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

    #[test]
    fn parses_retail_directory_and_ignores_no_keys() {
        let data = retail_xex2();
        let h = parse_xex(&data, &path()).unwrap();
        assert_eq!(h.format, XexFormat::Xex2);
        assert!(h.has_optional_directory());
        assert!(h.is_retail_layout());
        assert_eq!(h.header_size, 0x400);
        assert_eq!(h.entry_point, 0x8200_0100);
        assert_eq!(h.image_base, 0x8200_0000);
        assert_eq!(h.image_size, 0x0002_0000);
        assert_eq!(h.title_id[0], 0x1234_5678);
        assert_eq!(h.directory.len(), 4);
        assert_eq!(h.opt_inline(XEX_HEADER_ENTRY_POINT), Some(0x8200_0100));
        assert_eq!(
            h.opt_inline(XEX_HEADER_IMAGE_BASE_ADDRESS),
            Some(0x8200_0000)
        );
        assert_eq!(h.opt_offset(XEX_HEADER_EXECUTION_INFO), Some(0x100));
        assert!(h.has_digest);
    }

    #[test]
    fn directory_encodes_value_storage_in_low_key_byte() {
        let data = retail_xex2();
        let h = parse_xex(&data, &path()).unwrap();
        assert_eq!(
            h.opt(XEX_HEADER_ENTRY_POINT).unwrap().kind,
            XexOptValueKind::Inline
        );
        assert_eq!(
            h.opt(XEX_HEADER_IMAGE_BASE_ADDRESS).unwrap().kind,
            XexOptValueKind::DataRef
        );
        assert_eq!(
            h.opt(XEX_HEADER_EXECUTION_INFO).unwrap().kind,
            XexOptValueKind::Offset
        );
    }

    #[test]
    fn retail_sections_come_from_page_descriptors() {
        let data = retail_xex2();
        let h = parse_xex(&data, &path()).unwrap();
        let sec = h.security.as_ref().unwrap();
        assert_eq!(sec.page_size, 0x10000); // image_flags bit not set
        assert_eq!(sec.page_descriptors.len(), 3);
        let secs = h.sections(&data);
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].name, ".text");
        assert_eq!(secs[1].name, ".data");
        assert_eq!(secs[2].name, ".rdata");
        assert_eq!(secs[0].rva, 0);
        assert_eq!(secs[1].rva, 0x10000);
        assert_eq!(secs[2].rva, 0x20000);
        assert_eq!(secs[0].va, 0x8200_0000);
        assert_eq!(secs[2].va, 0x8202_0000);
        assert_eq!(secs[0].size, 0x10000);
        assert_eq!(secs[1].raw_offset, 0x400 + 0x10000);
        assert_eq!(secs[0].flags, SECTION_FLAGS_CODE);
        assert_eq!(secs[1].flags, SECTION_FLAGS_DATA);
        assert_eq!(secs[2].flags, SECTION_FLAGS_RODATA);
    }

    #[test]
    fn retail_sections_survive_missing_payload() {
        // Compressed/encrypted images have no contiguous payload at
        // header_size — page descriptors must still describe the layout.
        let mut data = retail_xex2();
        data.truncate(0x400);
        let h = parse_xex(&data, &path()).unwrap();
        assert_eq!(h.sections(&data).len(), 3);
    }

    #[test]
    fn four_kib_pages_are_honored() {
        let mut data = retail_xex2();
        data[0x30C..0x310].copy_from_slice(&XEX_IMAGE_PAGE_SIZE_4KB.to_be_bytes());
        let h = parse_xex(&data, &path()).unwrap();
        assert_eq!(h.security.as_ref().unwrap().page_size, 0x1000);
        assert_eq!(h.sections(&data)[1].rva, 0x1000);
    }
}
