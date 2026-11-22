//! Cloud cache — local LRU-ish cache of remote objects.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// A cached remote object.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// Key (URL or save id).
    pub key: String,
    /// Payload bytes.
    pub data: Vec<u8>,
    /// Insertion time (unix secs).
    pub inserted_at: u64,
    /// Last access time.
    pub accessed_at: u64,
    /// Hit count.
    pub hits: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// In-memory + optional file-backed cloud cache.
#[derive(Debug, Default)]
pub struct CloudCache {
    entries: BTreeMap<String, CacheEntry>,
    /// Max total bytes (0 = unlimited).
    pub max_bytes: usize,
    /// TTL seconds (0 = never expire).
    pub ttl_secs: u64,
}

impl CloudCache {
    /// New cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get (marks access; checks TTL).
    pub fn get(&mut self, key: &str) -> Option<&[u8]> {
        let now = now();
        // Check expiry first (immutable borrow), then mutate.
        let expired = self
            .entries
            .get(key)
            .map(|e| self.ttl_secs > 0 && now.saturating_sub(e.inserted_at) > self.ttl_secs)
            .unwrap_or(false);
        if expired {
            self.entries.remove(key);
            return None;
        }
        let e = self.entries.get_mut(key)?;
        e.accessed_at = now;
        e.hits += 1;
        Some(&e.data)
    }

    /// Insert/replace.
    pub fn put(&mut self, key: impl Into<String>, data: Vec<u8>) {
        let key = key.into();
        let t = now();
        self.entries.insert(
            key.clone(),
            CacheEntry {
                key,
                data,
                inserted_at: t,
                accessed_at: t,
                hits: 0,
            },
        );
        self.evict_if_needed();
    }

    /// Remove.
    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.entries.remove(key).map(|e| e.data)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes cached.
    pub fn bytes(&self) -> usize {
        self.entries.values().map(|e| e.data.len()).sum()
    }

    /// Evict oldest-accessed until under max_bytes.
    fn evict_if_needed(&mut self) {
        if self.max_bytes == 0 {
            return;
        }
        while self.bytes() > self.max_bytes && !self.entries.is_empty() {
            // Find LRU key.
            let lru = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.accessed_at)
                .map(|(k, _)| k.clone());
            if let Some(k) = lru {
                self.entries.remove(&k);
            } else {
                break;
            }
        }
    }

    /// Clear all.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_hit() {
        let mut c = CloudCache::new();
        c.put("k1", vec![1, 2, 3]);
        assert_eq!(c.get("k1"), Some(&[1, 2, 3][..]));
        assert_eq!(c.get("k1"), Some(&[1, 2, 3][..])); // hit twice
        let e = c.entries.get("k1").unwrap();
        assert_eq!(e.hits, 2);
    }

    #[test]
    fn lru_eviction() {
        let mut c = CloudCache::new();
        c.max_bytes = 10;
        c.put("a", vec![0; 6]);
        c.put("b", vec![0; 6]); // total 12 > 10 → evict a
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some());
    }

    #[test]
    fn ttl_expiry() {
        let mut c = CloudCache::new();
        c.ttl_secs = 1;
        c.put("x", vec![9]);
        // Can't wait 1s in test — manipulate inserted_at.
        c.entries.get_mut("x").unwrap().inserted_at = 0;
        assert!(c.get("x").is_none());
    }
}
