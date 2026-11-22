//! Parsed game image + asset catalog — Phase 10.
//!
//! Richer view than the executable-only [`super::GameImage`]: keeps
//! the disc-level metadata (title, region, discs, release date) and
//! an [`AssetCatalog`] of discovered payloads (shaders, textures,
//! config) that downstream stages (shader translation, texture
//! loading) consume.

use serde::{Deserialize, Serialize};

/// Region classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameRegion {
    /// NTSC-U / NTSC-J etc. — grouped as "NTSC".
    Ntsc,
    /// PAL regions.
    Pal,
    /// Japanese-only.
    Jpn,
    /// Unknown.
    Other,
}

impl GameRegion {
    /// Parse from a region string.
    pub fn parse(s: &str) -> Self {
        let u = s.to_ascii_uppercase();
        if u.contains("PAL") {
            Self::Pal
        } else if u.contains("JPN") || u.contains("JAPAN") {
            Self::Jpn
        } else if u.contains("NTSC") {
            Self::Ntsc
        } else {
            Self::Other
        }
    }
    /// Display string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ntsc => "NTSC",
            Self::Pal => "PAL",
            Self::Jpn => "JPN",
            Self::Other => "OTHER",
        }
    }
}

/// One disc in a multi-disc release.
#[derive(Debug, Clone)]
pub struct DiscInfo {
    /// 1-based disc number.
    pub number: u8,
    /// Path/identifier of this disc's payload.
    pub source: String,
    /// Whether this is the disc that contains the executable.
    pub has_executable: bool,
}

/// A discovered asset payload (shader blob, texture, config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Path inside the source image.
    pub path: String,
    /// Asset kind tag (`shader`, `texture`, `config`, `other`).
    pub kind: String,
    /// Size in bytes.
    pub size: u64,
}

/// Catalog of assets extracted from the game image.
#[derive(Debug, Clone, Default)]
pub struct AssetCatalog {
    pub entries: Vec<AssetEntry>,
}

impl AssetCatalog {
    /// All entries of a kind.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a AssetEntry> + 'a {
        self.entries.iter().filter(move |e| e.kind == kind)
    }
    /// Shader entries only.
    pub fn shaders(&self) -> impl Iterator<Item = &AssetEntry> {
        self.of_kind("shader")
    }
}

/// Fully parsed game image (spec structure from the Phase-10 prompt).
#[derive(Debug, Clone)]
pub struct ParsedGameImage {
    /// Display title.
    pub title: String,
    /// Region.
    pub region: GameRegion,
    /// Disc list (multi-disc support).
    pub discs: Vec<DiscInfo>,
    /// Asset catalog.
    pub assets: AssetCatalog,
    /// Guest entry point (VA).
    pub entry_point: u64,
    /// Image base (VA).
    pub image_base: u64,
    /// Title id when known.
    pub title_id: Option<String>,
    /// Release date string when known.
    pub release_date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_parsing() {
        assert_eq!(GameRegion::parse("PAL-EUR"), GameRegion::Pal);
        assert_eq!(GameRegion::parse("NTSC-U"), GameRegion::Ntsc);
        assert_eq!(GameRegion::parse("JPN"), GameRegion::Jpn);
        assert_eq!(GameRegion::parse("??"), GameRegion::Other);
    }

    #[test]
    fn asset_catalog_filters() {
        let mut cat = AssetCatalog::default();
        cat.entries.push(AssetEntry {
            path: "GAME/shader.cso".into(),
            kind: "shader".into(),
            size: 128,
        });
        cat.entries.push(AssetEntry {
            path: "GAME/tex.dds".into(),
            kind: "texture".into(),
            size: 64,
        });
        assert_eq!(cat.shaders().count(), 1);
        assert_eq!(cat.of_kind("texture").count(), 1);
    }
}
