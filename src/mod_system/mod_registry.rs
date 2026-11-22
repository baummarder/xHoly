//! Mod registry — local catalog + remote-repo stub.

use std::collections::BTreeMap;

use super::mod_loader::LoadedMod;
use super::mod_manifest::{ModManifest, Version};

/// Registry errors.
#[derive(Debug)]
pub enum RegistryError {
    /// Duplicate mod id.
    Duplicate(String),
    /// Unknown mod id.
    Unknown(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(f, "duplicate mod id: {id}"),
            Self::Unknown(id) => write!(f, "unknown mod id: {id}"),
        }
    }
}
impl std::error::Error for RegistryError {}

/// Local mod registry (in-memory catalog).
#[derive(Debug, Default)]
pub struct ModRegistry {
    mods: BTreeMap<String, LoadedMod>,
}

impl ModRegistry {
    /// New empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a loaded mod.
    pub fn register(&mut self, m: LoadedMod) -> Result<(), RegistryError> {
        let id = m.manifest.id.clone();
        if self.mods.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.mods.insert(id, m);
        Ok(())
    }

    /// Remove a mod.
    pub fn unregister(&mut self, id: &str) -> Result<LoadedMod, RegistryError> {
        self.mods
            .remove(id)
            .ok_or_else(|| RegistryError::Unknown(id.into()))
    }

    /// Look up a mod's manifest.
    pub fn get(&self, id: &str) -> Option<&ModManifest> {
        self.mods.get(id).map(|m| &m.manifest)
    }

    /// All registered ids (sorted).
    pub fn ids(&self) -> Vec<&str> {
        self.mods.keys().map(|s| s.as_str()).collect()
    }

    /// Count.
    pub fn len(&self) -> usize {
        self.mods.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    /// Remote-repo stub: "fetch" returns manifests discovered in a
    /// directory listing (same format as local). Kept synchronous —
    /// real HTTP fetch lives in `network`/`cloud`.
    pub fn fetch_repo_index(dir: &std::path::Path) -> Vec<ModManifest> {
        super::mod_loader::scan_dir(dir)
            .into_iter()
            .filter_map(|r| r.ok().map(|m| m.manifest))
            .collect()
    }

    /// Find the newest version among candidates satisfying `min`.
    pub fn best_version<'a>(
        candidates: impl Iterator<Item = &'a ModManifest>,
        min: Version,
    ) -> Option<&'a ModManifest> {
        candidates
            .filter(|m| m.version >= min)
            .max_by_key(|m| m.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_system::mod_loader::MANIFEST_NAME;

    fn mk_mod(root: &std::path::Path, id: &str, v: (u32, u32, u32)) -> LoadedMod {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_NAME),
            format!(
                "id=\"{id}\"\nname=\"{id}\"\nversion={{major={},minor={},patch={}}}",
                v.0, v.1, v.2
            ),
        )
        .unwrap();
        crate::mod_system::mod_loader::load_mod_dir(&dir).unwrap()
    }

    #[test]
    fn register_unregistered() {
        let root = std::env::temp_dir().join("xholy_reg");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut reg = ModRegistry::new();
        let m = mk_mod(&root, "m1", (1, 0, 0));
        assert!(reg.register(m).is_ok());
        assert!(matches!(
            reg.register(crate::mod_system::mod_loader::load_mod_dir(&root.join("m1")).unwrap()),
            Err(RegistryError::Duplicate(_))
        ));
        assert_eq!(reg.len(), 1);
        assert!(reg.unregister("m1").is_ok());
        assert!(reg.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn best_version_picks_newest_compatible() {
        let m1 = ModManifest {
            id: "x".into(),
            name: "x".into(),
            version: Version::new(1, 0, 0),
            authors: vec![],
            description: String::new(),
            dependencies: vec![],
            entry: None,
        };
        let m2 = ModManifest {
            version: Version::new(1, 2, 0),
            ..m1.clone()
        };
        let best = ModRegistry::best_version([&m1, &m2].into_iter(), Version::new(1, 1, 0));
        assert_eq!(best.unwrap().version, Version::new(1, 2, 0));
    }
}
