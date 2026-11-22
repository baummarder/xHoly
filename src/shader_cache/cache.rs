//! Shader cache — keyed blob store with FNV-1a hashing.

use std::collections::HashMap;
use std::path::PathBuf;

use super::cache_manager::CacheStats;

/// FNV-1a 64-bit hash.
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Cache key: hash of (source bytes + binding-layout descriptor).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Content hash.
    pub hash: u64,
    /// Format tag (e.g. "dxbc", "dxil", "wgsl", "spirv").
    pub format: String,
}

impl CacheKey {
    /// Build key from source bytes + format + optional layout salt.
    pub fn from_source(source: &[u8], format: &str, layout_salt: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(source.len() + layout_salt.len() + 8);
        buf.extend_from_slice(source);
        buf.extend_from_slice(layout_salt);
        buf.extend_from_slice(&(source.len() as u64).to_le_bytes());
        Self {
            hash: fnv1a(&buf),
            format: format.to_string(),
        }
    }

    /// Filesystem-safe filename.
    pub fn filename(&self) -> String {
        format!("{}_{:016x}.bin", sanitize(&self.format), self.hash)
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// The shader cache: memory store + optional disk dir.
pub struct ShaderCache {
    /// In-memory entries: key-string → bytes.
    pub memory: HashMap<String, Vec<u8>>,
    /// Disk directory (None = memory only).
    pub dir: Option<PathBuf>,
    /// Stats.
    pub stats: CacheStats,
}

impl ShaderCache {
    /// New memory-only cache.
    pub fn new() -> Self {
        Self {
            memory: HashMap::new(),
            dir: None,
            stats: CacheStats::default(),
        }
    }

    /// New cache persisted under `dir`.
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self {
            memory: HashMap::new(),
            dir: Some(dir),
            stats: CacheStats::default(),
        }
    }

    /// Lookup: memory first, then disk.
    pub fn get(&mut self, key: &CacheKey) -> Option<Vec<u8>> {
        let k = key.filename();
        if let Some(v) = self.memory.get(&k) {
            self.stats.hits += 1;
            return Some(v.clone());
        }
        if let Some(dir) = &self.dir {
            let p = dir.join(&k);
            if let Ok(b) = std::fs::read(&p) {
                self.memory.insert(k.clone(), b.clone());
                self.stats.hits += 1;
                self.stats.disk_hits += 1;
                return Some(b);
            }
        }
        self.stats.misses += 1;
        None
    }

    /// Store compiled artifact.
    pub fn put(&mut self, key: &CacheKey, data: &[u8]) {
        let k = key.filename();
        self.memory.insert(k.clone(), data.to_vec());
        if let Some(dir) = &self.dir {
            let _ = std::fs::write(dir.join(&k), data);
        }
        self.stats.stores += 1;
    }

    /// Contains?
    pub fn contains(&self, key: &CacheKey) -> bool {
        self.memory.contains_key(&key.filename())
    }

    /// Number of in-memory entries.
    pub fn len(&self) -> usize {
        self.memory.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.memory.is_empty()
    }

    /// Clear memory (disk untouched).
    pub fn clear(&mut self) {
        self.memory.clear();
    }

    /// Wipe disk dir too.
    pub fn clear_all(&mut self) {
        self.clear();
        if let Some(dir) = &self.dir {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

impl Default for ShaderCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known() {
        // FNV-1a of empty = offset basis
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        // Deterministic
        assert_eq!(fnv1a(b"abc"), fnv1a(b"abc"));
        assert_ne!(fnv1a(b"abc"), fnv1a(b"abd"));
    }

    #[test]
    fn key_filename_safe() {
        let k = CacheKey::from_source(b"shader", "dxbc", b"");
        let f = k.filename();
        assert!(f.starts_with("dxbc_"));
        assert!(f.ends_with(".bin"));
        assert!(!f.contains('/'));
    }

    #[test]
    fn cache_hit_miss() {
        let mut c = ShaderCache::new();
        let k = CacheKey::from_source(b"vs-code", "dxil", b"layout1");
        assert!(c.get(&k).is_none());
        c.put(&k, b"spirv-words");
        assert_eq!(c.get(&k).unwrap(), b"spirv-words");
        assert_eq!(c.stats.hits, 1);
        assert_eq!(c.stats.misses, 1);
    }

    #[test]
    fn disk_persistence() {
        let dir = std::env::temp_dir().join("xholy_shadercache_test");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut c = ShaderCache::with_dir(&dir);
            let k = CacheKey::from_source(b"fs", "dxbc", b"");
            c.put(&k, b"wgsl-out");
        }
        {
            let mut c = ShaderCache::with_dir(&dir);
            let k = CacheKey::from_source(b"fs", "dxbc", b"");
            assert_eq!(c.get(&k).unwrap(), b"wgsl-out");
            assert_eq!(c.stats.disk_hits, 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
