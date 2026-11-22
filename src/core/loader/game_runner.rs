//! Game execution pipeline — Phase 10.
//!
//! Ties the Phase-10 pieces together:
//!
//! ```text
//! ISO/directory/file
//!   → iso_parser / loader::load_game        (image + metadata)
//!   → xex_parser                            (header fields)
//!   → shader::compile_shader                (asset shaders → WGSL/SPV)
//!   → kernel + interpreter                  (execution.rs)
//! ```
//!
//! [`run_iso`] is the ISO entry point; [`run_path`] auto-detects
//! ISO vs. flat image vs. directory.

use std::path::{Path, PathBuf};

use super::game_image::{AssetCatalog, AssetEntry, DiscInfo, GameRegion, ParsedGameImage};
use super::iso_parser::IsoGameSource;
use super::{load_game, GameImage};
use crate::core::execution::{execute_game, RunReport};
use crate::core::kernel::USER_BASE;
use crate::core::shader::{compile_shader, GameShaderCache};

/// Errors from the runner pipeline.
#[derive(Debug)]
pub enum RunError {
    /// ISO open/list failure.
    Iso(crate::optical::iso_parser::IsoError),
    /// Loader failure.
    Load(super::LoadError),
    /// No executable found on the disc.
    NoExecutable(String),
    /// Guest execution failure.
    Exec(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Iso(e) => write!(f, "iso: {e}"),
            Self::Load(e) => write!(f, "load: {e}"),
            Self::NoExecutable(p) => write!(f, "no executable on {p}"),
            Self::Exec(e) => write!(f, "exec: {e}"),
        }
    }
}
impl std::error::Error for RunError {}
impl From<crate::optical::iso_parser::IsoError> for RunError {
    fn from(e: crate::optical::iso_parser::IsoError) -> Self {
        Self::Iso(e)
    }
}
impl From<super::LoadError> for RunError {
    fn from(e: super::LoadError) -> Self {
        Self::Load(e)
    }
}

/// What the runner produced.
#[derive(Debug)]
pub struct RunOutcome {
    /// Execution report (steps, output, kernel log).
    pub report: RunReport,
    /// Parsed game metadata.
    pub game: ParsedGameImage,
    /// Shader compilation results (key → warnings) for assets found.
    pub shader_results: Vec<(String, Vec<String>)>,
}

/// Run a game from any supported source: ISO image, directory, or
/// flat/XEX file.
pub fn run_path(path: &Path, max_steps: u64) -> Result<RunOutcome, RunError> {
    // ISO detection: try the ISO reader first when the file is large
    // enough to hold a PVD; fall back to flat loading on failure.
    if path.is_file() && is_probably_iso(path) {
        if let Ok(outcome) = run_iso(path, max_steps) {
            return Ok(outcome);
        }
    }
    run_flat(path, max_steps)
}

/// Heuristic: ISO9660 needs ≥ ~34 sectors; smaller files can't be discs.
fn is_probably_iso(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > 34 * 2048)
        .unwrap_or(false)
}

/// Run a game directly from an ISO9660 image.
pub fn run_iso(iso_path: &Path, max_steps: u64) -> Result<RunOutcome, RunError> {
    let mut src = IsoGameSource::open(iso_path)?;

    // Executable: prefer XBOX/DEFAULT.XEX, else first .xex found.
    let execs = src.find_executables();
    let exec = execs
        .iter()
        .find(|e| e.iso_path.to_ascii_lowercase().contains("default"))
        .or_else(|| execs.first())
        .ok_or_else(|| RunError::NoExecutable(iso_path.display().to_string()))?;

    let bytes = src.read_file(&exec.iso_path)?;
    let game_image = GameImage {
        title: src.volume_label.clone(),
        title_id: None,
        load_base: USER_BASE,
        entry: USER_BASE,
        image: bytes,
        source: PathBuf::from(&exec.iso_path),
        metadata: Default::default(),
    };

    // Asset catalog from the disc scan.
    let mut assets = AssetCatalog::default();
    for s in src.find_shaders() {
        assets.entries.push(AssetEntry {
            path: s.iso_path.clone(),
            kind: "shader".into(),
            size: s.size as u64,
        });
    }
    for c in src.find_cover_art() {
        assets.entries.push(AssetEntry {
            path: c.iso_path.clone(),
            kind: "texture".into(),
            size: c.size as u64,
        });
    }

    let parsed = ParsedGameImage {
        title: src.volume_label.clone(),
        region: GameRegion::Other,
        discs: vec![DiscInfo {
            number: 1,
            source: iso_path.display().to_string(),
            has_executable: true,
        }],
        assets,
        entry_point: USER_BASE,
        image_base: USER_BASE,
        title_id: None,
        release_date: None,
    };

    // Compile discovered shader blobs through the Phase-10 pipeline.
    let cache = GameShaderCache::open_default().map_err(RunError::Exec)?;
    let mut shader_results = Vec::new();
    for entry in parsed.assets.shaders().take(8) {
        if let Ok(blob) = src.read_file(&entry.path) {
            match compile_shader(&cache, &blob, b"") {
                Ok(art) => shader_results.push((entry.path.clone(), art.warnings)),
                Err(e) => shader_results.push((entry.path.clone(), vec![e])),
            }
        }
    }

    let report = execute_game(&game_image, max_steps).map_err(|e| RunError::Exec(e.to_string()))?;
    Ok(RunOutcome {
        report,
        game: parsed,
        shader_results,
    })
}

/// Run from a flat image / directory (Phase-8 path, wrapped).
fn run_flat(path: &Path, max_steps: u64) -> Result<RunOutcome, RunError> {
    let game_image: GameImage = load_game(path)?;
    let parsed = ParsedGameImage {
        title: game_image.title.clone(),
        region: GameRegion::Other,
        discs: vec![DiscInfo {
            number: 1,
            source: path.display().to_string(),
            has_executable: true,
        }],
        assets: AssetCatalog::default(),
        entry_point: game_image.entry,
        image_base: game_image.load_base,
        title_id: game_image.title_id.clone(),
        release_date: game_image.metadata.release_date.clone(),
    };
    let report = execute_game(&game_image, max_steps).map_err(|e| RunError::Exec(e.to_string()))?;
    Ok(RunOutcome {
        report,
        game: parsed,
        shader_results: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_path_runs() {
        let dir = std::env::temp_dir().join("xholy_runner_flat");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("mini.bin");
        // MOVZ x0,#0 ; SVC #0 (write, len 0) ; MOVZ x0,#7 ; SVC #0 ; BRK
        let words: [u32; 5] = [
            0xD2800000,            // MOVZ x0, #0
            0xD4000001,            // SVC #0
            0xD2800000 | (7 << 5), // MOVZ x0, #7
            0xD280000E,            // placeholder
            0xD4200000,            // BRK
        ];
        let mut bytes = Vec::new();
        for w in words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        std::fs::write(&bin, &bytes).unwrap();
        let out = run_path(&bin, 100).unwrap();
        assert!(out.report.steps > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
