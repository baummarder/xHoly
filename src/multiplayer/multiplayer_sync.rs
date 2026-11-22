//! State sync — replicated key/value state with consistency modes.

use std::collections::BTreeMap;

/// Consistency model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyMode {
    /// Host authoritative; clients send inputs only.
    ClientServer,
    /// All peers simulate; desyncs resolved by vote/hash.
    Lockstep,
    /// Peers exchange full state snapshots periodically.
    Snapshot,
}

/// Replicated state: peer_id → (key → value bytes).
/// Values are opaque blobs; interpretation is game-specific.
#[derive(Debug, Default)]
pub struct ReplicatedState {
    /// peer → key → (value, last_update_tick).
    inner: BTreeMap<u64, BTreeMap<String, (Vec<u8>, u64)>>,
}

impl ReplicatedState {
    /// New state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a value for a peer.
    pub fn set(&mut self, peer: u64, key: impl Into<String>, value: Vec<u8>, tick: u64) {
        self.inner
            .entry(peer)
            .or_default()
            .insert(key.into(), (value, tick));
    }

    /// Get a peer's value.
    pub fn get(&self, peer: u64, key: &str) -> Option<&[u8]> {
        self.inner.get(&peer)?.get(key).map(|(v, _)| v.as_slice())
    }

    /// Merge another peer's state (snapshot mode).
    pub fn merge(&mut self, peer: u64, other: &ReplicatedState) {
        if let Some(their) = other.inner.get(&peer) {
            let entry = self.inner.entry(peer).or_default();
            for (k, (v, t)) in their {
                let replace = entry.get(k).map(|(_, et)| t > et).unwrap_or(true);
                if replace {
                    entry.insert(k.clone(), (v.clone(), *t));
                }
            }
        }
    }

    /// All peers.
    pub fn peers(&self) -> Vec<u64> {
        self.inner.keys().copied().collect()
    }

    /// Keys for a peer.
    pub fn keys(&self, peer: u64) -> Vec<&str> {
        self.inner
            .get(&peer)
            .map(|m| m.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Deterministic hash of full state (desync detection).
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for (p, keys) in &self.inner {
            for b in p.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            for (k, (v, t)) in keys {
                for b in k.bytes().chain(v.iter().copied()).chain(t.to_le_bytes()) {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }
}

/// The sync engine: applies incoming updates per consistency mode.
pub struct SyncEngine {
    /// Mode.
    pub mode: ConsistencyMode,
    /// Host peer id (ClientServer mode).
    pub host: u64,
}

impl SyncEngine {
    /// New engine.
    pub fn new(mode: ConsistencyMode, host: u64) -> Self {
        Self { mode, host }
    }

    /// Should `peer`'s write to `key` be accepted?
    /// ClientServer: only host writes authoritative state; others
    /// may write "input" keys (prefixed `in:`).
    pub fn accept_write(&self, peer: u64, key: &str) -> bool {
        match self.mode {
            ConsistencyMode::ClientServer => peer == self.host || key.starts_with("in:"),
            _ => true,
        }
    }

    /// Detect desync between two state hashes.
    pub fn desynced(&self, a: &ReplicatedState, b: &ReplicatedState) -> bool {
        a.hash() != b.hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_newer() {
        let mut a = ReplicatedState::new();
        a.set(1, "hp", vec![100], 5);
        let mut b = ReplicatedState::new();
        b.set(1, "hp", vec![90], 3); // older
        b.set(1, "mp", vec![50], 7); // newer key
        a.merge(1, &b);
        assert_eq!(a.get(1, "hp"), Some(&[100][..])); // kept newer local
        assert_eq!(a.get(1, "mp"), Some(&[50][..]));
    }

    #[test]
    fn client_server_write_rules() {
        let e = SyncEngine::new(ConsistencyMode::ClientServer, 1);
        assert!(e.accept_write(1, "hp")); // host
        assert!(e.accept_write(2, "in:move")); // client input
        assert!(!e.accept_write(2, "hp")); // client can't write state
    }

    #[test]
    fn desync_detection() {
        let mut a = ReplicatedState::new();
        let mut b = ReplicatedState::new();
        a.set(1, "x", vec![1], 1);
        assert!(SyncEngine::new(ConsistencyMode::Lockstep, 1).desynced(&a, &b));
        b.set(1, "x", vec![1], 1);
        assert!(!SyncEngine::new(ConsistencyMode::Lockstep, 1).desynced(&a, &b));
    }
}
