//! Utilities — Phase 11.5.
//!
//! File/string/time/hash helpers, synchronization helpers, and
//! thread-pool utilities. No external deps beyond std.

#[allow(clippy::module_inception)]
pub mod utils;
pub mod utils_sync;
pub mod utils_thread;

pub use utils::{fnv1a, human_bytes, now_secs, sanitize_filename, slugify, HumanBytes};
pub use utils_sync::{Shared, SharedMut};
pub use utils_thread::{NamedThread, ThreadPool, WorkerCount};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::now_secs();
    }
}
