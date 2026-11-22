//! Cloud-sync for saves — stub transport + real conflict resolution.
//!
//! The actual upload/download goes through the `cloud` module's
//! client trait; this file owns *policy*: direction, conflict
//! resolution, and status reporting. Fully testable offline.

use std::collections::HashMap;

/// Sync direction for a save blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    /// Local → remote.
    Upload,
    /// Remote → local.
    Download,
}

/// How to resolve a local-vs-remote conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictResolution {
    /// Keep local (default for single-player saves).
    #[default]
    KeepLocal,
    /// Keep remote.
    KeepRemote,
    /// Keep whichever has the newer timestamp.
    Newest,
}

/// Result of a sync attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// In sync — nothing to do.
    UpToDate,
    /// Uploaded/downloaded successfully.
    Synced(SyncDirection),
    /// Conflict detected; resolved per policy.
    ConflictResolved(SyncDirection),
    /// Offline / transport failure.
    Offline(String),
    /// Remote object missing.
    MissingRemote,
}

/// A save blob as seen by the sync layer.
#[derive(Debug, Clone)]
pub struct SyncBlob {
    /// Slot/save identifier.
    pub key: String,
    /// Content hash (FNV-1a hex).
    pub hash: String,
    /// Last-modified timestamp (unix secs).
    pub mtime: u64,
    /// Serialized bytes (empty = tombstone/delete).
    pub data: Vec<u8>,
}

/// Minimal cloud transport the sync engine needs.
/// Implemented by `cloud::CloudClient`; tests use `MemTransport`.
pub trait SyncTransport {
    /// Fetch remote blob for `key` (None = missing).
    fn fetch(&self, key: &str) -> Result<Option<SyncBlob>, String>;
    /// Push `blob` to remote.
    fn push(&self, blob: &SyncBlob) -> Result<(), String>;
}

/// In-memory transport for tests.
#[derive(Debug, Default)]
pub struct MemTransport {
    pub blobs: std::sync::Mutex<HashMap<String, SyncBlob>>,
    /// When true, all ops fail (offline simulation).
    pub offline: std::sync::atomic::AtomicBool,
}

impl SyncTransport for MemTransport {
    fn fetch(&self, key: &str) -> Result<Option<SyncBlob>, String> {
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("offline".into());
        }
        Ok(self.blobs.lock().unwrap().get(key).cloned())
    }
    fn push(&self, blob: &SyncBlob) -> Result<(), String> {
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("offline".into());
        }
        self.blobs
            .lock()
            .unwrap()
            .insert(blob.key.clone(), blob.clone());
        Ok(())
    }
}

/// Sync engine: compares local/remote hashes and applies policy.
pub struct CloudSync<'a, T: SyncTransport> {
    transport: &'a T,
    /// Local blob provider (caller-owned store).
    pub local: HashMap<String, SyncBlob>,
    pub policy: ConflictResolution,
}

impl<'a, T: SyncTransport> CloudSync<'a, T> {
    /// New sync engine over `transport`.
    pub fn new(transport: &'a T) -> Self {
        Self {
            transport,
            local: HashMap::new(),
            policy: ConflictResolution::default(),
        }
    }

    /// Sync one key: fetch remote, compare, act.
    pub fn sync_key(&mut self, key: &str, now: u64) -> SyncStatus {
        let remote = match self.transport.fetch(key) {
            Ok(r) => r,
            Err(e) => return SyncStatus::Offline(e),
        };
        let local = self.local.get(key).cloned();

        match (local, remote) {
            (None, None) => SyncStatus::UpToDate,
            (Some(l), None) => {
                // Remote missing → upload local.
                match self.transport.push(&l) {
                    Ok(()) => SyncStatus::Synced(SyncDirection::Upload),
                    Err(e) => SyncStatus::Offline(e),
                }
            }
            (None, Some(r)) => {
                // Local missing → adopt remote.
                self.local.insert(key.to_string(), r);
                SyncStatus::Synced(SyncDirection::Download)
            }
            (Some(l), Some(r)) => {
                if l.hash == r.hash {
                    return SyncStatus::UpToDate;
                }
                // Conflict.
                let use_remote = match self.policy {
                    ConflictResolution::KeepLocal => false,
                    ConflictResolution::KeepRemote => true,
                    ConflictResolution::Newest => r.mtime >= l.mtime,
                };
                if use_remote {
                    self.local.insert(key.to_string(), r);
                } else {
                    // Push local (refresh mtime).
                    let mut l2 = l.clone();
                    l2.mtime = now;
                    self.local.insert(key.to_string(), l2.clone());
                    let _ = self.transport.push(&l2);
                }
                SyncStatus::ConflictResolved(if use_remote {
                    SyncDirection::Download
                } else {
                    SyncDirection::Upload
                })
            }
        }
    }

    /// Sync all local keys.
    pub fn sync_all(&mut self, now: u64) -> Vec<(String, SyncStatus)> {
        let keys: Vec<String> = self.local.keys().cloned().collect();
        keys.into_iter()
            .map(|k| {
                let s = self.sync_key(&k, now);
                (k, s)
            })
            .collect()
    }
}

/// FNV-1a 64 hash → hex (used for blob identity).
pub fn blob_hash(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(key: &str, data: &[u8], mtime: u64) -> SyncBlob {
        SyncBlob {
            key: key.into(),
            hash: blob_hash(data),
            mtime,
            data: data.to_vec(),
        }
    }

    #[test]
    fn upload_when_remote_missing() {
        let t = MemTransport::default();
        let mut s = CloudSync::new(&t);
        s.local.insert("k".into(), blob("k", b"abc", 1));
        assert_eq!(
            s.sync_key("k", 10),
            SyncStatus::Synced(SyncDirection::Upload)
        );
        assert!(t.blobs.lock().unwrap().contains_key("k"));
    }

    #[test]
    fn download_when_local_missing() {
        let t = MemTransport::default();
        t.blobs
            .lock()
            .unwrap()
            .insert("k".into(), blob("k", b"remote", 5));
        let mut s = CloudSync::new(&t);
        assert_eq!(
            s.sync_key("k", 10),
            SyncStatus::Synced(SyncDirection::Download)
        );
        assert_eq!(s.local["k"].data, b"remote");
    }

    #[test]
    fn conflict_newest_wins() {
        let t = MemTransport::default();
        t.blobs
            .lock()
            .unwrap()
            .insert("k".into(), blob("k", b"remote-new", 100));
        let mut s = CloudSync::new(&t);
        s.policy = ConflictResolution::Newest;
        s.local.insert("k".into(), blob("k", b"local-old", 1));
        assert_eq!(
            s.sync_key("k", 200),
            SyncStatus::ConflictResolved(SyncDirection::Download)
        );
        assert_eq!(s.local["k"].data, b"remote-new");
    }

    #[test]
    fn offline_reported() {
        let t = MemTransport::default();
        t.offline.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut s = CloudSync::new(&t);
        s.local.insert("k".into(), blob("k", b"x", 1));
        assert!(matches!(s.sync_key("k", 1), SyncStatus::Offline(_)));
    }

    #[test]
    fn identical_is_uptodate() {
        let t = MemTransport::default();
        let b = blob("k", b"same", 1);
        t.blobs.lock().unwrap().insert("k".into(), b.clone());
        let mut s = CloudSync::new(&t);
        s.local.insert("k".into(), b);
        assert_eq!(s.sync_key("k", 1), SyncStatus::UpToDate);
    }
}
