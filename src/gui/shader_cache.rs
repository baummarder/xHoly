//! Shader cache — Phase 9.
//!
//! WGSL sources live in `shaders/` and are compiled by naga inside
//! wgpu at pipeline-creation time; wgpu does not expose a portable
//! binary shader cache, so what we *can* persist is a validated
//! on-disk copy of every shader plus a manifest — warm starts skip
//! re-reading the source tree and corrupted entries are detected.
//!
//! Cache root: `%LOCALAPPDATA%\xHoly\cache\shaders\` (Windows) or
//! `$XDG_CACHE_HOME/xholy/shaders/` (else `~/.cache/xholy/shaders/`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A named shader source embedded at compile time.
pub struct ShaderSource {
    /// File name inside the cache dir (e.g. `gui.wgsl`).
    pub name: &'static str,
    /// WGSL source text.
    pub source: &'static str,
}

/// All shaders the GUI pipeline needs.
pub const KNOWN_SHADERS: &[ShaderSource] = &[
    ShaderSource {
        name: "gui.wgsl",
        source: include_str!("../../shaders/gui.wgsl"),
    },
    ShaderSource {
        name: "postfx.wgsl",
        source: include_str!("../../shaders/postfx.wgsl"),
    },
];

/// Manifest entry: name → content hash (FNV-1a, stable across runs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    pub hash: u64,
    pub bytes: usize,
}

/// On-disk shader cache.
pub struct ShaderCache {
    dir: PathBuf,
    manifest: HashMap<String, ManifestEntry>,
}

impl ShaderCache {
    /// Open (or create) the cache at its default location.
    pub fn open_default() -> Result<Self, String> {
        Self::open(&crate::gui::textures::cache_root().join("shaders"))
    }

    /// Open/create a cache rooted at `dir`.
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("shader cache dir: {e}"))?;
        let manifest_path = dir.join("manifest.json");
        let manifest: HashMap<String, ManifestEntry> = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    /// Ensure every known shader is present + hash-verified; returns
    /// the number of files (re)written this call.
    pub fn sync(&mut self) -> Result<usize, String> {
        let mut written = 0;
        for s in KNOWN_SHADERS {
            let hash = fnv1a(s.source.as_bytes());
            let path = self.dir.join(s.name);
            let needs_write = match fs::read(&path) {
                Ok(existing) => fnv1a(&existing) != hash,
                Err(_) => true,
            };
            // Also honor the manifest even if bytes match (defensive).
            let stale_manifest = self
                .manifest
                .get(s.name)
                .map(|e| e.hash != hash)
                .unwrap_or(true);
            if needs_write || stale_manifest {
                fs::write(&path, s.source).map_err(|e| format!("write {}: {e}", path.display()))?;
                written += 1;
            }
            self.manifest.insert(
                s.name.to_string(),
                ManifestEntry {
                    hash,
                    bytes: s.source.len(),
                },
            );
        }
        self.flush()?;
        Ok(written)
    }

    /// Read a cached shader's source (verifies hash when manifest exists).
    pub fn read(&self, name: &str) -> Result<String, String> {
        let path = self.dir.join(name);
        let bytes = fs::read(&path).map_err(|e| format!("read {name}: {e}"))?;
        if let Some(e) = self.manifest.get(name) {
            if fnv1a(&bytes) != e.hash {
                return Err(format!("shader cache hash mismatch for {name}"));
            }
        }
        String::from_utf8(bytes).map_err(|e| format!("shader {name} not utf8: {e}"))
    }

    /// Persist the manifest.
    pub fn flush(&self) -> Result<(), String> {
        let path = self.dir.join("manifest.json");
        let json = serde_json::to_string_pretty(&self.manifest)
            .map_err(|e| format!("manifest serialize: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("write manifest: {e}"))
    }
}

/// FNV-1a 64-bit — dependency-free stable hash for cache validation.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_and_read_roundtrip() {
        let dir = std::env::temp_dir().join("xholy_shader_cache_test");
        let _ = fs::remove_dir_all(&dir);
        let mut cache = ShaderCache::open(&dir).unwrap();
        let n = cache.sync().unwrap();
        assert_eq!(n, KNOWN_SHADERS.len());
        // Second sync writes nothing.
        assert_eq!(cache.sync().unwrap(), 0);
        let src = cache.read("postfx.wgsl").unwrap();
        assert!(src.contains("composite_fs"));
        // Tamper → hash mismatch detected.
        fs::write(dir.join("postfx.wgsl"), "garbage").unwrap();
        assert!(cache.read("postfx.wgsl").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
