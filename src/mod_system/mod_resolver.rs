//! Dependency resolver — order mods so deps load first.

use std::collections::{HashMap, HashSet};

use super::mod_manifest::{ModManifest, Version};

/// A mod selected for loading.
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    pub manifest: ModManifest,
    /// Depth in the dependency chain (0 = no deps).
    pub depth: usize,
}

/// Resolution errors.
#[derive(Debug, PartialEq)]
pub enum ResolutionError {
    /// Required dep missing (hard dep).
    Missing(String, String), // (mod id, dep id)
    /// Version requirement unmet.
    VersionTooOld(String, Version, Version), // (dep id, have, need)
    /// Dependency cycle detected.
    Cycle(Vec<String>),
}

impl std::fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(m, d) => write!(f, "mod '{m}' requires missing dep '{d}'"),
            Self::VersionTooOld(d, have, need) => {
                write!(f, "dep '{d}' has {have}, needs >= {need}")
            }
            Self::Cycle(c) => write!(f, "dependency cycle: {}", c.join(" -> ")),
        }
    }
}
impl std::error::Error for ResolutionError {}

/// Resolve `roots` + their deps into a load order (deps first).
/// `available` is the full candidate pool (id → manifest); `roots`
/// are the ids the user wants.
pub fn resolve(
    roots: &[String],
    available: &HashMap<String, ModManifest>,
) -> Result<Vec<ResolvedMod>, ResolutionError> {
    let mut order = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();

    fn visit(
        id: &str,
        available: &HashMap<String, ModManifest>,
        order: &mut Vec<ResolvedMod>,
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), ResolutionError> {
        if visited.contains(id) {
            return Ok(());
        }
        if stack.iter().any(|s| s == id) {
            let mut cyc = stack.clone();
            cyc.push(id.to_string());
            return Err(ResolutionError::Cycle(cyc));
        }
        let m = available
            .get(id)
            .ok_or_else(|| ResolutionError::Missing(id.into(), id.into()))?;
        stack.push(id.to_string());
        for dep in &m.dependencies {
            if dep.optional && !available.contains_key(&dep.id) {
                continue; // soft dep missing → skip
            }
            let dep_m = available
                .get(&dep.id)
                .ok_or_else(|| ResolutionError::Missing(m.id.clone(), dep.id.clone()))?;
            if dep_m.version < dep.min_version {
                return Err(ResolutionError::VersionTooOld(
                    dep.id.clone(),
                    dep_m.version,
                    dep.min_version,
                ));
            }
            visit(&dep.id, available, order, visited, stack)?;
        }
        stack.pop();
        visited.insert(id.to_string());
        order.push(ResolvedMod {
            manifest: m.clone(),
            depth: stack.len(),
        });
        Ok(())
    }

    for r in roots {
        visit(r, available, &mut order, &mut visited, &mut stack)?;
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_system::mod_manifest::ModDependency;

    fn man(id: &str, deps: Vec<(&str, (u32, u32, u32))>) -> ModManifest {
        ModManifest {
            id: id.into(),
            name: id.into(),
            version: Version::new(1, 0, 0),
            authors: vec![],
            description: String::new(),
            dependencies: deps
                .into_iter()
                .map(|(d, v)| ModDependency {
                    id: d.into(),
                    min_version: Version::new(v.0, v.1, v.2),
                    optional: false,
                })
                .collect(),
            entry: None,
        }
    }

    fn pool(ms: Vec<ModManifest>) -> HashMap<String, ModManifest> {
        ms.into_iter().map(|m| (m.id.clone(), m)).collect()
    }

    #[test]
    fn deps_load_first() {
        let p = pool(vec![
            man("app", vec![("lib", (1, 0, 0))]),
            man("lib", vec![]),
        ]);
        let order = resolve(&["app".into()], &p).unwrap();
        assert_eq!(order[0].manifest.id, "lib");
        assert_eq!(order[1].manifest.id, "app");
    }

    #[test]
    fn missing_hard_dep_errors() {
        let p = pool(vec![man("app", vec![("ghost", (1, 0, 0))])]);
        assert!(matches!(
            resolve(&["app".into()], &p),
            Err(ResolutionError::Missing(_, _))
        ));
    }

    #[test]
    fn version_too_old_errors() {
        let mut dep = man("lib", vec![]);
        dep.version = Version::new(0, 9, 0);
        let p = pool(vec![man("app", vec![("lib", (1, 0, 0))]), dep]);
        assert!(matches!(
            resolve(&["app".into()], &p),
            Err(ResolutionError::VersionTooOld(..))
        ));
    }

    #[test]
    fn cycle_detected() {
        let p = pool(vec![
            man("a", vec![("b", (1, 0, 0))]),
            man("b", vec![("a", (1, 0, 0))]),
        ]);
        assert!(matches!(
            resolve(&["a".into()], &p),
            Err(ResolutionError::Cycle(_))
        ));
    }
}
