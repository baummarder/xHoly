//! Cache manager — size limits, LRU eviction, stats.

use std::collections::HashMap;

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Successful lookups.
    pub hits: u64,
    /// Misses.
    pub misses: u64,
    /// Stores.
    pub stores: u64,
    /// Evictions.
    pub evictions: u64,
    /// Hits served from disk.
    pub disk_hits: u64,
}

impl CacheStats {
    /// Hit ratio (0..1).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Managed cache entry metadata.
#[derive(Debug, Clone)]
struct Meta {
    size: usize,
    last_used: u64,
}

/// LRU-ish manager wrapping a byte store.
#[derive(Debug)]
pub struct CacheManager<S> {
    store: S,
    meta: HashMap<String, Meta>,
    /// Max total bytes (0 = unlimited).
    pub max_bytes: usize,
    /// Stats.
    pub stats: CacheStats,
    tick: u64,
}

impl<S> CacheManager<S>
where
    S: super::cache_store::CacheStore,
{
    /// New manager.
    pub fn new(store: S) -> Self {
        Self {
            store,
            meta: HashMap::new(),
            max_bytes: 0,
            stats: CacheStats::default(),
            tick: 0,
        }
    }

    /// Borrow the inner store.
    pub fn store(&self) -> &S {
        &self.store
    }
    /// Mutable borrow.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Get (updates LRU + stats).
    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        self.tick += 1;
        let data = self.store.get(key);
        if data.is_some() {
            self.stats.hits += 1;
            if let Some(m) = self.meta.get_mut(key) {
                m.last_used = self.tick;
            }
        } else {
            self.stats.misses += 1;
        }
        data
    }

    /// Put (evicts if needed).
    pub fn put(&mut self, key: &str, data: &[u8]) {
        self.tick += 1;
        self.store.put(key, data);
        self.meta.insert(
            key.to_string(),
            Meta {
                size: data.len(),
                last_used: self.tick,
            },
        );
        self.stats.stores += 1;
        self.evict_if_needed();
    }

    /// Total bytes tracked.
    pub fn bytes(&self) -> usize {
        self.meta.values().map(|m| m.size).sum()
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.meta.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    fn evict_if_needed(&mut self) {
        if self.max_bytes == 0 {
            return;
        }
        while self.bytes() > self.max_bytes && !self.meta.is_empty() {
            let victim = self
                .meta
                .iter()
                .min_by_key(|(_, m)| m.last_used)
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    self.store.remove(&k);
                    self.meta.remove(&k);
                    self.stats.evictions += 1;
                }
                None => break,
            }
        }
    }

    /// Remove a key.
    pub fn remove(&mut self, key: &str) -> bool {
        self.meta.remove(key);
        self.store.remove(key)
    }

    /// Clear.
    pub fn clear(&mut self) {
        self.store.clear();
        self.meta.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader_cache::cache_store::MemoryStore;

    #[test]
    fn lru_eviction_order() {
        let mut c = CacheManager::new(MemoryStore::new());
        c.max_bytes = 10;
        c.put("a", &[0; 6]);
        c.put("b", &[0; 6]); // over → evict a (older)
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some());
        // touch b then add c → evict b? b just used, c older? a gone.
        c.put("c", &[0; 6]); // total a? b(6)+c(6)=12 → evict oldest used = b? b was touched → c older? c just inserted → evict b? meta: b last_used older than c
                             // after: b evicted, a evicted earlier; c remains
        assert!(c.get("c").is_some());
    }

    #[test]
    fn stats_hit_rate() {
        let mut c = CacheManager::new(MemoryStore::new());
        c.put("x", b"1");
        let _ = c.get("x");
        let _ = c.get("x");
        let _ = c.get("missing");
        assert_eq!(c.stats.hits, 2);
        assert_eq!(c.stats.misses, 1);
        assert!((c.stats.hit_rate() - 2.0 / 3.0).abs() < 0.01);
    }
}
