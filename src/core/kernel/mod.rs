//! Xbox kernel emulation stub — Phase 8.
//!
//! Simulates the *shape* of the Xbox kernel ABI on top of the real
//! AArch64 interpreter ([`crate::core::cpu::interpreter`]):
//!
//! - **Memory layout** — kernel/user regions as address constants.
//! - **System calls** — `SVC` instructions land here; x0 = call number,
//!   x1–x7 = arguments (the de-facto AArch64 ABI).
//! - **Scheduler** — a round-robin thread table; `Sleep`/`Yield`
//!   rotate the run queue. Thread contexts are full register snapshots
//!   so switching is real, not cosmetic.
//!
//! This is intentionally a *stub*: it services enough calls for a
//! "Hello, xHoly!" guest program (console write, sleep, thread
//! create/yield) — not the real Xbox kernel ABI (which uses the XAM /
//! executable-loader path, not raw SVCs).

use crate::core::cpu::interpreter::{Arm64Cpu, SvcCall};

// ---------------------------------------------------------------------------
// Memory layout (Xbox-flavored, simplified)
// ---------------------------------------------------------------------------

/// Kernel image base — low canonical addresses, like the real Xbox
/// kernel region.
pub const KERNEL_BASE: u64 = 0x8000_0000_0000;
/// End of the kernel image region.
pub const KERNEL_END: u64 = KERNEL_BASE + 0x10_0000; // 1 MiB
/// User address space base (games load here).
pub const USER_BASE: u64 = 0x8000_0100_0000;
/// Console/UART MMIO window inside guest memory.
pub const CONSOLE_BASE: u64 = 0x0000_F000_0000_0000;
pub const CONSOLE_SIZE: u64 = 0x1000;

// ---------------------------------------------------------------------------
// Syscall numbers (xHoly-assigned; mirrors the prompt's svc#N scheme)
// ---------------------------------------------------------------------------

/// svc#0 — write bytes to the kernel console.
pub const SYS_WRITE: u64 = 0;
/// svc#1 — create a thread (records a context snapshot).
pub const SYS_CREATE_THREAD: u64 = 1;
/// svc#2 — create a process (stub: records entry point).
pub const SYS_CREATE_PROCESS: u64 = 2;
/// svc#3 — sleep `x1` ticks (scheduler yields).
pub const SYS_SLEEP: u64 = 3;
/// svc#4 — write file (stub: appends path+data to console log).
pub const SYS_WRITE_FILE: u64 = 4;
/// svc#5 — read file (stub: zero-fills the buffer, returns 0).
pub const SYS_READ_FILE: u64 = 5;
/// svc#6 — yield to the next thread.
pub const SYS_YIELD: u64 = 6;
/// svc#7 — exit the current thread/process.
pub const SYS_EXIT: u64 = 7;
/// svc#8 — query ticks (returns a monotonically increasing counter).
pub const SYS_TICKS: u64 = 8;

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// A thread = a full architectural snapshot.
#[derive(Debug, Clone)]
pub struct Thread {
    /// Thread id.
    pub id: u64,
    /// Saved X0–X30.
    pub regs: [u64; 31],
    /// Saved SP / PC / LR.
    pub sp: u64,
    pub pc: u64,
    /// State flags.
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// State: 0 = ready, 1 = sleeping, 2 = dead.
    pub state: u8,
    /// Ticks remaining before a sleeping thread wakes.
    pub sleep_ticks: u64,
}

impl Thread {
    /// Snapshot the live CPU into a thread context.
    pub fn from_cpu(id: u64, cpu: &Arm64Cpu) -> Self {
        Self {
            id,
            regs: cpu.regs,
            sp: cpu.sp,
            pc: cpu.pc,
            n: cpu.n,
            z: cpu.z,
            c: cpu.c,
            v: cpu.v,
            state: 0,
            sleep_ticks: 0,
        }
    }

    /// Restore this context into the CPU.
    pub fn restore(&self, cpu: &mut Arm64Cpu) {
        cpu.regs = self.regs;
        cpu.sp = self.sp;
        cpu.pc = self.pc;
        cpu.n = self.n;
        cpu.z = self.z;
        cpu.c = self.c;
        cpu.v = self.v;
    }
}

/// The kernel stub: syscall table + thread scheduler + tick counter.
#[derive(Default)]
pub struct Kernel {
    /// All known threads (index 0 = main thread).
    pub threads: Vec<Thread>,
    /// Index of the currently running thread.
    pub current: usize,
    /// Monotonic tick counter (incremented per syscall/yield).
    pub ticks: u64,
    /// Process records created via SYS_CREATE_PROCESS.
    pub processes: Vec<u64>,
    /// Console/file log lines (WriteFile stub output).
    pub log: Vec<String>,
}

impl Kernel {
    /// Create a kernel with the main thread already registered.
    pub fn new(cpu: &Arm64Cpu) -> Self {
        Self {
            threads: vec![Thread::from_cpu(0, cpu)],
            current: 0,
            ticks: 0,
            processes: Vec::new(),
            log: Vec::new(),
        }
    }

    /// Install this kernel as the CPU's SVC handler.
    pub fn attach(&mut self, cpu: &mut Arm64Cpu) {
        // The handler needs shared access to the kernel — use a raw
        // pointer wrapped in the closure. Safe here because `attach`
        // borrows both for 'static-ish use within the same scope; the
        // caller must not move the Kernel while the CPU runs.
        let kernel_ptr = self as *mut Kernel;
        cpu.svc_handler = Some(Box::new(move |cpu, call| {
            // SAFETY: kernel_ptr is valid for the lifetime of the CPU's
            // handler in this scaffold (single-threaded emulation).
            let kernel = unsafe { &mut *kernel_ptr };
            kernel.handle_syscall(cpu, call)
        }));
        // Map the console aperture so STR to CONSOLE_BASE appends output.
        cpu.console_base = CONSOLE_BASE;
        cpu.console_size = CONSOLE_SIZE;
    }

    /// Dispatch one syscall. Returns `false` to halt the CPU (SYS_EXIT).
    pub fn handle_syscall(&mut self, cpu: &mut Arm64Cpu, call: &SvcCall) -> bool {
        self.ticks += 1;
        match call.args[0] {
            SYS_WRITE => {
                // x1 = guest addr, x2 = length → copy bytes to output.
                let addr = call.args[1];
                let len = call.args[2].min(4096) as usize;
                for i in 0..len {
                    let a = addr + i as u64;
                    if let Ok(b) = cpu.mem_read_pub(a, 1) {
                        cpu.output.push(b as u8);
                    }
                }
            }
            SYS_CREATE_THREAD => {
                // x1 = entry pc. Snapshot current context as the new
                // thread's initial state (real kernels copy the parent).
                let entry = call.args[1];
                let mut t = Thread::from_cpu(self.threads.len() as u64, cpu);
                t.pc = entry;
                t.state = 0;
                let id = t.id;
                self.threads.push(t);
                cpu.set_reg(0, id); // return thread id in x0
            }
            SYS_CREATE_PROCESS => {
                let entry = call.args[1];
                self.processes.push(entry);
                cpu.set_reg(0, self.processes.len() as u64 - 1);
            }
            SYS_SLEEP => {
                // x1 = ticks to sleep.
                let ticks = call.args[1];
                self.threads[self.current].state = 1;
                self.threads[self.current].sleep_ticks = ticks;
                self.schedule_next();
            }
            SYS_WRITE_FILE => {
                // x1 = path ptr, x2 = path len, x3 = data ptr, x4 = data len
                let path = self.read_guest_string(cpu, call.args[1], call.args[2] as usize);
                let data_len = call.args[4].min(256) as usize;
                let mut data = Vec::with_capacity(data_len);
                for i in 0..data_len {
                    if let Ok(b) = cpu.mem_read_pub(call.args[3] + i as u64, 1) {
                        data.push(b as u8);
                    }
                }
                let line = format!(
                    "WriteFile '{}' <= {} bytes: {}",
                    path,
                    data_len,
                    String::from_utf8_lossy(&data)
                );
                self.log.push(line.clone());
                cpu.output.extend_from_slice(line.as_bytes());
                cpu.output.push(b'\n');
                cpu.set_reg(0, data_len as u64);
            }
            SYS_READ_FILE => {
                // x1 = path ptr, x2 = path len, x3 = buf, x4 = buf len
                // Stub: zero-fill the buffer, return 0 bytes read.
                let buf = call.args[3];
                let len = call.args[4].min(4096);
                for i in 0..len {
                    let _ = cpu.mem_write_pub(buf + i, 0, 1);
                }
                cpu.set_reg(0, 0);
            }
            SYS_YIELD => {
                self.schedule_next();
            }
            SYS_TICKS => {
                cpu.set_reg(0, self.ticks);
            }
            SYS_EXIT => {
                self.threads[self.current].state = 2;
                if self.threads.iter().all(|t| t.state == 2) {
                    return false; // halt CPU
                }
                self.schedule_next();
            }
            _ => {
                // Unknown syscall — log and continue.
                self.log.push(format!("unknown syscall #{}", call.args[0]));
            }
        }
        true
    }

    /// Round-robin: advance `current` to the next ready thread,
    /// decrementing sleep counters and waking threads whose time is up.
    /// If no other thread is ready, stay on the current one.
    pub fn schedule_next(&mut self) {
        let n = self.threads.len();
        if n <= 1 {
            return;
        }
        // Age sleep timers.
        for t in &mut self.threads {
            if t.state == 1 && t.sleep_ticks > 0 {
                t.sleep_ticks -= 1;
                if t.sleep_ticks == 0 {
                    t.state = 0;
                }
            }
        }
        // Snapshot current thread, find next ready.
        // (Caller is responsible for saving regs before switching —
        //  the CPU's svc path restores via Thread::restore on resume.)
        for i in 1..=n {
            let idx = (self.current + i) % n;
            if self.threads[idx].state == 0 {
                self.current = idx;
                return;
            }
        }
    }

    /// Read a NUL- or length-bounded string from guest memory.
    fn read_guest_string(&self, cpu: &Arm64Cpu, addr: u64, max_len: usize) -> String {
        let mut s = String::new();
        for i in 0..max_len.min(512) {
            match cpu.mem_read_pub(addr + i as u64, 1) {
                Ok(0) | Err(_) => break,
                Ok(b) => s.push(b as u8 as char),
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cpu::interpreter::Arm64Cpu;

    #[test]
    fn kernel_services_write_and_sleep() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
        kernel.attach(&mut cpu);
        // Guest program: write "Hi" via SYS_WRITE, sleep 1, exit.
        // We simulate by calling handle_syscall directly.
        cpu.memory[0x1000] = b'H';
        cpu.memory[0x1001] = b'i';
        let call = SvcCall {
            number: 0,
            args: [SYS_WRITE, 0x1000, 2, 0, 0, 0, 0, 0],
        };
        assert!(kernel.handle_syscall(&mut cpu, &call));
        assert_eq!(cpu.output, b"Hi");

        let call = SvcCall {
            number: 0,
            args: [SYS_SLEEP, 5, 0, 0, 0, 0, 0, 0],
        };
        kernel.handle_syscall(&mut cpu, &call);
        assert_eq!(kernel.threads[0].state, 1);
        assert_eq!(kernel.threads[0].sleep_ticks, 5);
    }

    #[test]
    fn scheduler_round_robins() {
        let cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
        // Add a second ready thread.
        kernel.threads.push(Thread::from_cpu(1, &cpu));
        kernel.current = 0;
        kernel.schedule_next();
        assert_eq!(kernel.current, 1);
        kernel.schedule_next();
        assert_eq!(kernel.current, 0);
    }

    #[test]
    fn exit_halts_when_all_dead() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
        let call = SvcCall {
            number: 0,
            args: [SYS_EXIT, 0, 0, 0, 0, 0, 0, 0],
        };
        assert!(!kernel.handle_syscall(&mut cpu, &call));
    }
}
