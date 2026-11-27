//! PE32+ (PE64) loader for Xbox One / Xbox Series X|S executables — Phase 25.
//!
//! This is a real parser for the format documented by ScarlettX's README
//! (PE32+ mapping, DIR64/HIGHLOW relocations, named/ordinal IAT, API-set
//! redirection). No ScarlettX source code was copied — the local reference
//! has no LICENSE file, so only format behaviour is reimplemented here.
//!
//! A normal Windows `.exe` and an Xbox GDK game both use PE32+; the
//! distinguishing evidence is the package manifest (`MicrosoftGame.Config`)
//! and the imported Xbox runtime DLLs (`d3d12_xs.dll`, `xgameruntime.dll`,
//! `xmem.dll`, ...), not the file extension alone.

use std::fmt;

/// Architecture constants.
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
pub const PE32PLUS_MAGIC: u16 = 0x20B;

/// Base relocation types used by x86-64 PE images.
pub const RELOC_ABSOLUTE: u16 = 0;
pub const RELOC_HIGH: u16 = 1;
pub const RELOC_LOW: u16 = 2;
pub const RELOC_HIGHLOW: u16 = 3;
pub const RELOC_DIR64: u16 = 10;

/// A parsed PE32+ section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Name (up to 8 bytes).
    pub name: String,
    /// Virtual size.
    pub virtual_size: u32,
    /// RVA of the section.
    pub virtual_address: u32,
    /// Size of raw data in the file.
    pub raw_size: u32,
    /// File offset of raw data.
    pub raw_pointer: u32,
    /// Section characteristics.
    pub characteristics: u32,
}

/// One imported symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// Import by ordinal (high bit set in the thunk).
    ByOrdinal(u16),
    /// Import by name (RVA into the hint/name table).
    ByName(u32),
}

/// One imported DLL descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDescriptor {
    /// DLL name (e.g. `d3d12_xs.dll`).
    pub dll: String,
    /// Symbols imported from the DLL.
    pub symbols: Vec<Import>,
    /// RVA of the first thunk (IAT), used when applying bindings.
    pub first_thunk: u32,
}

/// One base relocation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relocation {
    /// Relocation type (`DIR64`, `HIGHLOW`, ...).
    pub kind: u16,
    /// RVA of the patch site.
    pub rva: u32,
}

/// A parsed PE32+ image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pe64Image {
    /// Machine field (expected `IMAGE_FILE_MACHINE_AMD64`).
    pub machine: u16,
    /// Preferred image base VA.
    pub image_base: u64,
    /// Entry-point RVA.
    pub entry_rva: u32,
    /// SizeOfImage.
    pub size_of_image: u32,
    /// SizeOfHeaders.
    pub size_of_headers: u32,
    /// Subsystem (2 = GUI).
    pub subsystem: u16,
    /// Sections.
    pub sections: Vec<Section>,
    /// Imports (empty when the file has no import directory).
    pub imports: Vec<ImportDescriptor>,
    /// Base relocations.
    pub relocations: Vec<Relocation>,
    /// Raw file bytes (kept for mapping).
    pub bytes: Vec<u8>,
}

impl Pe64Image {
    /// Absolute VA of the entry point.
    pub fn entry_va(&self) -> u64 {
        self.image_base.wrapping_add(self.entry_rva as u64)
    }

    /// Section containing `rva`, if any.
    pub fn section_at(&self, rva: u32) -> Option<&Section> {
        self.sections.iter().find(|s| {
            let span = s.virtual_size.max(s.raw_size);
            rva >= s.virtual_address && (rva as u64) < s.virtual_address as u64 + span as u64
        })
    }

    /// Read bytes at `rva` from the raw file image.
    pub fn read_rva(&self, rva: u32, len: usize) -> Option<&[u8]> {
        let section = self.section_at(rva)?;
        let offset = section.raw_pointer as usize + (rva - section.virtual_address) as usize;
        self.bytes.get(offset..offset + len)
    }

    /// Read a NUL-terminated string at `rva`.
    pub fn string_at(&self, rva: u32) -> Option<String> {
        let section = self.section_at(rva)?;
        let start = section.raw_pointer as usize + (rva - section.virtual_address) as usize;
        let tail = self.bytes.get(start..)?;
        let end = tail.iter().position(|b| *b == 0)? + start;
        Some(String::from_utf8_lossy(self.bytes.get(start..end)?).into_owned())
    }

    /// Resolve an import hint/name table entry to its symbol name.
    /// The first two bytes are the hint and are skipped.
    pub fn import_name(&self, rva: u32) -> Option<String> {
        let section = self.section_at(rva)?;
        let start = section.raw_pointer as usize + (rva - section.virtual_address) as usize + 2;
        let end = self.bytes.get(start..)?.iter().position(|b| *b == 0)? + start;
        Some(String::from_utf8_lossy(&self.bytes[start..end]).into_owned())
    }

    /// Whether the imports suggest an Xbox GDK title rather than a normal
    /// Windows application (evidence-based, never inferred from `.exe`).
    pub fn looks_like_xbox_gdk(&self) -> bool {
        const XBOX_DLLS: &[&str] = &[
            "d3d12_xs.dll",
            "xgameruntime.dll",
            "xmem.dll",
            "xg_xs.dll",
            "dstorage_xs.dll",
            "xaudio2_xs.dll",
            "GameInput.dll",
        ];
        self.imports.iter().any(|d| {
            let lower = d.dll.to_ascii_lowercase();
            XBOX_DLLS.iter().any(|x| lower == *x)
        })
    }
}

/// Loader failure — no panics on malformed input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pe64Error {
    /// Too short or bad DOS signature.
    NotPe,
    /// Bad PE signature / unsupported machine / not PE32+.
    Unsupported(&'static str),
    /// A structure was truncated or inconsistent.
    Malformed(&'static str),
}

impl fmt::Display for Pe64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPe => write!(f, "not a PE image"),
            Self::Unsupported(s) => write!(f, "unsupported PE64 image: {s}"),
            Self::Malformed(s) => write!(f, "malformed PE64 image: {s}"),
        }
    }
}
impl std::error::Error for Pe64Error {}

fn u16at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn u32at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn u64at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

/// Parse a PE32+ image from raw bytes.
pub fn parse(data: &[u8]) -> Result<Pe64Image, Pe64Error> {
    if data.len() < 0x40 || &data[0..2] != b"MZ" {
        return Err(Pe64Error::NotPe);
    }
    let pe = u32at(data, 0x3C).ok_or(Pe64Error::Malformed("e_lfanew"))? as usize;
    if data.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(Pe64Error::NotPe);
    }
    let coff = pe + 4;
    let machine = u16at(data, coff).ok_or(Pe64Error::Malformed("machine"))?;
    if machine != IMAGE_FILE_MACHINE_AMD64 {
        return Err(Pe64Error::Unsupported("machine is not x86-64"));
    }
    let section_count = u16at(data, coff + 2).ok_or(Pe64Error::Malformed("sections"))? as usize;
    let optional_size =
        u16at(data, coff + 16).ok_or(Pe64Error::Malformed("optional size"))? as usize;
    let optional = coff + 20;
    if u16at(data, optional) != Some(PE32PLUS_MAGIC) {
        return Err(Pe64Error::Unsupported("not PE32+"));
    }
    let entry_rva = u32at(data, optional + 16).ok_or(Pe64Error::Malformed("entry"))?;
    let image_base = u64at(data, optional + 24).ok_or(Pe64Error::Malformed("image base"))?;
    let size_of_image = u32at(data, optional + 56).ok_or(Pe64Error::Malformed("SizeOfImage"))?;
    let size_of_headers =
        u32at(data, optional + 60).ok_or(Pe64Error::Malformed("SizeOfHeaders"))?;
    let subsystem = u16at(data, optional + 68).unwrap_or(0);
    let dir_count = u32at(data, optional + 108).unwrap_or(0) as usize;
    if optional_size < 112 || optional + optional_size > data.len() {
        return Err(Pe64Error::Malformed("optional header"));
    }

    let mut sections = Vec::with_capacity(section_count);
    let section_table = optional + optional_size;
    for i in 0..section_count {
        let o = section_table + i * 40;
        if o + 40 > data.len() {
            return Err(Pe64Error::Malformed("section table"));
        }
        let name = String::from_utf8_lossy(&data[o..o + 8])
            .trim_matches('\0')
            .to_string();
        sections.push(Section {
            name,
            virtual_size: u32at(data, o + 8).unwrap_or(0),
            virtual_address: u32at(data, o + 12).unwrap_or(0),
            raw_size: u32at(data, o + 16).unwrap_or(0),
            raw_pointer: u32at(data, o + 20).unwrap_or(0),
            characteristics: u32at(data, o + 36).unwrap_or(0),
        });
    }

    let dir = |index: usize| -> Option<(u32, u32)> {
        if index >= dir_count || 112 + index * 8 + 8 > optional_size {
            return None;
        }
        let o = optional + 112 + index * 8;
        Some((u32at(data, o)?, u32at(data, o + 4)?))
    };

    // Import directory (index 1).
    let mut imports = Vec::new();
    if let Some((import_rva, _)) = dir(1) {
        if import_rva != 0 {
            let mut cursor = import_rva;
            for _ in 0..256 {
                let fields = [
                    u32at(
                        data,
                        file_offset(&sections, cursor).ok_or(Pe64Error::Malformed("rva"))?,
                    ),
                    u32at(
                        data,
                        file_offset(&sections, cursor + 4).ok_or(Pe64Error::Malformed("rva"))?,
                    ),
                    u32at(
                        data,
                        file_offset(&sections, cursor + 8).ok_or(Pe64Error::Malformed("rva"))?,
                    ),
                    u32at(
                        data,
                        file_offset(&sections, cursor + 12).ok_or(Pe64Error::Malformed("rva"))?,
                    ),
                    u32at(
                        data,
                        file_offset(&sections, cursor + 16).ok_or(Pe64Error::Malformed("rva"))?,
                    ),
                ];
                let f: Vec<u32> = fields.into_iter().flatten().collect();
                if f.len() != 5 || f.iter().all(|v| *v == 0) {
                    break;
                }
                let dll = data
                    .get(file_offset(&sections, f[3]).ok_or(Pe64Error::Malformed("rva"))?)
                    .and_then(|_| {
                        let start = file_offset(&sections, f[3])?;
                        let end = data[start..].iter().position(|b| *b == 0)? + start;
                        Some(String::from_utf8_lossy(&data[start..end]).into_owned())
                    })
                    .unwrap_or_default();
                let thunk_rva = if f[4] != 0 { f[4] } else { f[0] };
                let mut symbols = Vec::new();
                let mut t = thunk_rva;
                for _ in 0..4096 {
                    let off = file_offset(&sections, t).ok_or(Pe64Error::Malformed("rva"))?;
                    let value = u64at(data, off).ok_or(Pe64Error::Malformed("IAT"))?;
                    if value == 0 {
                        break;
                    }
                    if value & (1u64 << 63) != 0 {
                        symbols.push(Import::ByOrdinal((value & 0xFFFF) as u16));
                    } else {
                        symbols.push(Import::ByName(value as u32));
                    }
                    t = t
                        .checked_add(8)
                        .ok_or(Pe64Error::Malformed("IAT overflow"))?;
                }
                imports.push(ImportDescriptor {
                    dll,
                    symbols,
                    first_thunk: f[4],
                });
                cursor = cursor
                    .checked_add(20)
                    .ok_or(Pe64Error::Malformed("import overflow"))?;
            }
        }
    }

    // Base relocation directory (index 5).
    let mut relocations = Vec::new();
    if let Some((reloc_rva, reloc_size)) = dir(5) {
        if reloc_rva != 0 && reloc_size != 0 {
            let mut block = reloc_rva;
            let end = reloc_rva.saturating_add(reloc_size);
            while block < end {
                let page = u32at(
                    data,
                    file_offset(&sections, block).ok_or(Pe64Error::Malformed("rva"))?,
                )
                .ok_or(Pe64Error::Malformed("reloc block"))?;
                let block_size = u32at(
                    data,
                    file_offset(&sections, block + 4).ok_or(Pe64Error::Malformed("rva"))?,
                )
                .ok_or(Pe64Error::Malformed("reloc size"))?;
                if block_size < 8 {
                    break;
                }
                let count = (block_size - 8) / 2;
                for i in 0..count {
                    let entry = u16at(
                        data,
                        file_offset(&sections, block + 8 + i * 2)
                            .ok_or(Pe64Error::Malformed("rva"))?,
                    )
                    .ok_or(Pe64Error::Malformed("reloc entry"))?;
                    let kind = entry >> 12;
                    let offset = entry & 0x0FFF;
                    if kind != RELOC_ABSOLUTE {
                        relocations.push(Relocation {
                            kind,
                            rva: page + offset as u32,
                        });
                    }
                }
                block = block.saturating_add(block_size);
            }
        }
    }

    Ok(Pe64Image {
        machine,
        image_base,
        entry_rva,
        size_of_image,
        size_of_headers,
        subsystem,
        sections,
        imports,
        relocations,
        bytes: data.to_vec(),
    })
}

/// Convert an RVA to a file offset using the section table.
pub fn file_offset(sections: &[Section], rva: u32) -> Option<usize> {
    if rva < 0x1000 {
        // Headers are mapped at RVA 0.
        return Some(rva as usize);
    }
    sections.iter().find_map(|s| {
        if rva >= s.virtual_address && (rva as u64) < s.virtual_address as u64 + s.raw_size as u64 {
            Some(s.raw_pointer as usize + (rva - s.virtual_address) as usize)
        } else {
            None
        }
    })
}

/// Structural validation of a parsed PE32+ image.
///
/// [`parse`] already checks the DOS/COFF/optional headers, the machine
/// type and the PE32+ magic. This function additionally checks the
/// *layout* the loader is about to trust:
///
/// * `SizeOfImage`/`SizeOfHeaders` are consistent with the file,
/// * every section fits inside `SizeOfImage` and inside the file,
/// * no section overlaps the headers or a previous section,
/// * the entry point lies inside the image.
///
/// It is deliberately not called from [`parse`]: read-only import
/// analysis must keep working on any parseable file. The guest loading
/// path calls it before mapping anything into guest memory.
pub fn validate(image: &Pe64Image) -> Result<(), Pe64Error> {
    if image.size_of_image == 0 {
        return Err(Pe64Error::Malformed("SizeOfImage is zero"));
    }
    if image.size_of_headers == 0 || image.size_of_headers > image.size_of_image {
        return Err(Pe64Error::Malformed("SizeOfHeaders exceeds SizeOfImage"));
    }
    if image.size_of_headers as usize > image.bytes.len() {
        return Err(Pe64Error::Malformed("SizeOfHeaders exceeds file size"));
    }
    if image.entry_rva != 0 && image.entry_rva >= image.size_of_image {
        return Err(Pe64Error::Malformed("entry point outside the image"));
    }
    // End of the mapped header region in RVA space.
    let mut prev_end = u64::from(image.size_of_headers);
    for section in &image.sections {
        let span = u64::from(section.virtual_size.max(section.raw_size));
        let start = u64::from(section.virtual_address);
        let end = start
            .checked_add(span)
            .ok_or(Pe64Error::Malformed("section range overflows"))?;
        if end > u64::from(image.size_of_image) {
            return Err(Pe64Error::Malformed("section exceeds SizeOfImage"));
        }
        if section.raw_size != 0 {
            let raw_end = u64::from(section.raw_pointer).checked_add(u64::from(section.raw_size));
            match raw_end {
                Some(raw_end) if raw_end > image.bytes.len() as u64 => {
                    return Err(Pe64Error::Malformed("section raw data exceeds file size"));
                }
                Some(_) => {}
                None => return Err(Pe64Error::Malformed("section raw range overflows")),
            }
            if u64::from(section.raw_pointer) < u64::from(image.size_of_headers) {
                return Err(Pe64Error::Malformed("section raw data overlaps headers"));
            }
        }
        if start < prev_end {
            return Err(Pe64Error::Malformed("section overlaps a previous section"));
        }
        prev_end = prev_end.max(end);
    }
    // M35: the entry point must lie in a section marked executable, not
    // merely anywhere inside SizeOfImage (a zero-filled gap would pass).
    if image.entry_rva != 0 {
        let executable = image.sections.iter().any(|section| {
            let start = u64::from(section.virtual_address);
            let span = u64::from(section.virtual_size.max(section.raw_size));
            let in_section = u64::from(image.entry_rva) >= start
                && u64::from(image.entry_rva) < start.saturating_add(span);
            // IMAGE_SCN_MEM_EXECUTE (0x2000_0000) is required; CNT_CODE
            // alone does not make a section executable.
            in_section && section.characteristics & 0x2000_0000 != 0
        });
        if !executable {
            return Err(Pe64Error::Malformed(
                "entry point is not in an executable section",
            ));
        }
    }
    // Relocation sites must lie inside a mapped section.
    for reloc in &image.relocations {
        if image.section_at(reloc.rva).is_none() {
            return Err(Pe64Error::Malformed("relocation site outside a section"));
        }
    }
    Ok(())
}

/// Map headers and sections into a flat guest image of `SizeOfImage` bytes.
pub fn map_image(image: &Pe64Image) -> Result<Vec<u8>, Pe64Error> {
    let mut out = vec![0u8; image.size_of_image as usize];
    let headers = image.size_of_headers as usize;
    if headers > out.len() {
        return Err(Pe64Error::Malformed("SizeOfHeaders"));
    }
    out[..headers].copy_from_slice(&image.bytes[..headers.min(image.bytes.len())]);
    for section in &image.sections {
        let dst = section.virtual_address as usize;
        let src = section.raw_pointer as usize;
        let len = section.raw_size as usize;
        if dst + len > out.len() || src + len > image.bytes.len() {
            return Err(Pe64Error::Malformed("section bounds"));
        }
        out[dst..dst + len].copy_from_slice(&image.bytes[src..src + len]);
    }
    Ok(out)
}

/// Apply base relocations to a mapped image.
///
/// `delta = actual_base - preferred_base`; unsupported relocation types are
/// reported instead of being silently ignored.
pub fn apply_relocations(
    mapped: &mut [u8],
    image: &Pe64Image,
    actual_base: u64,
) -> Result<usize, Pe64Error> {
    // M35: reject unsupported relocation kinds even when the base is
    // unchanged; otherwise a malformed image silently passes at its
    // preferred base and fails only after being relocated.
    if image
        .relocations
        .iter()
        .any(|r| r.kind != RELOC_DIR64 && r.kind != RELOC_HIGHLOW)
    {
        return Err(Pe64Error::Unsupported("relocation type"));
    }
    let delta = actual_base.wrapping_sub(image.image_base);
    if delta == 0 {
        return Ok(0);
    }
    let mut applied = 0usize;
    for reloc in &image.relocations {
        let off = reloc.rva as usize;
        match reloc.kind {
            RELOC_DIR64 => {
                let slice = mapped
                    .get_mut(off..off + 8)
                    .ok_or(Pe64Error::Malformed("DIR64 site"))?;
                let value = u64::from_le_bytes(slice.try_into().unwrap());
                slice.copy_from_slice(&value.wrapping_add(delta).to_le_bytes());
                applied += 1;
            }
            RELOC_HIGHLOW => {
                let slice = mapped
                    .get_mut(off..off + 4)
                    .ok_or(Pe64Error::Malformed("HIGHLOW site"))?;
                let value = u32::from_le_bytes(slice.try_into().unwrap());
                slice.copy_from_slice(&value.wrapping_add(delta as u32).to_le_bytes());
                applied += 1;
            }
            _ => return Err(Pe64Error::Unsupported("relocation type")),
        }
    }
    Ok(applied)
}

/// Bind one IAT slot to an HLE handler address.
pub fn bind_iat(mapped: &mut [u8], rva: u32, handler: u64) -> Result<(), Pe64Error> {
    let off = rva as usize;
    let slice = mapped
        .get_mut(off..off + 8)
        .ok_or(Pe64Error::Malformed("IAT slot"))?;
    slice.copy_from_slice(&handler.to_le_bytes());
    Ok(())
}

/// Generate a tiny synthetic PE32+ image for first-boot tests.
/// Build a tiny synthetic PE32+ image (one .text section, one DIR64
/// relocation, one import descriptor). All fixtures are generated from
/// source — no third-party binaries are used.
pub fn synthetic_pe() -> Vec<u8> {
    build_synthetic_pe(None)
}

/// Build a synthetic PE32+ image whose entry point calls `module!symbol`
/// through its IAT slot.
///
/// The generated x86-64 entry code is:
///
/// ```text
/// mov  ecx, 64            ; argument 0 (Microsoft x64 convention)
/// xor  edx, edx           ; argument 1 = 0
/// call qword ptr [rip+X]  ; indirect call through the IAT slot
/// mov  qword ptr [rax], 0x11
/// ret
/// ```
///
/// Limits (asserted): `module` ≤ 15 bytes, `symbol` ≤ 61 bytes. The
/// base-relocation directory is cleared because the fixture's DIR64 test
/// patch site overlaps the entry code. Like [`synthetic_pe`] this is a
/// fixture generated from source — no third-party binary is involved.
/// Build a self-contained synthetic guest for the M33 first-boot test.
///
/// Starts from [`synthetic_pe_calling`] (`xmem.dll!XMemAlloc`) and replaces
/// only the entry code. The guest passes `(64, 0)` in `RCX`/`RDX`, calls the
/// single IAT slot, writes a marker through the returned guest pointer and
/// finishes with a documented status in `RAX`:
///
/// | Bytes | Instruction | Purpose |
/// |---|---|---|
/// | `B9 40 00 00 00` | `mov ecx, 64` | `XMemAlloc(size=64, …)` |
/// | `31 D2` | `xor edx, edx` | `XMemAlloc(…, attributes=0)` |
/// | `FF 15 F3 05 00 00` | `call [rip+0x5f3]` | IAT slot at RVA `0x1600` |
/// | `48 C7 00 11 00 00 00` | `mov qword [rax], 0x11` | guest heap marker |
/// | `B8 33 4D 00 00` | `mov eax, 0x4D33` | **documented exit status** |
/// | `C3` | `ret` | entry return sentinel |
///
/// `0x4D33` ("M33") is a synthetic marker chosen by xHoly. It is **not** an
/// Xbox status code. Every opcode is in the supported x86-64 subset; no
/// CPU extension is required.
pub const SYNTHETIC_BOOT_STATUS: u64 = 0x4D33;
/// Marker the synthetic guest writes through the HLE-returned pointer.
pub const SYNTHETIC_BOOT_MARKER: u64 = 0x11;

/// Byte encoding of the M33 guest entry code (RVA `0x1000`, file `0x400`).
pub const SYNTHETIC_BOOT_CODE: &[u8] = &[
    0xB9, 0x40, 0x00, 0x00, 0x00, // mov ecx, 64
    0x31, 0xD2, // xor edx, edx
    0xFF, 0x15, 0xF3, 0x05, 0x00, 0x00, // call [rip+0x5f3] -> RVA 0x1600
    0x48, 0xC7, 0x00, 0x11, 0x00, 0x00, 0x00, // mov qword [rax], 0x11
    0xB8, 0x33, 0x4D, 0x00, 0x00, // mov eax, 0x4D33 (status)
    0xC3, // ret
];

pub fn synthetic_boot_guest() -> Vec<u8> {
    let mut bytes = synthetic_pe_calling("xmem.dll", "XMemAlloc");
    bytes[0x400..0x400 + SYNTHETIC_BOOT_CODE.len()].copy_from_slice(SYNTHETIC_BOOT_CODE);
    bytes
}

/// Expected guest result of [`synthetic_compute_guest`] (M35).
///
/// Loop sum (5 × 3 = 15) plus the `SETcc` success bit (1) = **16**.
pub const SYNTHETIC_COMPUTE_RESULT: u64 = 16;

/// Byte encoding of the M35 compute guest (RVA `0x1000`, file `0x400`).
///
/// | Offset | Bytes | Instruction | Purpose |
/// |---|---|---|---|
/// | 0x00 | `B9 03 00 00 00` | `mov ecx, 3` | loop counter |
/// | 0x05 | `31 C0` | `xor eax, eax` | accumulator = 0 |
/// | 0x07 | `83 C0 05` | `add eax, 5` | accumulate |
/// | 0x0A | `FF C9` | `dec ecx` | counter-- (preserves CF) |
/// | 0x0C | `75 F9` | `jnz -7` | loop back to 0x07 |
/// | 0x0E | `48 8D 48 01` | `lea rcx, [rax+1]` | size = 15 + 1 = 16 (RCX = arg0) |
/// | 0x12 | `31 D2` | `xor edx, edx` | attributes = 0 (RDX = arg1) |
/// | 0x14 | `41 89 C0` | `mov r8d, eax` | preserve the loop sum across HLE |
/// | 0x17 | `FF 15 <disp32>` | `call [rip+disp]` | IAT slot RVA 0x1600 |
/// | 0x1D | `85 C0` | `test eax, eax` | pointer non-zero? |
/// | 0x1F | `0F 95 C1` | `setnz cl` | CL = 1 on success |
/// | 0x22 | `0F B6 C9` | `movzx ecx, cl` | zero-extend the bit |
/// | 0x25 | `44 01 C1` | `add ecx, r8d` | ECX = 1 + 15 = 16 |
/// | 0x28 | `89 C8` | `mov eax, ecx` | result in RAX |
/// | 0x2A | `C3` | `ret` | sentinel |
///
/// The displacement at 0x19 is `0x1600 - (0x1000 + 0x1D) = 0x5E3`.
pub const SYNTHETIC_COMPUTE_CODE: &[u8] = &[
    0xB9, 0x03, 0x00, 0x00, 0x00, // mov ecx, 3
    0x31, 0xC0, // xor eax, eax
    0x83, 0xC0, 0x05, // add eax, 5
    0xFF, 0xC9, // dec ecx
    0x75, 0xF9, // jnz -7 -> 0x07
    0x48, 0x8D, 0x48, 0x01, // lea rcx, [rax+1] (size=16 in arg0)
    0x31, 0xD2, // xor edx, edx (attributes=0 in arg1)
    0x41, 0x89, 0xC0, // mov r8d, eax (preserve loop sum)
    0xFF, 0x15, 0xE3, 0x05, 0x00, 0x00, // call [rip+0x5e3] -> 0x1600
    0x85, 0xC0, // test eax, eax
    0x0F, 0x95, 0xC1, // setnz cl
    0x0F, 0xB6, 0xC9, // movzx ecx, cl
    0x44, 0x01, 0xC1, // add ecx, r8d
    0x89, 0xC8, // mov eax, ecx
    0xC3, // ret
];

/// Build a more complex self-authored guest for M35.
///
/// Combines a counted loop, INC/DEC-style counter update, conditional
/// branch, LEA address arithmetic, an HLE allocation, `TEST`/`SETcc` and
/// `MOVZX`. The returned `RAX` must equal [`SYNTHETIC_COMPUTE_RESULT`].
pub fn synthetic_compute_guest() -> Vec<u8> {
    let mut bytes = synthetic_pe_calling("xmem.dll", "XMemAlloc");
    bytes[0x400..0x400 + SYNTHETIC_COMPUTE_CODE.len()].copy_from_slice(SYNTHETIC_COMPUTE_CODE);
    bytes
}

pub fn synthetic_pe_calling(module: &str, symbol: &str) -> Vec<u8> {
    assert!(
        module.len() <= 15,
        "fixture module name too long: {module} (max 15 bytes)"
    );
    assert!(
        symbol.len() <= 61,
        "fixture symbol name too long: {symbol} (max 61 bytes)"
    );
    build_synthetic_pe(Some((module, symbol)))
}

/// Shared fixture builder; `call` selects the entry-code variant.
fn build_synthetic_pe(call: Option<(&str, &str)>) -> Vec<u8> {
    let mut d = vec![0u8; 0x1000];
    d[0..2].copy_from_slice(b"MZ");
    d[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    let pe = 0x80usize;
    d[pe..pe + 4].copy_from_slice(b"PE\0\0");
    let coff = pe + 4;
    d[coff..coff + 2].copy_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    d[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    d[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let opt = coff + 20;
    d[opt..opt + 2].copy_from_slice(&PE32PLUS_MAGIC.to_le_bytes());
    d[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry
    d[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    d[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    d[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    d[opt + 56..opt + 60].copy_from_slice(&0x3000u32.to_le_bytes());
    d[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes());
    d[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    // import directory RVA 0x1400 -> file 0x800 (one descriptor)
    d[opt + 112 + 8..opt + 112 + 12].copy_from_slice(&0x1400u32.to_le_bytes());
    // relocation directory RVA 0x1700 -> file 0xB00
    d[opt + 112 + 40..opt + 112 + 44].copy_from_slice(&0x1700u32.to_le_bytes());
    d[opt + 112 + 44..opt + 112 + 48].copy_from_slice(&16u32.to_le_bytes());
    // section header at opt+240
    let sec = opt + 240;
    d[sec..sec + 5].copy_from_slice(b".text");
    d[sec + 8..sec + 12].copy_from_slice(&0x100u32.to_le_bytes());
    d[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    d[sec + 16..sec + 20].copy_from_slice(&0xC00u32.to_le_bytes());
    d[sec + 20..sec + 24].copy_from_slice(&0x400u32.to_le_bytes());
    d[sec + 36..sec + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
    // synthetic guest code at RVA 0x1000 (file 0x400): mov eax,42; ret
    // (B8 + 4-byte little-endian imm32 + C3).
    d[0x400..0x408].copy_from_slice(&[0xB8, 42, 0, 0, 0, 0xC3, 0, 0]);
    // import descriptor at file 0x800: name RVA 0x1500, IAT 0x1600
    d[0x800 + 12..0x800 + 16].copy_from_slice(&0x1500u32.to_le_bytes());
    d[0x800 + 16..0x800 + 20].copy_from_slice(&0x1600u32.to_le_bytes());
    // IAT slot at RVA 0x1600 -> file 0xA00, by name RVA 0x1510
    d[0xA00..0xA08].copy_from_slice(&0x1510u64.to_le_bytes());
    d[0xA08..0xA10].copy_from_slice(&0u64.to_le_bytes());
    // DLL name and hint/name entry at RVA 0x1510 (file 0x910): the
    // 2-byte hint is skipped by `Pe64::import_name`, exactly like a real
    // hint/name table entry.
    d[0x900..0x910].copy_from_slice(b"d3d12_xs.dll\0\0\0\0");
    d[0x910..0x912].copy_from_slice(&0u16.to_le_bytes());
    d[0x912..0x924].copy_from_slice(b"D3D12CreateDevice\0");
    // relocation block at file 0xB00: page 0x1000, size 12, DIR64 at 0x1008
    // (kept clear of the 6-byte entry code at 0x1000..0x1006).
    d[0xB00..0xB04].copy_from_slice(&0x1000u32.to_le_bytes());
    d[0xB04..0xB08].copy_from_slice(&12u32.to_le_bytes());
    d[0xB08..0xB0A].copy_from_slice(&((RELOC_DIR64 << 12) | 8u16).to_le_bytes());

    if let Some((module, symbol)) = call {
        // Import the requested symbol instead of d3d12_xs.dll's default.
        // The hint/name entry moves to RVA 0x1530 (file 0x930) so symbols
        // of any realistic length fit; the IAT slot follows it.
        d[0x900..0x900 + module.len()].copy_from_slice(module.as_bytes());
        d[0x900 + module.len()] = 0;
        d[0x930..0x932].copy_from_slice(&0u16.to_le_bytes());
        d[0x932..0x932 + symbol.len()].copy_from_slice(symbol.as_bytes());
        d[0x932 + symbol.len()] = 0;
        d[0xA00..0xA08].copy_from_slice(&0x1530u64.to_le_bytes());
        // The entry code below occupies the bytes used by the DIR64
        // relocation patch site of `synthetic_pe`, so drop the
        // relocation directory for this variant.
        d[opt + 112 + 40..opt + 112 + 48].fill(0);
        // Entry code at RVA 0x1000 (file 0x400).
        let code: &[u8] = &[
            0xB9, 0x40, 0x00, 0x00, 0x00, // mov  ecx, 64
            0x31, 0xD2, //                   xor  edx, edx
            0xFF, 0x15, 0xF3, 0x05, 0x00, 0x00, // call [rip+0x5f3] -> RVA 0x1600
            0x48, 0xC7, 0x00, 0x11, 0x00, 0x00, 0x00, // mov qword [rax], 0x11
            0xC3, //                         ret
        ];
        d[0x400..0x400 + code.len()].copy_from_slice(code);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pe32plus_and_sections() {
        let image = parse(&synthetic_pe()).expect("parse synthetic PE");
        assert_eq!(image.machine, IMAGE_FILE_MACHINE_AMD64);
        assert_eq!(image.entry_rva, 0x1000);
        assert_eq!(image.entry_va(), 0x1_4000_1000);
        assert_eq!(image.size_of_image, 0x3000);
        assert_eq!(image.sections.len(), 1);
        assert_eq!(image.sections[0].name, ".text");
        assert_eq!(
            image.section_at(0x1004).map(|s| s.name.as_str()),
            Some(".text")
        );
    }

    #[test]
    fn rejects_non_pe_and_wrong_machine() {
        assert!(parse(b"NOPE").is_err());
        let mut bad = synthetic_pe();
        bad[0x84..0x86].copy_from_slice(&0xAA64u16.to_le_bytes()); // ARM64
        assert_eq!(
            parse(&bad),
            Err(Pe64Error::Unsupported("machine is not x86-64"))
        );
    }

    #[test]
    fn maps_sections_into_guest_memory() {
        let image = parse(&synthetic_pe()).unwrap();
        let mapped = map_image(&image).unwrap();
        assert_eq!(mapped.len(), 0x3000);
        assert_eq!(&mapped[0x1000..0x1008], &[0xB8, 42, 0, 0, 0, 0xC3, 0, 0]);
    }

    #[test]
    fn applies_dir64_relocation() {
        let mut image = parse(&synthetic_pe()).unwrap();
        let mut mapped = map_image(&image).unwrap();
        // relocate to a different base; patch site initially zero.
        mapped[0x1008..0x1010].fill(0);
        let n = apply_relocations(&mut mapped, &image, 0x1_5000_0000).unwrap();
        assert_eq!(n, 1);
        let value = u64::from_le_bytes(mapped[0x1008..0x1010].try_into().unwrap());
        assert_eq!(value, 0x1000_0000);
        // Entry code at 0x1000..0x1006 must remain intact after relocation.
        assert_eq!(&mapped[0x1000..0x1006], &[0xB8, 42, 0, 0, 0, 0xC3]);
        image.image_base = 0x1_5000_0000;
        assert_eq!(
            apply_relocations(&mut mapped, &image, 0x1_5000_0000).unwrap(),
            0
        );
    }

    #[test]
    fn parses_imports_and_binds_iat() {
        let image = parse(&synthetic_pe()).unwrap();
        assert!(image.looks_like_xbox_gdk());
        assert_eq!(image.imports.len(), 1);
        assert_eq!(image.imports[0].dll, "d3d12_xs.dll");
        let mut mapped = map_image(&image).unwrap();
        let iat = image.imports[0].first_thunk;
        assert_eq!(iat, 0x1600);
        bind_iat(&mut mapped, iat, 0xDEAD_BEEF).unwrap();
        assert_eq!(
            u64::from_le_bytes(mapped[0x1600..0x1608].try_into().unwrap()),
            0xDEAD_BEEF
        );
    }

    #[test]
    fn validate_accepts_the_synthetic_image() {
        let image = parse(&synthetic_pe()).unwrap();
        validate(&image).expect("fixture must be structurally valid");
        let calling = parse(&synthetic_pe_calling("xmem.dll", "XMemAlloc")).unwrap();
        validate(&calling).expect("call fixture must be structurally valid");
    }

    #[test]
    fn hle_call_fixture_carries_the_requested_import_and_real_code() {
        let image = parse(&synthetic_pe_calling("xmem.dll", "XMemAlloc")).unwrap();
        assert_eq!(image.imports[0].dll, "xmem.dll");
        assert_eq!(image.imports[0].symbols.len(), 1);
        let name_rva = match image.imports[0].symbols[0] {
            Import::ByName(rva) => rva,
            Import::ByOrdinal(o) => panic!("expected import by name, got ordinal {o}"),
        };
        assert_eq!(image.import_name(name_rva).as_deref(), Some("XMemAlloc"));
        // No base relocations: the entry code owns those bytes.
        assert!(image.relocations.is_empty());
        // mov ecx, 64 ; xor edx, edx ; call [rip+disp] ; mov [rax], 0x11 ; ret
        let code = image.read_rva(0x1000, 21).expect("entry code");
        assert_eq!(&code[0..5], &[0xB9, 0x40, 0x00, 0x00, 0x00]);
        assert_eq!(&code[5..7], &[0x31, 0xD2]);
        assert_eq!(&code[7..13], &[0xFF, 0x15, 0xF3, 0x05, 0x00, 0x00]);
        assert_eq!(code[20], 0xC3, "fixture ends with ret");
        // The indirect call targets the IAT slot: 0x100D + 0x5F3 = 0x1600.
        assert_eq!(image.imports[0].first_thunk, 0x1600);
    }

    #[test]
    fn validate_rejects_structurally_broken_images() {
        // Entry point outside the image.
        let mut image = parse(&synthetic_pe()).unwrap();
        image.entry_rva = image.size_of_image;
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("entry point outside the image"))
        ));

        // Section extending past SizeOfImage.
        let mut image = parse(&synthetic_pe()).unwrap();
        image.sections[0].virtual_address = 0x2F00;
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("section exceeds SizeOfImage"))
        ));

        // Section raw data extending past the file.
        let mut image = parse(&synthetic_pe()).unwrap();
        image.sections[0].raw_pointer = image.bytes.len() as u32 - 1;
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("section raw data exceeds file size"))
        ));

        // Raw data overlapping the headers.
        let mut image = parse(&synthetic_pe()).unwrap();
        image.sections[0].raw_pointer = 0x200;
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("section raw data overlaps headers"))
        ));

        // Two sections occupying the same virtual range.
        let mut image = parse(&synthetic_pe()).unwrap();
        let mut overlap = image.sections[0].clone();
        overlap.virtual_address = 0x1800;
        image.sections.push(overlap);
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("section overlaps a previous section"))
        ));

        // Headers larger than the image.
        let mut image = parse(&synthetic_pe()).unwrap();
        image.size_of_headers = image.size_of_image + 1;
        assert!(matches!(
            validate(&image),
            Err(Pe64Error::Malformed("SizeOfHeaders exceeds SizeOfImage"))
        ));
    }
}
