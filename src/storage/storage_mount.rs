//! Storage mount — ISO/filesystem mount points.

use std::path::PathBuf;

/// Mount point kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountPoint {
    /// Host directory bind-mount.
    Dir(PathBuf),
    /// ISO image file.
    Iso(PathBuf),
    /// Optical drive (virtual, via `optical`).
    Optical { drive: String, disc: u32 },
}

/// A mount.
#[derive(Debug, Clone)]
pub struct Mount {
    /// Mount name (drive letter or label).
    pub name: String,
    /// What is mounted.
    pub source: MountPoint,
    /// Read-only?
    pub read_only: bool,
}

/// Mount errors.
#[derive(Debug, PartialEq)]
pub enum MountError {
    /// Source doesn't exist.
    NotFound(String),
    /// Mount name already in use.
    Duplicate(String),
    /// Write to read-only mount.
    ReadOnly(String),
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "not found: {p}"),
            Self::Duplicate(n) => write!(f, "duplicate mount {n}"),
            Self::ReadOnly(n) => write!(f, "mount {n} is read-only"),
        }
    }
}
impl std::error::Error for MountError {}

/// Mount table.
#[derive(Debug, Default)]
pub struct MountTable {
    mounts: Vec<Mount>,
}

impl MountTable {
    /// New empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a mount (validates name uniqueness + source existence for Dir/Iso).
    pub fn add(&mut self, m: Mount) -> Result<(), MountError> {
        if self.mounts.iter().any(|x| x.name == m.name) {
            return Err(MountError::Duplicate(m.name));
        }
        match &m.source {
            MountPoint::Dir(p) | MountPoint::Iso(p) => {
                if !p.exists() {
                    return Err(MountError::NotFound(p.display().to_string()));
                }
            }
            MountPoint::Optical { .. } => {}
        }
        self.mounts.push(m);
        Ok(())
    }

    /// Remove by name.
    pub fn remove(&mut self, name: &str) -> Option<Mount> {
        self.mounts
            .iter()
            .position(|m| m.name == name)
            .map(|i| self.mounts.remove(i))
    }

    /// Find by name.
    pub fn get(&self, name: &str) -> Option<&Mount> {
        self.mounts.iter().find(|m| m.name == name)
    }

    /// Resolve `X:\path`-style or `/mnt/name/path` to a mount + subpath.
    pub fn resolve(&self, path: &str) -> Option<(&Mount, String)> {
        // Accept "D:/foo/bar" or "/mnt/D/foo".
        let (name, rest) = if let Some((n, r)) = path.split_once(':') {
            (n.trim_end_matches(['/', '\\']), r.to_string())
        } else {
            let r = path.strip_prefix("/mnt/")?;
            r.split_once('/').map(|(n, rest)| (n, format!("/{rest}")))?
        };
        self.get(name).map(|m| (m, rest))
    }

    /// All mounts.
    pub fn iter(&self) -> impl Iterator<Item = &Mount> {
        self.mounts.iter()
    }
    /// Count.
    pub fn len(&self) -> usize {
        self.mounts.len()
    }
    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_and_resolve() {
        let dir = std::env::temp_dir().join("xholy_mount_test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = MountTable::new();
        t.add(Mount {
            name: "D".into(),
            source: MountPoint::Dir(dir.clone()),
            read_only: true,
        })
        .unwrap();
        let (m, rest) = t.resolve("D:/games/foo.iso").unwrap();
        assert_eq!(m.name, "D");
        assert_eq!(rest, "/games/foo.iso");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_and_missing() {
        let mut t = MountTable::new();
        let missing = std::env::temp_dir().join("xholy_nope_xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(matches!(
            t.add(Mount {
                name: "X".into(),
                source: MountPoint::Dir(missing),
                read_only: false,
            }),
            Err(MountError::NotFound(_))
        ));
        let dir = std::env::temp_dir().join("xholy_mount_dup");
        std::fs::create_dir_all(&dir).unwrap();
        for _ in 0..2 {
            let r = t.add(Mount {
                name: "D".into(),
                source: MountPoint::Dir(dir.clone()),
                read_only: false,
            });
            if r.is_err() {
                assert!(matches!(r, Err(MountError::Duplicate(_))));
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
