//! Cloud save / CDN layer — Phase 11.5.
//!
//! Client-facing cloud API. All network ops go through an injectable
//! `Transport` so tests never touch the network. Conflict resolution
//! is last-writer-wins by timestamp with explicit `Conflict` reports.

pub mod cloud_auth;
pub mod cloud_cache;
pub mod cloud_client;
pub mod cloud_sync;

pub use cloud_auth::{AuthError, AuthToken, CloudAuth};
pub use cloud_cache::{CacheEntry, CloudCache};
pub use cloud_client::{CloudClient, CloudError, HttpResponse, Transport};
pub use cloud_sync::{CloudSync, ConflictResolution, SyncDirection, SyncReport};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::SyncDirection::Upload;
    }
}
