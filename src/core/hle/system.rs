//! Deterministic xHoly VM system information — Milestone 36.4.
//!
//! **These are xHoly VM services, not Xbox system APIs.** No host OS
//! version, CPU model, clock epoch or frequency is exposed as console
//! information. The only time source here is the same monotonic VM tick
//! used by [`super::kernel::KernelContext`], and all info fields are
//! constants defined by this project.
//!
//! Exposed through the separate `xh_vm.sys.dll` registry module.

/// xHoly VM interface version (owned by this project, not an Xbox version).
pub const VM_VERSION_MAJOR: u64 = 1;
/// Minor interface version.
pub const VM_VERSION_MINOR: u64 = 0;

/// Static, deterministic information returned by `VmInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmInfo {
    /// Interface major version.
    pub major: u64,
    /// Interface minor version.
    pub minor: u64,
    /// Maximum number of guest threads (policy constant).
    pub max_threads: u64,
    /// Maximum number of guest events.
    pub max_events: u64,
    /// Maximum number of guest mutexes.
    pub max_mutexes: u64,
}

impl Default for VmInfo {
    fn default() -> Self {
        Self {
            major: VM_VERSION_MAJOR,
            minor: VM_VERSION_MINOR,
            max_threads: super::kernel::MAX_THREADS as u64,
            max_events: super::kernel::MAX_EVENTS as u64,
            max_mutexes: super::kernel::MAX_MUTEXES as u64,
        }
    }
}

/// Monotonic, deterministic system tick source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemContext {
    ticks: u64,
}

impl SystemContext {
    /// New context at tick zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current VM tick.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Advance by one deterministic service step.
    pub fn advance(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Return the static, project-owned VM information.
    pub fn info(&self) -> VmInfo {
        VmInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_is_static_and_project_owned() {
        let ctx = SystemContext::new();
        let info = ctx.info();
        assert_eq!(info.major, 1);
        assert_eq!(info.minor, 0);
        assert_eq!(info.max_threads, 64);
        assert_eq!(info, VmInfo::default());
    }

    #[test]
    fn ticks_are_monotonic_and_deterministic() {
        let mut ctx = SystemContext::new();
        assert_eq!(ctx.ticks(), 0);
        for expected in 1..=5 {
            ctx.advance();
            assert_eq!(ctx.ticks(), expected);
        }
        // Two contexts advanced equally stay identical — no wall clock.
        let mut other = SystemContext::new();
        for _ in 0..5 {
            other.advance();
        }
        assert_eq!(other, ctx);
    }
}
