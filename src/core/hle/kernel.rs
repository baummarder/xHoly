//! Guest-side kernel primitives for the HLE runtime — Milestone 36.3.
//!
//! **These are xHoly VM services, not Xbox kernel APIs.** Nothing here
//! claims a verified `Ke*`/`Nt*`/XAM signature. Names, state machine and
//! timeout units are defined by this file and are exposed through the
//! separate `xh_vm.krn.dll` / `xh_vm.sync.dll` registry modules. No real
//! Xbox kernel semantics, ordinals or structure layouts are invented.
//!
//! Design constraints:
//!
//! * guest threads/events/mutexes are **data**, never host threads;
//! * all tables are bounded, so guest input cannot grow host memory;
//! * time is a deterministic VM tick (one service call / tick advance),
//!   **not** a wall clock and not an Xbox QPC;
//! * a self-deadlock is reported, never waited on forever.

use std::collections::BTreeMap;
use std::fmt;

/// Maximum number of guest threads in one context.
pub const MAX_THREADS: usize = 64;
/// Maximum number of guest events.
pub const MAX_EVENTS: usize = 64;
/// Maximum number of guest mutexes.
pub const MAX_MUTEXES: usize = 64;

/// Lifecycle of a guest thread (xHoly VM state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Runnable.
    Ready,
    /// Sleeping until `wake_tick`.
    Sleeping,
    /// Suspended by the guest.
    Suspended,
    /// Finished.
    Dead,
}

/// A guest thread — register-free bookkeeping only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestThread {
    /// Opaque guest-visible handle (small integer, not an address).
    pub id: u64,
    /// Current lifecycle state.
    pub state: ThreadState,
    /// Tick at which a sleeping thread becomes ready.
    pub wake_tick: u64,
}

/// Outcome of a timed wait (deterministic, no host timing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    /// The object became ready/signaled.
    Ready,
    /// The timeout expired while still unsignaled.
    TimedOut,
    /// Unknown handle.
    InvalidHandle,
    /// The caller already owns the mutex (self-deadlock refused).
    Deadlock,
}

impl fmt::Display for WaitResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("ready"),
            Self::TimedOut => f.write_str("timed out"),
            Self::InvalidHandle => f.write_str("invalid handle"),
            Self::Deadlock => f.write_str("deadlock refused"),
        }
    }
}

/// A guest event with an explicit signaled state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestEvent {
    /// Opaque handle.
    pub id: u64,
    /// Whether the event is currently signaled.
    pub signaled: bool,
}

/// A guest mutex with owner and waiter bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMutex {
    /// Opaque handle.
    pub id: u64,
    /// Owning thread, if any.
    pub owner: Option<u64>,
    /// Threads queued behind the lock (in arrival order).
    pub waiters: Vec<u64>,
}

/// Synchronous, deterministic guest kernel state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelContext {
    threads: BTreeMap<u64, GuestThread>,
    events: BTreeMap<u64, GuestEvent>,
    mutexes: BTreeMap<u64, GuestMutex>,
    next_id: u64,
    /// Monotonic VM tick; incremented by `advance()` and `sleep()`.
    ticks: u64,
}

impl Default for KernelContext {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelContext {
    /// Empty context with a reserved handle zero ("invalid").
    pub fn new() -> Self {
        Self {
            threads: BTreeMap::new(),
            events: BTreeMap::new(),
            mutexes: BTreeMap::new(),
            next_id: 1,
            ticks: 0,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Current monotonic VM tick.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Advance the VM tick by one service step.
    pub fn advance(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
        // Sleeping threads wake when their deadline is reached.
        for thread in self.threads.values_mut() {
            if thread.state == ThreadState::Sleeping && thread.wake_tick <= self.ticks {
                thread.state = ThreadState::Ready;
                thread.wake_tick = 0;
            }
        }
    }

    /// Create a ready guest thread. Returns `None` when the table is full.
    pub fn create_thread(&mut self) -> Option<u64> {
        if self.threads.len() >= MAX_THREADS {
            return None;
        }
        let id = self.fresh_id();
        self.threads.insert(
            id,
            GuestThread {
                id,
                state: ThreadState::Ready,
                wake_tick: 0,
            },
        );
        Some(id)
    }

    /// Current state of a thread, if it exists.
    pub fn thread_state(&self, id: u64) -> Option<ThreadState> {
        self.threads.get(&id).map(|t| t.state)
    }

    /// Number of live thread records.
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// Put a thread to sleep for `duration` VM ticks.
    pub fn sleep(&mut self, id: u64, duration: u64) -> bool {
        let deadline = self.ticks.saturating_add(duration);
        match self.threads.get_mut(&id) {
            Some(thread) if thread.state == ThreadState::Ready => {
                thread.state = ThreadState::Sleeping;
                thread.wake_tick = deadline;
                if duration == 0 {
                    thread.state = ThreadState::Ready;
                    thread.wake_tick = 0;
                }
                true
            }
            _ => false,
        }
    }

    /// Suspend / resume a thread.
    pub fn suspend(&mut self, id: u64) -> bool {
        match self.threads.get_mut(&id) {
            Some(thread) if thread.state == ThreadState::Ready => {
                thread.state = ThreadState::Suspended;
                true
            }
            _ => false,
        }
    }

    pub fn resume(&mut self, id: u64) -> bool {
        match self.threads.get_mut(&id) {
            Some(thread) if thread.state == ThreadState::Suspended => {
                thread.state = ThreadState::Ready;
                true
            }
            _ => false,
        }
    }

    /// Mark a thread dead. Unknown or already-dead handles are refused.
    pub fn exit_thread(&mut self, id: u64) -> bool {
        match self.threads.get_mut(&id) {
            Some(thread) if thread.state != ThreadState::Dead => {
                thread.state = ThreadState::Dead;
                true
            }
            _ => false,
        }
    }

    /// Cooperative yield: a ready thread stays ready and the VM tick moves.
    pub fn yield_thread(&mut self, id: u64) -> bool {
        if self.thread_state(id) != Some(ThreadState::Ready) {
            return false;
        }
        self.advance();
        true
    }

    /// Create an unsignaled guest event.
    pub fn create_event(&mut self) -> Option<u64> {
        if self.events.len() >= MAX_EVENTS {
            return None;
        }
        let id = self.fresh_id();
        self.events.insert(
            id,
            GuestEvent {
                id,
                signaled: false,
            },
        );
        Some(id)
    }

    /// Signal an event.
    pub fn set_event(&mut self, id: u64) -> bool {
        match self.events.get_mut(&id) {
            Some(event) => {
                event.signaled = true;
                true
            }
            None => false,
        }
    }

    /// Clear an event.
    pub fn clear_event(&mut self, id: u64) -> bool {
        match self.events.get_mut(&id) {
            Some(event) => {
                event.signaled = false;
                true
            }
            None => false,
        }
    }

    /// Wait for an event at most `timeout` VM ticks.
    pub fn wait_event(&mut self, id: u64, timeout: u64) -> WaitResult {
        if !self.events.contains_key(&id) {
            return WaitResult::InvalidHandle;
        }
        if self.events.get(&id).is_some_and(|event| event.signaled) {
            return WaitResult::Ready;
        }
        for _ in 0..timeout {
            self.advance();
            if self.events.get(&id).is_some_and(|event| event.signaled) {
                return WaitResult::Ready;
            }
        }
        WaitResult::TimedOut
    }

    /// Create an unlocked guest mutex.
    pub fn create_mutex(&mut self) -> Option<u64> {
        if self.mutexes.len() >= MAX_MUTEXES {
            return None;
        }
        let id = self.fresh_id();
        self.mutexes.insert(
            id,
            GuestMutex {
                id,
                owner: None,
                waiters: Vec::new(),
            },
        );
        Some(id)
    }

    /// Acquire a mutex for `thread`, waiting at most `timeout` ticks.
    pub fn lock_mutex(&mut self, id: u64, thread: u64, timeout: u64) -> WaitResult {
        let Some(mutex) = self.mutexes.get(&id) else {
            return WaitResult::InvalidHandle;
        };
        if mutex.owner == Some(thread) {
            return WaitResult::Deadlock;
        }
        if mutex.owner.is_none() {
            if let Some(mutex) = self.mutexes.get_mut(&id) {
                mutex.owner = Some(thread);
            }
            return WaitResult::Ready;
        }
        // Deterministic, bounded wait: poll the owner state per VM tick.
        for _ in 0..timeout {
            self.advance();
            let free = self
                .mutexes
                .get(&id)
                .is_some_and(|mutex| mutex.owner.is_none());
            if free {
                if let Some(mutex) = self.mutexes.get_mut(&id) {
                    mutex.owner = Some(thread);
                }
                return WaitResult::Ready;
            }
        }
        if let Some(mutex) = self.mutexes.get_mut(&id) {
            if !mutex.waiters.contains(&thread) && mutex.waiters.len() < MAX_THREADS {
                mutex.waiters.push(thread);
            }
        }
        WaitResult::TimedOut
    }

    /// Release a mutex. Only the owning thread may unlock it.
    pub fn unlock_mutex(&mut self, id: u64, thread: u64) -> WaitResult {
        let Some(mutex) = self.mutexes.get(&id) else {
            return WaitResult::InvalidHandle;
        };
        if mutex.owner != Some(thread) {
            return WaitResult::InvalidHandle;
        }
        if let Some(mutex) = self.mutexes.get_mut(&id) {
            mutex.owner = mutex.waiters.first().copied();
            if !mutex.waiters.is_empty() {
                mutex.waiters.remove(0);
            }
        }
        WaitResult::Ready
    }

    /// Mutex owner, if any (diagnostics).
    pub fn mutex_owner(&self, id: u64) -> Option<u64> {
        self.mutexes.get(&id).and_then(|mutex| mutex.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_lifecycle_is_deterministic() {
        let mut ctx = KernelContext::new();
        let id = ctx.create_thread().expect("thread");
        assert_eq!(ctx.thread_state(id), Some(ThreadState::Ready));
        assert!(ctx.suspend(id));
        assert_eq!(ctx.thread_state(id), Some(ThreadState::Suspended));
        assert!(!ctx.yield_thread(id), "suspended threads cannot yield");
        assert!(ctx.resume(id));
        assert!(ctx.yield_thread(id));
        assert_eq!(ctx.ticks(), 1);
        assert!(ctx.sleep(id, 3));
        assert_eq!(ctx.thread_state(id), Some(ThreadState::Sleeping));
        for _ in 0..3 {
            ctx.advance();
        }
        assert_eq!(ctx.thread_state(id), Some(ThreadState::Ready));
        assert!(ctx.exit_thread(id));
        assert!(!ctx.exit_thread(id), "double exit is refused");
    }

    #[test]
    fn event_wait_times_out_and_then_succeeds() {
        let mut ctx = KernelContext::new();
        let event = ctx.create_event().expect("event");
        assert_eq!(ctx.wait_event(event, 2), WaitResult::TimedOut);
        assert_eq!(ctx.ticks(), 2);
        assert!(ctx.set_event(event));
        assert_eq!(ctx.wait_event(event, 0), WaitResult::Ready);
        assert!(ctx.clear_event(event));
        assert_eq!(ctx.wait_event(event, 0), WaitResult::TimedOut);
        assert_eq!(ctx.wait_event(999, 0), WaitResult::InvalidHandle);
    }

    #[test]
    fn mutex_refuses_self_deadlock_and_transfers_ownership() {
        let mut ctx = KernelContext::new();
        let mutex = ctx.create_mutex().expect("mutex");
        let a = ctx.create_thread().expect("thread a");
        let b = ctx.create_thread().expect("thread b");
        assert_eq!(ctx.lock_mutex(mutex, a, 0), WaitResult::Ready);
        assert_eq!(ctx.lock_mutex(mutex, a, 5), WaitResult::Deadlock);
        assert_eq!(ctx.lock_mutex(mutex, b, 2), WaitResult::TimedOut);
        assert_eq!(ctx.unlock_mutex(mutex, a), WaitResult::Ready);
        assert_eq!(ctx.mutex_owner(mutex), Some(b), "waiter inherits the lock");
        assert_eq!(ctx.unlock_mutex(mutex, b), WaitResult::Ready);
        assert_eq!(ctx.mutex_owner(mutex), None);
        assert_eq!(ctx.lock_mutex(mutex, 999, 0), WaitResult::Ready);
    }

    #[test]
    fn tables_are_bounded() {
        let mut ctx = KernelContext::new();
        for _ in 0..MAX_THREADS {
            assert!(ctx.create_thread().is_some());
        }
        assert!(ctx.create_thread().is_none());
        for _ in 0..MAX_EVENTS {
            assert!(ctx.create_event().is_some());
        }
        assert!(ctx.create_event().is_none());
        for _ in 0..MAX_MUTEXES {
            assert!(ctx.create_mutex().is_some());
        }
        assert!(ctx.create_mutex().is_none());
    }
}
