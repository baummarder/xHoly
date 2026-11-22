//! ISO game-source wrapper — Phase 10.
//!
//! Sits on top of the Phase-4 ISO9660 reader
//! ([`crate::optical::iso_parser::Iso9660Image`]) and knows the Xbox
//! disc layout: `XBOX/` (system + executable), `GAME/` (assets),
//! `PATCH/`, `DATA/`, `DLC/`.

use std::path::Path;

use crate::optical::iso_parser::{Iso9660Image, IsoError};

/// Xbox disc top-level folders we care about.
pub const XBOX_DIR: &str = "XBOX";
pub const GAME_DIR: &str = "GAME";
pub const PATCH_DIR: &str = "PATCH";
pub const DATA_DIR: &str = "DATA";
pub const DLC_DIR: &str = "DLC";

/// A game payload discovered on the disc.
#[derive(Debug, Clone)]
pub struct DiscPayload {
    /// Path inside the ISO (e.g. `XBOX/DEFAULT.XEX;1`).
    pub iso_path: String,
    /// Kind guess from extension.
    pub kind: PayloadKind,
    /// Size in bytes.
    pub size: u32,
}

/// Payload classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    /// XEX executable.
    Executable,
    /// Shader container (DXBC/DXIL blob).
    Shader,
    /// Texture-ish asset.
    Texture,
    /// Other file.
    Other,
}

/// ISO-backed game source.
pub struct IsoGameSource {
    image: Iso9660Image,
    /// Display label (volume id).
    pub volume_label: String,
}

impl IsoGameSource {
    /// Open an ISO image.
    pub fn open(path: &Path) -> Result<Self, IsoError> {
        let image = Iso9660Image::open(path)?;
        let volume_label = image.metadata().volume_label;
        Ok(Self {
            image,
            volume_label,
        })
    }

    /// List a directory inside the ISO.
    pub fn list_dir(&mut self, path: &str) -> Result<Vec<String>, IsoError> {
        Ok(self
            .image
            .list_dir(path)?
            .into_iter()
            .map(|e| e.name)
            .collect())
    }

    /// Read a file from the ISO.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, IsoError> {
        self.image.read_file(path)
    }

    /// Does a path exist?
    pub fn exists(&mut self, path: &str) -> bool {
        self.image.exists(path)
    }

    /// Find candidate executables: `XBOX/*.xex`, `XBOX/GAME/*.xex`,
    /// `GAME/*.xex`, `default.xex` anywhere at depth ≤ 2.
    pub fn find_executables(&mut self) -> Vec<DiscPayload> {
        let mut out = Vec::new();
        for dir in [XBOX_DIR, &format!("{XBOX_DIR}/GAME"), GAME_DIR] {
            self.collect_xex(dir, &mut out);
        }
        out
    }

    fn collect_xex(&mut self, dir: &str, out: &mut Vec<DiscPayload>) {
        let Ok(entries) = self.image.list_dir(dir) else {
            return;
        };
        for e in entries {
            let p = format!("{dir}/{}", e.name);
            if e.is_dir {
                // One level of nesting (XBOX/GAME/...).
                self.collect_xex(&p, out);
            } else if e.name.to_ascii_lowercase().ends_with(".xex") {
                out.push(DiscPayload {
                    iso_path: p,
                    kind: PayloadKind::Executable,
                    size: e.size,
                });
            }
        }
    }

    /// Scan for shader-ish files (`.cso`, `.dxbc`, `.dxil`, `.shad`).
    pub fn find_shaders(&mut self) -> Vec<DiscPayload> {
        let mut out = Vec::new();
        for dir in [GAME_DIR, &format!("{XBOX_DIR}/GAME"), PATCH_DIR, DATA_DIR] {
            self.collect_ext(
                dir,
                &["cso", "dxbc", "dxil", "shad"],
                PayloadKind::Shader,
                &mut out,
            );
        }
        out
    }

    /// Scan for cover art (`*.png`, `*cover*`, `*.dds`, `*.tga`).
    pub fn find_cover_art(&mut self) -> Vec<DiscPayload> {
        let mut out = Vec::new();
        for dir in [GAME_DIR, XBOX_DIR, ""] {
            self.collect_ext(
                dir,
                &["png", "dds", "tga", "jpg"],
                PayloadKind::Texture,
                &mut out,
            );
        }
        // Also flag files with "cover" in the name under GAME/.
        out.retain(|p| {
            p.iso_path.to_ascii_lowercase().contains("cover") || p.iso_path.ends_with(".png")
        });
        out
    }

    fn collect_ext(
        &mut self,
        dir: &str,
        exts: &[&str],
        kind: PayloadKind,
        out: &mut Vec<DiscPayload>,
    ) {
        let Ok(entries) = self.image.list_dir(dir) else {
            return;
        };
        for e in entries {
            if e.is_dir {
                if dir.is_empty() || dir.matches('/').count() < 2 {
                    let sub = if dir.is_empty() {
                        e.name.clone()
                    } else {
                        format!("{dir}/{}", e.name)
                    };
                    self.collect_ext(&sub, exts, kind, out);
                }
                continue;
            }
            let lower = e.name.to_ascii_lowercase();
            if exts.iter().any(|x| lower.ends_with(&format!(".{x}"))) {
                let iso_path = if dir.is_empty() {
                    e.name.clone()
                } else {
                    format!("{dir}/{}", e.name)
                };
                out.push(DiscPayload {
                    iso_path,
                    kind,
                    size: e.size,
                });
            }
        }
    }

    /// Multi-disc hint: sibling files like `DISK2`, `DISC 2`, `_cd2`.
    pub fn disc_hints(&mut self) -> Vec<String> {
        let mut hints = Vec::new();
        for dir in ["", XBOX_DIR, GAME_DIR] {
            if let Ok(entries) = self.image.list_dir(dir) {
                for e in entries {
                    let n = e.name.to_ascii_lowercase();
                    if n.contains("disc") || n.contains("disk") || n.contains("cd2") {
                        hints.push(e.name);
                    }
                }
            }
        }
        hints
    }
}

/// Convenience: open + summarize a disc for the game runner.
pub fn inspect_iso(path: &Path) -> Result<IsoSummary, IsoError> {
    let mut src = IsoGameSource::open(path)?;
    let executables = src.find_executables();
    let shaders = src.find_shaders();
    let covers = src.find_cover_art();
    Ok(IsoSummary {
        volume_label: src.volume_label.clone(),
        executables,
        shader_count: shaders.len(),
        cover_count: covers.len(),
        disc_hints: src.disc_hints(),
    })
}

/// Summary returned by [`inspect_iso`].
#[derive(Debug)]
pub struct IsoSummary {
    pub volume_label: String,
    pub executables: Vec<DiscPayload>,
    pub shader_count: usize,
    pub cover_count: usize,
    pub disc_hints: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_iso() {
        let dir = std::env::temp_dir().join("xholy_isosrc_bad");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.iso");
        std::fs::write(&p, vec![0u8; 4096]).unwrap();
        assert!(IsoGameSource::open(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
