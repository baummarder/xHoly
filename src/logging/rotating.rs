//! Rotating log file — size/age/count based rotation.
//!
//! Keeps `name.log`, `name.log.1`, … `name.log.N`. Rolls when the
//! active file exceeds `max_bytes` or is older than `max_age`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Rotation policy.
#[derive(Debug, Clone)]
pub struct RotationPolicy {
    /// Max active-file size in bytes.
    pub max_bytes: u64,
    /// Max file age before rotation.
    pub max_age: Duration,
    /// Keep this many rotated files.
    pub max_files: usize,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024,
            max_age: Duration::from_secs(24 * 3600),
            max_files: 5,
        }
    }
}

/// A rotating append-only log file.
pub struct RotatingFile {
    path: PathBuf,
    policy: RotationPolicy,
    file: std::fs::File,
    written: u64,
    created: SystemTime,
}

impl RotatingFile {
    /// Open `path` (e.g. `logs/xholy.log`), creating parents.
    pub fn open(path: impl Into<PathBuf>, policy: RotationPolicy) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        let created = file
            .metadata()
            .and_then(|m| m.created())
            .unwrap_or_else(|_| SystemTime::now());
        Ok(Self {
            path,
            policy,
            file,
            written,
            created,
        })
    }

    /// Append a line, rotating first if needed.
    pub fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.maybe_rotate()?;
        let bytes = line.as_bytes();
        self.file.write_all(bytes)?;
        self.file.write_all(b"\n")?;
        self.written += bytes.len() as u64 + 1;
        Ok(())
    }

    fn maybe_rotate(&mut self) -> std::io::Result<()> {
        let size_hit = self.written >= self.policy.max_bytes;
        let age_hit = self.created.elapsed().unwrap_or_default() >= self.policy.max_age;
        if !size_hit && !age_hit {
            return Ok(());
        }
        self.rotate()?;
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        // Shift .N-1 → .N (delete oldest first).
        let oldest = rotated_path(&self.path, self.policy.max_files);
        if oldest.exists() {
            let _ = fs::remove_file(&oldest);
        }
        for i in (1..self.policy.max_files).rev() {
            let from = rotated_path(&self.path, i);
            let to = rotated_path(&self.path, i + 1);
            if from.exists() {
                fs::rename(&from, &to)?;
            }
        }
        let first = rotated_path(&self.path, 1);
        fs::rename(&self.path, &first)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        self.created = SystemTime::now();
        Ok(())
    }
}

fn rotated_path(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_on_size() {
        let dir = std::env::temp_dir().join("xholy_rotlog");
        let _ = fs::remove_dir_all(&dir);
        let p = dir.join("x.log");
        let pol = RotationPolicy {
            max_bytes: 100,
            max_age: Duration::from_secs(3600),
            max_files: 3,
        };
        let mut f = RotatingFile::open(&p, pol).unwrap();
        for i in 0..10 {
            f.write_line(&format!("line {i} with some padding text"))
                .unwrap();
        }
        assert!(rotated_path(&p, 1).exists());
        let cur = fs::read_to_string(&p).unwrap();
        assert!(cur.contains("line 9"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_files_bounded() {
        let dir = std::env::temp_dir().join("xholy_rotlog2");
        let _ = fs::remove_dir_all(&dir);
        let p = dir.join("x.log");
        let pol = RotationPolicy {
            max_bytes: 50,
            max_age: Duration::from_secs(3600),
            max_files: 2,
        };
        let mut f = RotatingFile::open(&p, pol).unwrap();
        for i in 0..20 {
            f.write_line(&format!("entry number {i}")).unwrap();
        }
        assert!(rotated_path(&p, 1).exists());
        assert!(!rotated_path(&p, 3).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
