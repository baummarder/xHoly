//! Storage cache — SSD read cache for slower backing devices.

use std::collections::{BTreeMap, VecDeque};

/// Cache policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Cache everything read.
    ReadAll,
    /// Cache only sequential-access regions (streaming).
    SequentialOnly,
    /// No caching.
    Bypass,
}

/// A cached block.
#[derive(Debug, Clone)]
struct Block {
    data: Vec<u8>,
    /// Last access counter.
    last_used: u64,
}

/// SSD cache: fixed-size, LRU-evicted block cache.
#[derive(Debug)]
pub struct SsdCache {
    blocks: BTreeMap<u64, Block>,
    /// Access-order queue (block addrs).
    order: VecDeque<u64>,
    /// Block size bytes.
    pub block_size: usize,
    /// Max blocks.
    pub max_blocks: usize,
    /// Current policy.
    pub policy: CachePolicy,
    tick: u64,
    /// Stats.
    pub hits: u64,
    pub misses: u64,
}

impl SsdCache {
    /// New cache.
    pub fn new(block_size: usize, max_blocks: usize) -> Self {
        Self {
            blocks: BTreeMap::new(),
            order: VecDeque::new(),
            block_size: block_size.max(1),
            max_blocks,
            policy: CachePolicy::ReadAll,
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Block address for a byte offset.
    fn block_addr(offset: u64) -> u64 {
        offset / 4096 // 4 KiB blocks typical
    }

    /// Read `len` bytes at `offset` from backing `src` (a closure).
    /// Returns cached+backing composite.
    pub fn read(
        &mut self,
        offset: u64,
        len: usize,
        mut backing: impl FnMut(u64, usize) -> Vec<u8>,
    ) -> Vec<u8> {
        if self.policy == CachePolicy::Bypass {
            return backing(offset, len);
        }
        let mut out = Vec::with_capacity(len);
        let mut cur = offset;
        let end = offset + len as u64;
        while cur < end {
            let ba = Self::block_addr(cur);
            let boff = (cur % 4096) as usize;
            let want = ((end - cur) as usize).min(4096 - boff);
            self.tick += 1;
            if let Some(b) = self.blocks.get_mut(&ba) {
                self.hits += 1;
                b.last_used = self.tick;
                out.extend_from_slice(&b.data[boff..boff + want]);
            } else {
                self.misses += 1;
                let data = backing(ba * 4096, 4096);
                let blk = Block {
                    data: data.clone(),
                    last_used: self.tick,
                };
                self.blocks.insert(ba, blk);
                self.order.push_back(ba);
                self.evict();
                out.extend_from_slice(&data[boff..boff + want]);
            }
            cur += want as u64;
        }
        out
    }

    /// Invalidate a block.
    pub fn invalidate(&mut self, offset: u64) {
        let ba = Self::block_addr(offset);
        self.blocks.remove(&ba);
        self.order.retain(|&a| a != ba);
    }

    /// Invalidate all.
    pub fn invalidate_all(&mut self) {
        self.blocks.clear();
        self.order.clear();
    }

    /// Cached block count.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Hit rate.
    pub fn hit_rate(&self) -> f64 {
        let t = self.hits + self.misses;
        if t == 0 {
            0.0
        } else {
            self.hits as f64 / t as f64
        }
    }

    fn evict(&mut self) {
        while self.blocks.len() > self.max_blocks {
            // Evict oldest-used.
            let victim = self
                .blocks
                .iter()
                .min_by_key(|(_, b)| b.last_used)
                .map(|(k, _)| *k);
            match victim {
                Some(k) => {
                    self.blocks.remove(&k);
                    self.order.retain(|&a| a != k);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_through_populates() {
        let mut c = SsdCache::new(4096, 4);
        // backing: byte value = offset low byte pattern
        let data = c.read(0, 8192, |off, len| vec![(off % 251) as u8; len]);
        assert_eq!(data.len(), 8192);
        // Second read hits cache.
        let data2 = c.read(0, 8192, |_, _| panic!("should hit cache"));
        assert_eq!(data, data2);
        assert_eq!(c.hits, 2); // 2 blocks hit
    }

    #[test]
    fn bypass_policy() {
        let mut c = SsdCache::new(4096, 4);
        c.policy = CachePolicy::Bypass;
        let d = c.read(0, 100, |_, _| vec![7; 100]);
        assert_eq!(d, vec![7; 100]);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn eviction_bounds() {
        let mut c = SsdCache::new(4096, 2);
        for off in [0u64, 4096, 8192, 12288] {
            let _ = c.read(off, 16, |o, l| vec![(o % 256) as u8; l]);
        }
        assert!(c.len() <= 2);
    }
}
