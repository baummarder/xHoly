//! Synchronization helpers — shared-state wrappers with clear names.
//!
//! Thin aliases over std sync primitives so call sites read
//! intentfully (`Shared<T>` vs `SharedMut<T>`).

use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Immutable-shareable handle: `Arc<T>` for read-mostly data.
pub type Shared<T> = Arc<T>;

/// Mutable-shareable handle: `Arc<Mutex<T>>`.
pub type SharedMut<T> = Arc<Mutex<T>>;

/// Create a shared-mutable value.
pub fn shared_mut<T>(v: T) -> SharedMut<T> {
    Arc::new(Mutex::new(v))
}

/// Lock a SharedMut, panicking on poison (documented choice: poisoned
/// mutex = unrecoverable state → crash is safer than silent UB-adjacent
/// behavior for emulator state).
pub fn lock<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Create a shared read-write value.
pub fn shared_rw<T>(v: T) -> Arc<RwLock<T>> {
    Arc::new(RwLock::new(v))
}

/// Read-lock.
pub fn read<'a, T>(l: &'a RwLock<T>) -> RwLockReadGuard<'a, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}
/// Write-lock.
pub fn write<'a, T>(l: &'a RwLock<T>) -> RwLockWriteGuard<'a, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Atomic u64 helpers for counters (seq_cst).
pub mod atomic {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Fetch-add and return previous.
    pub fn fetch_add(a: &AtomicU64, v: u64) -> u64 {
        a.fetch_add(v, Ordering::SeqCst)
    }
    /// Load.
    pub fn load(a: &AtomicU64) -> u64 {
        a.load(Ordering::SeqCst)
    }
    /// Store.
    pub fn store(a: &AtomicU64, v: u64) {
        a.store(v, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn shared_mut_works() {
        let s = shared_mut(vec![1, 2]);
        lock(&s).push(3);
        assert_eq!(lock(&s).len(), 3);
    }

    #[test]
    fn rw_shared_works() {
        let l = shared_rw(0u32);
        *write(&l) = 42;
        assert_eq!(*read(&l), 42);
    }

    #[test]
    fn atomic_helpers() {
        let a = AtomicU64::new(0);
        assert_eq!(atomic::fetch_add(&a, 5), 0);
        assert_eq!(atomic::load(&a), 5);
        atomic::store(&a, 9);
        assert_eq!(atomic::load(&a), 9);
    }
}
