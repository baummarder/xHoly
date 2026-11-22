//! Mod loading — read `xholy.mod.toml` from a directory.

use std::path::{Path, PathBuf};

use super::mod_manifest::ModManifest;

/// Errors loading a mod.
#[derive(Debug)]
pub enum ModLoadError {
    /// Manifest missing.
    NoManifest(PathBuf),
    /// IO error.
    Io(std::io::Error),
    /// TOML parse error.
    Parse(String),
}

impl std::fmt::Display for ModLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoManifest(p) => write!(f, "no xholy.mod.toml in {}", p.display()),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(e) => write!(f, "manifest parse: {e}"),
        }
    }
}
impl std::error::Error for ModLoadError {}

/// A loaded mod: manifest + its directory.
#[derive(Debug, Clone)]
pub struct LoadedMod {
    pub manifest: ModManifest,
    /// Directory containing the mod (manifest lives at its root).
    pub dir: PathBuf,
}

/// Manifest filename.
pub const MANIFEST_NAME: &str = "xholy.mod.toml";

/// Load a mod from `dir` (expects `xholy.mod.toml` inside).
pub fn load_mod_dir(dir: &Path) -> Result<LoadedMod, ModLoadError> {
    let path = dir.join(MANIFEST_NAME);
    if !path.is_file() {
        return Err(ModLoadError::NoManifest(dir.to_path_buf()));
    }
    load_manifest_file(&path).map(|manifest| LoadedMod {
        manifest,
        dir: dir.to_path_buf(),
    })
}

/// Load a manifest from an explicit file path.
pub fn load_manifest_file(path: &Path) -> Result<ModManifest, ModLoadError> {
    let text = std::fs::read_to_string(path).map_err(ModLoadError::Io)?;
    toml::from_str(&text).map_err(|e| ModLoadError::Parse(e.to_string()))
}

/// Scan `root` for subdirectories containing manifests (one level deep,
/// plus root itself if it has one).
pub fn scan_dir(root: &Path) -> Vec<Result<LoadedMod, ModLoadError>> {
    let mut out = Vec::new();
    if root.join(MANIFEST_NAME).is_file() {
        out.push(load_mod_dir(root));
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join(MANIFEST_NAME).is_file() {
                out.push(load_mod_dir(&p));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_manifest() {
        let dir = std::env::temp_dir().join("xholy_modload");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_NAME),
            r#"id = "test.mod"
name = "Test"
version = { major = 1, minor = 0, patch = 0 }"#,
        )
        .unwrap();
        let m = load_mod_dir(&dir).unwrap();
        assert_eq!(m.manifest.id, "test.mod");
        assert_eq!(m.dir, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_manifest_errors() {
        let dir = std::env::temp_dir().join("xholy_modnone");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            load_mod_dir(&dir),
            Err(ModLoadError::NoManifest(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_finds_mods() {
        let root = std::env::temp_dir().join("xholy_modscan");
        let _ = std::fs::remove_dir_all(&root);
        let m1 = root.join("modA");
        std::fs::create_dir_all(&m1).unwrap();
        std::fs::write(
            m1.join(MANIFEST_NAME),
            r#"id="a"
name="A"
version={major=0,minor=1,patch=0}"#,
        )
        .unwrap();
        let found = scan_dir(&root);
        assert_eq!(found.len(), 1);
        assert!(found[0].is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}
