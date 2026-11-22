//! Mod manifest — `xholy.mod.toml` schema + semver-ish Version.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic-ish version (major.minor.patch, no pre-release).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// New version.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
    /// Parse "1.2.3".
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.split('.');
        Some(Self {
            major: it.next()?.parse().ok()?,
            minor: it.next()?.parse().ok()?,
            patch: it.next()?.parse().ok()?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A declared dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDependency {
    /// Mod id required.
    pub id: String,
    /// Minimum version (inclusive).
    pub min_version: Version,
    /// Optional: mark as soft dep (missing = warning, not error).
    #[serde(default)]
    pub optional: bool,
}

/// `xholy.mod.toml` contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    /// Unique mod id (reverse-domain or slug).
    pub id: String,
    /// Human name.
    pub name: String,
    /// Version.
    pub version: Version,
    /// Author(s).
    #[serde(default)]
    pub authors: Vec<String>,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Hard dependencies.
    #[serde(default)]
    pub dependencies: Vec<ModDependency>,
    /// Entry callback name the executor looks up.
    #[serde(default)]
    pub entry: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_order() {
        let a = Version::parse("1.2.3").unwrap();
        let b = Version::parse("1.10.0").unwrap();
        assert!(b > a);
        assert_eq!(a.to_string(), "1.2.3");
    }

    #[test]
    fn manifest_roundtrip() {
        let m = ModManifest {
            id: "com.example.mod".into(),
            name: "Example".into(),
            version: Version::new(0, 1, 0),
            authors: vec!["dev".into()],
            description: "d".into(),
            dependencies: vec![ModDependency {
                id: "dep".into(),
                min_version: Version::new(1, 0, 0),
                optional: false,
            }],
            entry: Some("init".into()),
        };
        let s = toml::to_string(&m).unwrap();
        let back: ModManifest = toml::from_str(&s).unwrap();
        assert_eq!(back.id, "com.example.mod");
        assert_eq!(back.dependencies[0].id, "dep");
    }
}
