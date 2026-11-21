//! Disc/game metadata extraction.
//!
//! Parses `xboxconfig.xml`/`GameConfig.xml`-style configuration files and
//! ISO9660 volume descriptors into structured metadata.

use std::path::Path;

/// Disc region, derived from disc ID prefix or config flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Region {
    /// North America (disc IDs starting with `J` family codes vary).
    NtScu,
    /// Europe / PAL.
    Pal,
    /// Japan / Asia.
    Jpn,
    /// Unknown or unrestricted.
    #[default]
    Other,
}

impl Region {
    /// Infer region from an Xbox disc ID (e.g. `JUDG` → NTSC-U style,
    /// `PD6G`/`PE6G` → PAL, `HJUD` → JPN). Heuristic for the scaffold.
    pub fn from_disc_id(disc_id: &str) -> Self {
        let b = disc_id.as_bytes();
        if b.is_empty() {
            return Self::Other;
        }
        match b[0].to_ascii_uppercase() {
            b'J' | b'K' | b'L' => Self::NtScu,
            b'P' => Self::Pal,
            b'H' => Self::Jpn,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NtScu => "NTSC-U",
            Self::Pal => "PAL",
            Self::Jpn => "JPN",
            Self::Other => "OTHER",
        }
    }
}

/// Metadata for a single game installation on a disc.
#[derive(Debug, Clone, Default)]
pub struct GameMetadata {
    /// Xbox title ID (e.g. `1S1000GWW00000`).
    pub title_id: Option<String>,
    /// Human-readable title.
    pub title_name: Option<String>,
    /// Base version string from the config.
    pub version: Option<String>,
    /// Disc number for multi-disc titles (1-based; 0 = unspecified).
    pub disc_number: u32,
    /// Detected region.
    pub region: Region,
    /// Language codes declared by the title.
    pub languages: Vec<String>,
    /// Path (within the mounted filesystem) of the game payload folder.
    pub payload_path: Option<String>,
}

/// Metadata describing the mounted volume itself.
#[derive(Debug, Clone)]
pub struct DiscMetadata {
    /// ISO9660 volume ID / label.
    pub volume_label: String,
    /// System identifier (usually "MICROSOFT XBOX" family or "CDROM").
    pub system_id: String,
    /// Volume set / publication info when present.
    pub publisher: Option<String>,
    /// Volume creation timestamp decoded from the PVD.
    pub created: Option<String>,
    /// Detected filesystem family.
    pub filesystem: FilesystemKind,
}

/// Filesystem family detected on the mounted media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemKind {
    /// ISO 9660 (also covers Xbox One game discs).
    Iso9660,
    /// UDF (Blu-ray Video / BDAV). Parsing TBD — detected only.
    Udf,
    /// Plain host directory (physical drive already mounted by the OS).
    HostDirectory,
}

impl Default for DiscMetadata {
    fn default() -> Self {
        Self {
            volume_label: String::new(),
            system_id: String::new(),
            publisher: None,
            created: None,
            filesystem: FilesystemKind::HostDirectory,
        }
    }
}

/// Minimal XML-ish tag extractor used for `xboxconfig.xml`-style files.
///
/// These files are simple enough that a full XML parser is overkill; we
/// scan for `<TagName>value</TagName>` pairs case-insensitively.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let lower = xml.to_ascii_lowercase();
    let o = open.to_ascii_lowercase();
    let c = close.to_ascii_lowercase();
    let start = lower.find(&o)? + o.len();
    let end = lower[start..].find(&c)? + start;
    let value = xml[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse an `xboxconfig.xml` / `GameConfig.xml` payload into metadata.
///
/// Tolerant of schema variation: looks for common tag names used across
/// Xbox One title configurations.
pub fn parse_game_config(xml: &str, payload_path: &str) -> GameMetadata {
    let title_id = extract_tag(xml, "TitleId")
        .or_else(|| extract_tag(xml, "TitleID"))
        .or_else(|| extract_tag(xml, "GameId"));
    let title_name = extract_tag(xml, "DisplayName")
        .or_else(|| extract_tag(xml, "TitleName"))
        .or_else(|| extract_tag(xml, "Name"));
    let version = extract_tag(xml, "BaseVersion").or_else(|| extract_tag(xml, "Version"));
    let disc_number = extract_tag(xml, "DiscNumber")
        .or_else(|| extract_tag(xml, "DiskNumber"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    let region = extract_tag(xml, "Region")
        .map(|r| match r.to_ascii_uppercase().as_str() {
            "NTSC" | "NTSC-U" | "USA" => Region::NtScu,
            "PAL" => Region::Pal,
            "JPN" | "JAPAN" => Region::Jpn,
            _ => Region::Other,
        })
        .unwrap_or_else(|| {
            title_id
                .as_deref()
                .map(|id| Region::from_disc_id(id.split('-').next().unwrap_or(id)))
                .unwrap_or(Region::Other)
        });

    let languages = extract_tag(xml, "SupportedLanguages")
        .or_else(|| extract_tag(xml, "Languages"))
        .map(|s| {
            s.split([',', ';', ' '])
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
                .collect()
        })
        .unwrap_or_default();

    GameMetadata {
        title_id,
        title_name,
        version,
        disc_number,
        region,
        languages,
        payload_path: Some(payload_path.to_string()),
    }
}

/// Read and parse a game config file from a host directory.
pub fn parse_config_file(path: &Path) -> Option<GameMetadata> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some(parse_game_config(&text, &path.display().to_string()))
}
