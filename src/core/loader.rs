//! Game loader — Phase 8.
//!
//! Loads guest game images into the AArch64 interpreter:
//!
//! - **Flat images** (`.bin`, `.img`, `.xex` payloads we can map):
//!   raw bytes loaded at [`USER_BASE`](crate::core::kernel::USER_BASE).
//! - **XEX headers**: the `XEX2` header is parsed for load address,
//!   entry point and (Phase 19) the optional-header directory plus the
//!   security block's section layout. Both real big-endian XEX2 and
//!   xHoly's minimal little-endian fixture are recognized.
//! - **Game directories**: scans for `default.xex`/`*.xex` plus
//!   `xboxconfig.xml` metadata (title, title id, path).
//!
//! This is still a *loader scaffold*: it produces a [`GameImage`] the
//! kernel + interpreter can execute. It does not decrypt or decompress
//! the image, parse the embedded PowerPC PE or service the real import
//! directory — see `xex_parser` and STATUS.md for honest limits.

pub mod game_image;
pub mod game_runner;
pub mod iso_parser;
pub mod xex_parser;

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::kernel::USER_BASE;

/// Errors loading a game image.
#[derive(Debug)]
pub enum LoadError {
    /// Filesystem error.
    Io(std::io::Error),
    /// File exists but isn't a recognized game image.
    Unrecognized(PathBuf),
    /// XEX header present but malformed.
    BadXex(PathBuf),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Unrecognized(p) => write!(f, "unrecognized game image: {}", p.display()),
            Self::BadXex(p) => write!(f, "malformed XEX header: {}", p.display()),
        }
    }
}

impl std::error::Error for LoadError {}
impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A loaded game image ready for the CPU.
#[derive(Debug, Clone)]
pub struct GameImage {
    /// Display title (from xboxconfig or filename).
    pub title: String,
    /// Title ID if present in xboxconfig.
    pub title_id: Option<String>,
    /// Guest load base.
    pub load_base: u64,
    /// Entry point (guest address).
    pub entry: u64,
    /// Image bytes to copy into guest memory at `load_base`.
    pub image: Vec<u8>,
    /// Path the image was loaded from.
    pub source: PathBuf,
    /// Extra metadata from xboxconfig.xml.
    pub metadata: GameMetadata,
}

/// Parsed `xboxconfig.xml` fields.
#[derive(Debug, Clone, Default)]
pub struct GameMetadata {
    /// <TitleID>
    pub title_id: Option<String>,
    /// <Name>
    pub name: Option<String>,
    /// <Studio>
    pub studio: Option<String>,
    /// <Path> (disc root path inside the image)
    pub path: Option<String>,
    /// <ReleaseDate> if present
    pub release_date: Option<String>,
}

impl GameImage {
    /// Copy this image into a CPU's memory and set `pc` to the entry.
    pub fn load_into(
        &self,
        cpu: &mut crate::core::cpu::interpreter::Arm64Cpu,
    ) -> Result<(), LoadError> {
        cpu.load_at(self.load_base, &self.image)
            .map_err(|_| LoadError::BadXex(self.source.clone()))?;
        cpu.pc = self.entry;
        cpu.sp = cpu.va_base + cpu.memory.len() as u64 - 16;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// XEX-lite header parsing
// ---------------------------------------------------------------------------

/// Parse an `XEX2` header — delegates to the structured
/// [`xex_parser`] (fixed fields, optional-header directory, section
/// layout and import stub). Both the real big-endian layout and the
/// minimal scaffold fixture are supported.
fn parse_xex_header(data: &[u8], source: &Path) -> Result<xex_parser::XexHeader, LoadError> {
    xex_parser::parse_xex(data, source)
}

// ---------------------------------------------------------------------------
// xboxconfig.xml minimal parser
// ---------------------------------------------------------------------------

/// Extract `<Tag>value</Tag>` from xml-ish text (no real XML parser —
/// the config is machine-generated and flat).
fn xml_field(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let v = xml[start..end].trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Parse an `xboxconfig.xml` file.
pub fn parse_xboxconfig(path: &Path) -> Result<GameMetadata, LoadError> {
    let xml = fs::read_to_string(path)?;
    Ok(GameMetadata {
        title_id: xml_field(&xml, "TitleID"),
        name: xml_field(&xml, "Name"),
        studio: xml_field(&xml, "Studio"),
        path: xml_field(&xml, "Path"),
        release_date: xml_field(&xml, "ReleaseDate"),
    })
}

// ---------------------------------------------------------------------------
// Public loader API
// ---------------------------------------------------------------------------

/// Load a game image from `path` (file or directory).
///
/// - Directory: looks for `default.xex`, `*.xex`, or the first `.bin/.img`;
///   parses `xboxconfig.xml` if present.
/// - File: detects XEX2 magic → XEX path; otherwise flat image.
pub fn load_game(path: &Path) -> Result<GameImage, LoadError> {
    if path.is_dir() {
        return load_game_dir(path);
    }
    let data = fs::read(path)?;
    load_image_bytes(path, &data)
}

fn load_game_dir(dir: &Path) -> Result<GameImage, LoadError> {
    // Prefer default.xex, then any .xex, then .bin/.img.
    let mut candidates: Vec<PathBuf> = Vec::new();
    let default_xex = dir.join("default.xex");
    if default_xex.is_file() {
        candidates.push(default_xex);
    }
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let recognized = p
                .extension()
                .map(|x| x == "xex" || x == "exe" || x == "xbe")
                .unwrap_or(false);
            if recognized && !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for ext in ["bin", "img"] {
        let p = dir.join(format!("default.{ext}"));
        if p.is_file() {
            candidates.push(p);
        }
    }
    let exe = candidates
        .into_iter()
        .next()
        .ok_or_else(|| LoadError::Unrecognized(dir.to_path_buf()))?;

    let data = fs::read(&exe)?;
    let mut img = load_image_bytes(&exe, &data)?;

    // Merge xboxconfig metadata — an explicit <Name> wins over the
    // executable's file stem.
    let cfg = dir.join("xboxconfig.xml");
    if cfg.is_file() {
        if let Ok(meta) = parse_xboxconfig(&cfg) {
            if let Some(name) = &meta.name {
                img.title = name.clone();
            } else if img.title.is_empty() {
                img.title = meta.name.clone().unwrap_or_else(|| img.title.clone());
            }
            // xboxconfig <TitleID> is authoritative over the raw
            // XEX header quad.
            if meta.title_id.is_some() {
                img.title_id = meta.title_id.clone();
            }
            img.metadata = meta;
        }
    }
    if img.title.is_empty() {
        img.title = exe
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<untitled>".into());
    }
    Ok(img)
}

/// Build a [`GameImage`] from raw file bytes (shared by the file and
/// directory paths; also usable by the ISO runner).
pub fn image_from_bytes(path: &Path, data: &[u8]) -> Result<GameImage, LoadError> {
    if data.len() < 4 {
        return Err(LoadError::Unrecognized(path.to_path_buf()));
    }
    let mut title_id = None;
    let (load_base, entry, image) = if &data[0..4] == b"XEX2" {
        let hdr = parse_xex_header(data, path)?;
        // XEX layout: header_size bytes of header, then the image
        // payload. Only the payload is mapped into guest memory.
        let hs = hdr.header_size as usize;
        if hs > data.len() {
            return Err(LoadError::BadXex(path.to_path_buf()));
        }
        // XEX VAs (0x82xxxxxx) map onto our USER_BASE window; the entry
        // point is stored as an RVA-style offset from image_base.
        let base = USER_BASE;
        let rva = hdr.entry_point.saturating_sub(hdr.image_base) as u64;
        title_id = xex_parser::format_title_id(&hdr.title_id);
        (base, base + rva, data[hs..].to_vec())
    } else {
        (USER_BASE, USER_BASE, data.to_vec())
    };

    Ok(GameImage {
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<untitled>".into()),
        title_id,
        load_base,
        entry,
        image,
        source: path.to_path_buf(),
        metadata: GameMetadata::default(),
    })
}

fn load_image_bytes(path: &Path, data: &[u8]) -> Result<GameImage, LoadError> {
    image_from_bytes(path, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_xboxconfig_fields() {
        let dir = std::env::temp_dir().join("xholy_loader_test");
        fs::create_dir_all(&dir).unwrap();
        let xml = r#"<?xml version="1.0"?>
<GameInfo>
  <TitleID>ABCD12345</TitleID>
  <Name>Test Game</Name>
  <Studio>Test Studio</Studio>
  <Path>/default.xex</Path>
</GameInfo>"#;
        let cfg = dir.join("xboxconfig.xml");
        fs::write(&cfg, xml).unwrap();
        let meta = parse_xboxconfig(&cfg).unwrap();
        assert_eq!(meta.title_id.as_deref(), Some("ABCD12345"));
        assert_eq!(meta.name.as_deref(), Some("Test Game"));
        assert_eq!(meta.studio.as_deref(), Some("Test Studio"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_flat_image() {
        let dir = std::env::temp_dir().join("xholy_flat_test");
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("test.bin");
        let mut f = fs::File::create(&bin).unwrap();
        f.write_all(&[0xAA; 256]).unwrap();
        let img = load_game(&bin).unwrap();
        assert_eq!(img.image.len(), 256);
        assert_eq!(img.load_base, USER_BASE);
        assert_eq!(img.entry, USER_BASE);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_file() {
        let dir = std::env::temp_dir().join("xholy_empty_test");
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("empty.bin");
        fs::write(&bin, b"").unwrap();
        assert!(matches!(load_game(&bin), Err(LoadError::Unrecognized(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_bundled_minimal_game_dir() {
        // games/minimal ships a real XEX2 + xboxconfig.xml fixture.
        let img = load_game(Path::new("games/minimal")).unwrap();
        assert_eq!(img.title, "xHoly Minimal Guest");
        assert_eq!(img.title_id.as_deref(), Some("XHOLYMINIMAL01"));
        assert_eq!(img.load_base, USER_BASE);
        assert_eq!(img.entry, USER_BASE);
        // Payload only — the XEX2 header (0x1A0 bytes) is stripped.
        assert_eq!(img.image.len(), 172);
        // First instruction: MOVZ x19, #0 (FB base setup).
        assert_eq!(&img.image[0..4], &[0x13, 0x00, 0x80, 0xD2]);
    }

    #[test]
    fn loads_bundled_minimal_xex_file() {
        let img = load_game(Path::new("games/minimal/default.xex")).unwrap();
        assert_eq!(img.image.len(), 172);
        assert_eq!(img.entry, USER_BASE);
        // Title id quad from the XEX header is picked up.
        assert!(img.title_id.is_some());
    }

    #[test]
    fn image_from_bytes_reads_real_xex2_directory() {
        // Phase 19: a real big-endian XEX2 header must not be ignored.
        // The loader used to byte-swap header_size at 0x08 and reject it.
        fn put_be32(d: &mut [u8], off: usize, v: u32) {
            d[off..off + 4].copy_from_slice(&v.to_be_bytes());
        }
        let mut data = vec![0u8; 0x80];
        data[0..4].copy_from_slice(b"XEX2");
        put_be32(&mut data, 0x08, 0x60); // header size (PE data offset)
        put_be32(&mut data, 0x10, 0); // no security block
        put_be32(&mut data, 0x14, 2); // two optional headers
        put_be32(&mut data, 0x18, 0x0001_0100); // entry point (inline)
        put_be32(&mut data, 0x1C, 0x8200_0020);
        put_be32(&mut data, 0x20, 0x0001_0201); // image base (inline data ref)
        put_be32(&mut data, 0x24, 0x8200_0000);
        data[0x60..0x64].copy_from_slice(&0xD503_201Fu32.to_le_bytes()); // NOP

        let img = image_from_bytes(Path::new("retail.xex"), &data).unwrap();
        assert_eq!(img.load_base, USER_BASE);
        assert_eq!(img.entry, USER_BASE + 0x20); // RVA from the directory
        assert_eq!(img.image, data[0x60..]); // header stripped via BE size
        assert!(img.title_id.is_none()); // no execution-info title id
    }

    #[test]
    fn game_image_loads_into_cpu() {
        let mut cpu =
            crate::core::cpu::interpreter::Arm64Cpu::new(crate::core::execution::GUEST_MEM_SIZE);
        cpu.va_base = crate::core::kernel::KERNEL_BASE;
        let img = GameImage {
            title: "t".into(),
            title_id: None,
            load_base: USER_BASE,
            entry: USER_BASE,
            image: vec![0xAA; 64],
            source: PathBuf::from("mem"),
            metadata: GameMetadata::default(),
        };
        img.load_into(&mut cpu).unwrap();
        assert_eq!(cpu.pc, USER_BASE);
        let off = (USER_BASE - cpu.va_base) as usize;
        assert_eq!(cpu.memory[off], 0xAA);
    }
}
