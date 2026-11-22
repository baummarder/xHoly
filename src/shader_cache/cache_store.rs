//! Cache store trait + memory impl — abstraction for disk/remote stores.

/// A store backend.
pub trait CacheStore {
    /// Fetch by key string.
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    /// Store under key.
    fn put(&mut self, key: &str, data: &[u8]);
    /// Remove.
    fn remove(&mut self, key: &str) -> bool;
    /// Number of entries.
    fn len(&self) -> usize;
    /// Empty? (default impl).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Clear all.
    fn clear(&mut self);
}

/// In-memory store (HashMap-backed).
#[derive(Debug, Default)]
pub struct MemoryStore {
    map: std::collections::HashMap<String, Vec<u8>>,
}

impl MemoryStore {
    /// New store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CacheStore for MemoryStore {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.map.get(key).cloned()
    }
    fn put(&mut self, key: &str, data: &[u8]) {
        self.map.insert(key.to_string(), data.to_vec());
    }
    fn remove(&mut self, key: &str) -> bool {
        self.map.remove(key).is_some()
    }
    fn len(&self) -> usize {
        self.map.len()
    }
    fn clear(&mut self) {
        self.map.clear();
    }
}

/// Filesystem store rooted at a directory.
#[derive(Debug, Clone)]
pub struct FsStore {
    root: std::path::PathBuf,
}

impl FsStore {
    /// New store at `root` (created if missing).
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        let root = root.into();
        let _ = std::fs::create_dir_all(&root);
        Self { root }
    }
    fn path(&self, key: &str) -> std::path::PathBuf {
        self.root.join(key)
    }
}

impl CacheStore for FsStore {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)).ok()
    }
    fn put(&mut self, key: &str, data: &[u8]) {
        let _ = std::fs::write(self.path(key), data);
    }
    fn remove(&mut self, key: &str) -> bool {
        std::fs::remove_file(self.path(key)).is_ok()
    }
    fn len(&self) -> usize {
        std::fs::read_dir(&self.root)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0)
    }
    fn clear(&mut self) {
        if let Ok(rd) = std::fs::read_dir(&self.root) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let mut s = MemoryStore::new();
        s.put("a", b"1");
        assert_eq!(s.get("a").unwrap(), b"1");
        assert!(s.remove("a"));
        assert!(s.get("a").is_none());
    }

    #[test]
    fn fs_store_roundtrip() {
        let dir = std::env::temp_dir().join("xholy_fsstore_test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = FsStore::new(&dir);
        s.put("k", b"v");
        assert_eq!(s.get("k").unwrap(), b"v");
        assert_eq!(s.len(), 1);
        s.clear();
        assert_eq!(s.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
