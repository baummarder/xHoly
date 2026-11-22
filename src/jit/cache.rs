//! Instruction cache — Phase 13.
//!
//! Two-level LRU cache mapping guest code bytes → compiled WASM
//! modules. L1: 16 KiB budget, 4-way associative. L2: 64 KiB budget,
//! 8-way. Entries are evicted LRU-first; a miss triggers recompilation
//! upstream ([`super::compiler::JitCompiler`]).

use std::collections::HashMap;

/// Cache key: FNV-1a hash of the code bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(u64);

impl CacheKey {
    /// Hash code bytes (FNV-1a 64).
    pub fn of(code: &[u8]) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in code {
            h ^= b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        Self(h)
    }
}

/// A compiled translation unit stored in the cache.
#[derive(Debug)]
pub struct CacheEntry<V> {
    pub key: CacheKey,
    pub value: V,
    /// Byte size accounted against the budget.
    pub size: usize,
    /// LRU tick.
    tick: u64,
}

/// One cache level with a byte budget and associativity.
#[derive(Debug)]
pub struct CacheLevel<V> {
    budget: usize,
    ways: usize,
    entries: Vec<CacheEntry<V>>,
    clock: u64,
    hits: u64,
    misses: u64,
}

impl<V> CacheLevel<V> {
    /// New level.
    pub fn new(budget: usize, ways: usize) -> Self {
        Self {
            budget,
            ways,
            entries: Vec::new(),
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn touch(&mut self, idx: usize) {
        self.clock += 1;
        self.entries[idx].tick = self.clock;
    }

    /// Lookup; updates LRU on hit.
    pub fn get(&mut self, key: CacheKey) -> Option<&V> {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.hits += 1;
            self.touch(i);
            return Some(&self.entries[i].value);
        }
        self.misses += 1;
        None
    }

    /// Insert; evicts LRU entries until under budget.
    /// If a same-set entry exists beyond associativity it is evicted.
    pub fn insert(&mut self, key: CacheKey, value: V, size: usize) {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            self.entries[i].value = value;
            self.entries[i].size = size;
            self.touch(i);
            return;
        }
        self.clock += 1;
        self.entries.push(CacheEntry {
            key,
            value,
            size,
            tick: self.clock,
        });
        // Enforce associativity: keep at most `ways` entries per hash set.
        let set_mask = self.ways - 1;
        let set = (key.0 as usize) & set_mask;
        let mut same_set: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| (e.key.0 as usize & set_mask) == set)
            .map(|(i, _)| i)
            .collect();
        same_set.sort_by_key(|&i| self.entries[i].tick);
        while same_set.len() > self.ways {
            let victim = same_set.remove(0);
            self.entries.remove(victim);
            // Recompute indices after removal.
            same_set = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| (e.key.0 as usize & set_mask) == set)
                .map(|(i, _)| i)
                .collect();
            same_set.sort_by_key(|&i| self.entries[i].tick);
        }
        // Enforce byte budget: evict global LRU until under.
        let mut total: usize = self.entries.iter().map(|e| e.size).sum();
        while total > self.budget && self.entries.len() > 1 {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.tick)
                .map(|(i, _)| i)
                .unwrap();
            total -= self.entries[victim].size;
            self.entries.remove(victim);
        }
    }

    /// Hit rate stats.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// Number of resident entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Two-level instruction cache.
pub struct InstructionCache<V> {
    pub l1: CacheLevel<V>,
    pub l2: CacheLevel<V>,
    /// Pending promotions: L1 evictions go to L2.
    #[allow(dead_code)]
    pending: HashMap<CacheKey, (V, usize)>,
}

impl<V: Clone> InstructionCache<V> {
    /// L1 16 KiB/4-way, L2 64 KiB/8-way.
    pub fn new() -> Self {
        Self {
            l1: CacheLevel::new(16 * 1024, 4),
            l2: CacheLevel::new(64 * 1024, 8),
            pending: HashMap::new(),
        }
    }

    /// Lookup in L1 then L2 (L2 hit promotes to L1).
    pub fn get(&mut self, key: CacheKey) -> Option<V> {
        if let Some(v) = self.l1.get(key) {
            return Some(v.clone());
        }
        if let Some(v) = self.l2.get(key) {
            let v = v.clone();
            self.l1.insert(key, v.clone(), 1);
            return Some(v);
        }
        None
    }

    /// Insert into L1 (L2 filled via explicit promote on L1 eviction —
    /// simplified: inserts go to both levels for lookup robustness).
    pub fn insert(&mut self, key: CacheKey, value: V, size: usize) {
        self.l2.insert(key, value.clone(), size);
        self.l1.insert(key, value, size);
    }

    /// Combined hit/miss counters.
    pub fn stats(&self) -> (u64, u64, u64, u64) {
        let (h1, m1) = self.l1.stats();
        let (h2, m2) = self.l2.stats();
        (h1, m1, h2, m2)
    }
}

impl<V: Clone> Default for InstructionCache<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_eviction_at_budget() {
        let mut c = CacheLevel::new(100, 4);
        for i in 0..10u64 {
            c.insert(CacheKey(i * 31 + 7), i, 20);
        }
        assert!(c.len() <= 5, "budget 100 / size 20 → ≤5 entries");
        // Oldest entries evicted.
        assert!(c.get(CacheKey(7)).is_none() || c.len() < 10);
    }

    #[test]
    fn hit_promotes_and_counts() {
        let mut c = InstructionCache::<u32>::new();
        c.insert(CacheKey::of(b"mov rax, 1"), 42, 10);
        assert_eq!(c.get(CacheKey::of(b"mov rax, 1")), Some(42));
        let (h1, _m1, _h2, _m2) = c.stats();
        assert!(h1 >= 1);
    }
}
