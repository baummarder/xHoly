//! Round-robin thread scheduler — Phase 10.
//!
//! Extracted from `kernel/mod.rs`: the thread context type plus the
//! scheduling policy (sleep aging + next-ready rotation).

/// A thread = a full architectural snapshot.
#[derive(Debug, Clone)]
pub struct Thread {
    /// Thread id.
    pub id: u64,
    /// Saved X0–X30.
    pub regs: [u64; 31],
    /// Saved SP / PC / LR.
    pub sp: u64,
    /// Saved PC.
    pub pc: u64,
    /// State flags.
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// State: 0 = ready, 1 = sleeping, 2 = dead, 3 = suspended.
    pub state: u8,
    /// Ticks remaining before a sleeping thread wakes.
    pub sleep_ticks: u64,
}

/// Thread states.
pub const THREAD_READY: u8 = 0;
pub const THREAD_SLEEPING: u8 = 1;
pub const THREAD_DEAD: u8 = 2;
pub const THREAD_SUSPENDED: u8 = 3;

impl Thread {
    /// Fresh ready thread.
    pub fn blank(id: u64) -> Self {
        Self {
            id,
            regs: [0; 31],
            sp: 0,
            pc: 0,
            n: false,
            z: false,
            c: false,
            v: false,
            state: THREAD_READY,
            sleep_ticks: 0,
        }
    }

    /// Snapshot the live CPU into a thread context.
    pub fn from_cpu(id: u64, cpu: &crate::core::cpu::interpreter::Arm64Cpu) -> Self {
        Self {
            id,
            regs: cpu.regs,
            sp: cpu.sp,
            pc: cpu.pc,
            n: cpu.n,
            z: cpu.z,
            c: cpu.c,
            v: cpu.v,
            ..Self::blank(id)
        }
    }

    /// Restore this context into the CPU.
    pub fn restore(&self, cpu: &mut crate::core::cpu::interpreter::Arm64Cpu) {
        cpu.regs = self.regs;
        cpu.sp = self.sp;
        cpu.pc = self.pc;
        cpu.n = self.n;
        cpu.z = self.z;
        cpu.c = self.c;
        cpu.v = self.v;
    }
}

/// Age sleep timers: decrement `sleep_ticks` on sleeping threads and
/// wake those that reach zero. Returns the number woken.
pub fn age_sleepers(threads: &mut [Thread]) -> usize {
    let mut woken = 0;
    for t in threads.iter_mut() {
        if t.state == THREAD_SLEEPING && t.sleep_ticks > 0 {
            t.sleep_ticks -= 1;
            if t.sleep_ticks == 0 {
                t.state = THREAD_READY;
                woken += 1;
            }
        }
    }
    woken
}

/// Pick the next ready thread index after `current` (round-robin).
/// Returns `current` unchanged if no other thread is runnable.
pub fn next_ready(threads: &[Thread], current: usize) -> usize {
    let n = threads.len();
    if n <= 1 {
        return current;
    }
    for i in 1..=n {
        let idx = (current + i) % n;
        if threads[idx].state == THREAD_READY {
            return idx;
        }
    }
    current
}

/// Round-robin step: advance `current` to the next ready thread.
/// Sleep aging is *not* done here — it happens once per kernel tick
/// ([`age_sleepers`]) so a fresh `Sleep(n)` keeps all `n` ticks.
pub fn round_robin(threads: &mut [Thread], current: &mut usize) {
    *current = next_ready(threads, *current);
}

/// Count threads in each state (diagnostics).
pub fn state_counts(threads: &[Thread]) -> (usize, usize, usize, usize) {
    let mut c = (0, 0, 0, 0);
    for t in threads {
        match t.state {
            THREAD_READY => c.0 += 1,
            THREAD_SLEEPING => c.1 += 1,
            THREAD_DEAD => c.2 += 1,
            _ => c.3 += 1,
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_robin_rotates_and_wakes() {
        let mut threads = vec![
            Thread {
                state: THREAD_SLEEPING,
                sleep_ticks: 2,
                ..Thread::blank(0)
            },
            Thread {
                state: THREAD_READY,
                ..Thread::blank(1)
            },
        ];
        let mut cur = 0usize;
        round_robin(&mut threads, &mut cur);
        assert_eq!(cur, 1);
        // Age sleepers explicitly (kernel tick), then rotate.
        age_sleepers(&mut threads);
        round_robin(&mut threads, &mut cur);
        age_sleepers(&mut threads);
        round_robin(&mut threads, &mut cur);
        assert_eq!(threads[0].state, THREAD_READY);
        assert_eq!(threads[0].sleep_ticks, 0);
    }

    #[test]
    fn suspended_not_scheduled() {
        let threads = vec![
            Thread {
                state: THREAD_READY,
                ..Thread::blank(0)
            },
            Thread {
                state: THREAD_SUSPENDED,
                ..Thread::blank(1)
            },
        ];
        assert_eq!(next_ready(&threads, 0), 0); // only ready is self
    }
}
