//! Save/load system — Phase 11.5.
//!
//! Versioned save files (binary header + serde payload), slot
//! management (1–8 + quick/auto), optional RLE compression, and a
//! cloud-sync hook (stub transport; see `cloud` module).

pub mod cloud_sync;
pub mod save_compression;
pub mod save_format;
pub mod save_manager;
pub mod save_slot;

pub use cloud_sync::{CloudSync, ConflictResolution, SyncDirection, SyncStatus};
pub use save_compression::{compress, decompress};
pub use save_format::{SaveError, SaveHeader, SavePayload};
pub use save_manager::{AutoSavePolicy, SaveManager, SaveOutcome};
pub use save_slot::{SaveMeta, SaveSlot, MAX_SLOTS, QUICK_SLOT};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        assert_eq!(super::MAX_SLOTS, 8);
    }
}
