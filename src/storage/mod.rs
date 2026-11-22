//! Storage system — Phase 11.5.
//!
//! NVMe/HDD/SSD device model, ISO mounts, and an SSD cache layer.
//! Complements `io::storage` (which handles game-image streaming);
//! this layer models *devices* and *caching policy*.

#[allow(clippy::module_inception)]
pub mod storage;
pub mod storage_cache;
pub mod storage_mount;

pub use storage::{DriveKind, StorageDevice, StorageError, StorageManager};
pub use storage_cache::{CachePolicy as StorageCachePolicy, SsdCache};
pub use storage_mount::{Mount, MountError, MountPoint};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::DriveKind::Nvme;
    }
}
