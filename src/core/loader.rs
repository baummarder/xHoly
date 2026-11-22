//! Game loader — Phase 8.
//!
//! Loads guest game images into the AArch64 interpreter:
//!
//! - **Flat images** (`.bin`, `.img`, `.xex` payloads we can map):
//!   raw bytes loaded at [`USER_BASE`](crate::core::kernel::USER_BASE).
//! - **XEX headers**: the `XEX2` header is parsed for the load address
//!   and entry point when present (real XEX layout is more complex —
//!   see limitations).
//! - **Game directories**: scans for `default.xex`/`*.xex` plus
//!   `xboxconfig.xml` metadata (title, title id, path).
//!
//! This is a *loader scaffold*: it produces a [`GameImage`] the kernel
//! + interpreter can execute. It does not implement the real Xbox
//!
//! executable format (XEX sections, imports, TLS, etc.).

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

/// Parse a minimal `XEX2` header: magic + module flags + header size +
/// timestamp. Real XEX has ~20 optional headers (kernel imports, TLS,
/// sections); we extract the load address and entry point fields that
/// appear at fixed offsets in retail headers.
///
/// Layout (offsets from file start):
/// ```text
/// 0x00  magic "XEX2"
/// 0x04  module_flags u32
/// 0x08  header_size u32   (usually 0x178)
/// 0x0C  reserved u32
/// 0x10  timestamp u32
/// ...
/// 0x104 entry_point u32   (guest VA, low 32 bits)
/// 0x108 image_base u32    (guest VA)
/// ```
fn parse_xex_header(data: &[u8]) -> Result<(u64, u64), LoadError> {
    if data.len() < 0x110 {
        return Err(LoadError::BadXex(PathBuf::new()));
    }
    if &data[0..4] != b"XEX2" {
        return Err(LoadError::BadXex(PathBuf::new()));
    }
    let header_size = u32::from_le_bytes(data[0x08..0x0C].try_into().unwrap()) as usize;
    if header_size < 0x110 || header_size > data.len() {
        return Err(LoadError::BadXex(PathBuf::new()));
    }
    // Entry point & image base live in the header area.
    let entry = u32::from_le_bytes(data[0x104..0x108].try_into().unwrap()) as u64;
    let image_base = u32::from_le_bytes(data[0x108..0x10C].try_into().unwrap()) as u64;
    // XEX VAs are typically 0x82xxxxxx (guest VA); our flat memory uses
    // offsets — map VA → offset by subtracting image_base if nonzero.
    Ok((image_base, entry))
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
            if p.extension().map(|x| x == "xex").unwrap_or(false) && !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for ext in ["bin", "img", "xbe"] {
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

    // Merge xboxconfig metadata.
    let cfg = dir.join("xboxconfig.xml");
    if cfg.is_file() {
        if let Ok(meta) = parse_xboxconfig(&cfg) {
            if img.title.is_empty() {
                img.title = meta.name.clone().unwrap_or_else(|| img.title.clone());
            }
            img.title_id = meta.title_id.clone();
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

fn load_image_bytes(path: &Path, data: &[u8]) -> Result<GameImage, LoadError> {
    if data.len() < 4 {
        return Err(LoadError::Unrecognized(path.to_path_buf()));
    }
    let (load_base, entry_offset) = if &data[0..4] == b"XEX2" {
        let (image_base, entry) = parse_xex_header(data)?;
        // XEX stores a VA; our flat memory is offset-based. If the VA
        // looks like a real Xbox VA (0x82xxxxxx), map to USER_BASE.
        let base = if image_base >= 0x8000_0000 {
            USER_BASE
        } else {
            image_base.max(USER_BASE)
        };
        (base, entry)
    } else {
        (USER_BASE, USER_BASE)
    };

    // For XEX the code doesn't start at file offset 0 — sections live
    // after the header. As a scaffold we load the whole file; a real
    // loader would walk the section table.
    let entry = if entry_offset >= load_base {
        entry_offset
    } else {
        load_base + entry_offset
    };

    Ok(GameImage {
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<untitled>".into()),
        title_id: None,
        load_base,
        entry,
        image: data.to_vec(),
        source: path.to_path_buf(),
        metadata: GameMetadata::default(),
    })
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
