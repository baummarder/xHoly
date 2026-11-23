//! Xbox kernel emulation stub — Phase 8/10.
//!
//! Simulates the *shape* of the Xbox kernel ABI on top of the real
//! AArch64 interpreter ([`crate::core::cpu::interpreter`]):
//!
//! - **Memory layout** — [`memory`] region table (kernel/user/device/
//!   paged-pool) as address constants.
//! - **System calls** — `SVC` instructions land here; x0 = call number,
//!   x1–x7 = arguments (the de-facto AArch64 ABI). svc#0–16 per the
//!   Phase-10 table; `SYS_TICKS` moved to 17 to free svc#8 for Suspend.
//! - **Scheduler** — [`scheduler`] round-robin with context snapshots.
//! - **Objects** — [`syscall::KernelObjects`] heaps/events/files/devices.
//! - **Interrupts** — [`interrupt::Gic`] timer + IRQ stub.
//! - **MMIO** — [`mmio::MmioSpace`] device apertures.
//! - **Console** — [`console::KernelConsole`] buffered kprint + panic.
//!
//! This is intentionally a *stub*: it services enough calls for
//! guest programs (console write, sleep, threads, heaps) — not the
//! real Xbox kernel ABI (which uses the XAM / executable-loader path,
//! not raw SVCs).

pub mod console;
pub mod interrupt;
pub mod memory;
pub mod mmio;
pub mod scheduler;
pub mod syscall;
pub mod xam;

pub use console::KernelConsole;
pub use interrupt::Gic;
pub use memory::{layout as memory_layout, region_of, MemoryRegion};
pub use mmio::MmioSpace;
pub use scheduler::{
    round_robin, Thread, THREAD_DEAD, THREAD_READY, THREAD_SLEEPING, THREAD_SUSPENDED,
};
pub use syscall::{KernelObjects, HEAP_ARENA_SIZE, HEAP_BASE};
pub use xam::{import_stub_va, XamImport, XamTable, IMPORT_STUB_BASE};

use crate::core::cpu::interpreter::{Arm64Cpu, Arm64Error, SvcCall};

// ---------------------------------------------------------------------------
// Memory layout constants (re-exported from memory.rs for compat)
// ---------------------------------------------------------------------------

pub use memory::{KERNEL_BASE, KERNEL_END, USER_BASE, USER_END};
/// Console/UART MMIO window inside guest memory.
pub const CONSOLE_BASE: u64 = 0x0000_F000_0000_0000;
pub const CONSOLE_SIZE: u64 = 0x1000;

// ---------------------------------------------------------------------------
// Guest framebuffer (Phase 18)
// ---------------------------------------------------------------------------

/// Guest framebuffer aperture inside the user VA window. Layout:
/// `[u32 magic][u32 width][u32 height][u32 seq]` header followed by
/// `width * height * 4` RGBA8 pixels. The guest writes frames with
/// plain STR instructions; the host blits them into the GUI.
pub const GUEST_FB_MAGIC: u32 = 0x5342_4658; // "XFBS" little-endian
/// Guest framebuffer base VA (USER_BASE + 8 MiB — above the loaded
/// image, below the heap arena).
pub const GUEST_FB_BASE: u64 = 0x8000_0180_0000;
/// Guest framebuffer size.
pub const GUEST_FB_W: u32 = 160;
pub const GUEST_FB_H: u32 = 120;
/// Header size in bytes.
pub const GUEST_FB_HEADER: usize = 16;
/// Total aperture size (header + pixels).
pub const GUEST_FB_SIZE: usize =
    GUEST_FB_HEADER + (GUEST_FB_W as usize) * (GUEST_FB_H as usize) * 4;

// ---------------------------------------------------------------------------
// Guest input aperture (Phase 22)
// ---------------------------------------------------------------------------

/// Guest VA of the input aperture. Host writes button/delta state here
/// before each frame; the guest reads it with plain `LDR W` instructions.
/// Sits just above the framebuffer aperture, inside the user VA window.
pub const GUEST_INPUT_BASE: u64 = 0x8000_0190_0000;
/// Input aperture size: buttons, dx, dy, sequence (four u32).
pub const GUEST_INPUT_SIZE: usize = 16;

/// Write host input state into the guest input aperture.
///
/// Bounds-checked against the kernel memory-layer region table and the
/// flat guest buffer, exactly like the framebuffer aperture.
pub fn write_guest_input(
    cpu: &mut Arm64Cpu,
    buttons: u32,
    dx: i32,
    dy: i32,
) -> Result<(), Arm64Error> {
    use memory::region_of;
    let region = region_of(GUEST_INPUT_BASE).ok_or(Arm64Error::MemoryFault {
        addr: GUEST_INPUT_BASE,
        write: true,
    })?;
    if !region.permissions.write || region.permissions.supervisor {
        return Err(Arm64Error::MemoryFault {
            addr: GUEST_INPUT_BASE,
            write: true,
        });
    }
    let rel = GUEST_INPUT_BASE
        .checked_sub(cpu.va_base)
        .ok_or(Arm64Error::MemoryFault {
            addr: GUEST_INPUT_BASE,
            write: true,
        })? as usize;
    if rel + GUEST_INPUT_SIZE > cpu.memory.len() {
        return Err(Arm64Error::MemoryFault {
            addr: GUEST_INPUT_BASE,
            write: true,
        });
    }
    let seq = cpu
        .memory
        .get(rel + 12..rel + 16)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .unwrap_or(0)
        .wrapping_add(1);
    cpu.memory[rel..rel + 4].copy_from_slice(&buttons.to_le_bytes());
    cpu.memory[rel + 4..rel + 8].copy_from_slice(&dx.to_le_bytes());
    cpu.memory[rel + 8..rel + 12].copy_from_slice(&dy.to_le_bytes());
    cpu.memory[rel + 12..rel + 16].copy_from_slice(&seq.to_le_bytes());
    Ok(())
}

// ---------------------------------------------------------------------------
// Syscall numbers — Phase-10 table (x0 = number, x1–x7 = args)
// ---------------------------------------------------------------------------

/// svc#0 — write bytes to the kernel console.
pub const SYS_WRITE: u64 = 0;
/// svc#1 — create a thread.
pub const SYS_CREATE_THREAD: u64 = 1;
/// svc#2 — create a process.
pub const SYS_CREATE_PROCESS: u64 = 2;
/// svc#3 — sleep `x1` ticks.
pub const SYS_SLEEP: u64 = 3;
/// svc#4 — write file.
pub const SYS_WRITE_FILE: u64 = 4;
/// svc#5 — read file.
pub const SYS_READ_FILE: u64 = 5;
/// svc#6 — yield to the next thread.
pub const SYS_YIELD: u64 = 6;
/// svc#7 — exit the current thread/process.
pub const SYS_EXIT: u64 = 7;
/// svc#8 — suspend a thread.
pub const SYS_SUSPEND: u64 = 8;
/// svc#9 — resume a thread.
pub const SYS_RESUME: u64 = 9;
/// svc#10 — create a heap.
pub const SYS_CREATE_HEAP: u64 = 10;
/// svc#11 — heap allocation.
pub const SYS_HEAP_ALLOC: u64 = 11;
/// svc#12 — heap free.
pub const SYS_HEAP_FREE: u64 = 12;
/// svc#13 — create event.
pub const SYS_CREATE_EVENT: u64 = 13;
/// svc#14 — open file.
pub const SYS_OPEN_FILE: u64 = 14;
/// svc#15 — create device.
pub const SYS_CREATE_DEVICE: u64 = 15;
/// svc#16 — set thread context.
pub const SYS_SET_CONTEXT: u64 = 16;
/// svc#17 — query ticks (xHoly extension; kept off the prompt's
/// svc#8 which is Suspend in the Phase-10 table).
pub const SYS_TICKS: u64 = 17;
/// svc#18 — end the current guest frame and yield to the host loop
/// (xHoly extension). The guest resumes at the following instruction
/// on the next frame; its registers and memory are preserved.
pub const SYS_FRAME_END: u64 = 18;

// ---------------------------------------------------------------------------
// Kernel
// ---------------------------------------------------------------------------

/// The kernel stub: syscall dispatch + thread scheduler + objects.
#[derive(Default)]
pub struct Kernel {
    /// All known threads (index 0 = main thread).
    pub threads: Vec<Thread>,
    /// Index of the currently running thread.
    pub current: usize,
    /// Monotonic tick counter.
    pub ticks: u64,
    /// Process records created via SYS_CREATE_PROCESS.
    pub processes: Vec<u64>,
    /// Console/file log lines.
    pub log: Vec<String>,
    /// Kernel objects: heaps/events/files/devices (Phase 10).
    pub objects: KernelObjects,
    /// Buffered kernel console.
    pub console: KernelConsole,
    /// Interrupt controller stub.
    pub gic: Gic,
    /// MMIO region registry.
    pub mmio: MmioSpace,
    /// XAM import table (Phase 17 stub).
    pub xam: XamTable,
    /// Guest memory regions currently mapped (Phase 17 memory layer).
    pub regions: Vec<memory::MemoryRegion>,
    /// Instruction budget per thread before a pre-emptive reschedule.
    pub preempt_budget: u64,
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
            objects: KernelObjects::new(),
            console: KernelConsole::default(),
            gic: Gic::new(),
            mmio: MmioSpace::with_defaults(),
            xam: XamTable::new(),
            regions: memory::layout(),
            preempt_budget: 1024,
        }
    }

    /// Register XEX imports resolved by the loader into the XAM table.
    pub fn register_imports<I>(&mut self, imports: I)
    where
        I: IntoIterator<Item = (String, String, u64)>,
    {
        for (library, name, thunk_va) in imports {
            self.xam.register(&library, &name, thunk_va);
            self.log
                .push(format!("xam: import {library}!{name} -> {thunk_va:#x}"));
        }
    }

    /// Pre-emptive scheduling tick: called by the CPU every
    /// [`Self::preempt_budget`] instructions. Saves the current thread
    /// context, rotates to the next ready thread and restores it.
    /// Returns `true` when a different thread is now running.
    ///
    /// This is the Phase-17 pre-emptive scheduler: unlike the
    /// cooperative yield (SYS_YIELD), the CPU can be interrupted
    /// mid-instruction-stream by the kernel tick.
    pub fn preempt_tick(&mut self, cpu: &mut Arm64Cpu) -> bool {
        if self.threads.len() <= 1 {
            return false;
        }
        let cur = self.current;
        if self.threads[cur].state != THREAD_READY {
            return false;
        }
        // Snapshot outgoing context.
        self.threads[cur] = Thread::from_cpu(self.threads[cur].id, cpu);
        self.threads[cur].state = THREAD_READY;
        // Pick next runnable thread (skip self unless nothing else).
        let next = scheduler::next_ready(&self.threads, cur);
        if next == cur {
            // No other runnable thread: restore the snapshot anyway
            // (context is identical, but keeps the invariant simple).
            self.threads[cur].restore(cpu);
            return false;
        }
        self.threads[next].restore(cpu);
        self.current = next;
        self.ticks += 1;
        true
    }

    /// Install this kernel as the CPU's SVC handler.
    pub fn attach(&mut self, cpu: &mut Arm64Cpu) {
        let kernel_ptr = self as *mut Kernel;
        cpu.svc_handler = Some(Box::new(move |cpu, call| {
            // SAFETY: kernel_ptr is valid for the lifetime of the CPU's
            // handler in this scaffold (single-threaded emulation).
            let kernel = unsafe { &mut *kernel_ptr };
            kernel.handle_syscall(cpu, call)
        }));
        cpu.console_base = CONSOLE_BASE;
        cpu.console_size = CONSOLE_SIZE;
        // Pre-emptive scheduling: the CPU calls back every
        // `preempt_budget` instructions; the kernel rotates threads.
        cpu.preempt_interval = self.preempt_budget.max(1);
        let kernel_ptr = self as *mut Kernel;
        cpu.preempt_handler = Some(Box::new(move |cpu| {
            // SAFETY: same single-threaded scaffold invariant as the
            // SVC handler above — kernel_ptr outlives the CPU handler.
            let kernel = unsafe { &mut *kernel_ptr };
            kernel.preempt_tick(cpu)
        }));
    }

    /// Dispatch one syscall. Returns `false` to halt the CPU (SYS_EXIT).
    pub fn handle_syscall(&mut self, cpu: &mut Arm64Cpu, call: &SvcCall) -> bool {
        self.ticks += 1;
        self.gic.tick(); // timer IRQ bookkeeping per syscall
        scheduler::age_sleepers(&mut self.threads);
        match call.args[0] {
            SYS_WRITE => {
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
                let entry = call.args[1];
                let mut t = Thread::from_cpu(self.threads.len() as u64, cpu);
                t.pc = entry;
                t.state = THREAD_READY;
                let id = t.id;
                self.threads.push(t);
                cpu.set_reg(0, id);
            }
            SYS_CREATE_PROCESS => {
                let entry = call.args[1];
                self.processes.push(entry);
                cpu.set_reg(0, self.processes.len() as u64 - 1);
            }
            SYS_SLEEP => {
                let ticks = call.args[1];
                let cur = &mut self.threads[self.current];
                cur.state = THREAD_SLEEPING;
                cur.sleep_ticks = ticks;
                self.schedule_next();
            }
            SYS_WRITE_FILE => {
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
                let buf = call.args[3];
                let len = call.args[4].min(4096);
                for i in 0..len {
                    let _ = cpu.mem_write_pub(buf + i, 0, 1);
                }
                cpu.set_reg(0, 0);
            }
            SYS_YIELD => self.schedule_next(),
            SYS_SUSPEND => {
                let tid = call.args[1];
                if let Some(t) = self.threads.iter_mut().find(|t| t.id == tid) {
                    t.state = THREAD_SUSPENDED;
                }
                cpu.set_reg(0, 0);
            }
            SYS_RESUME => {
                let tid = call.args[1];
                if let Some(t) = self.threads.iter_mut().find(|t| t.id == tid) {
                    if t.state == THREAD_SUSPENDED {
                        t.state = THREAD_READY;
                    }
                }
                cpu.set_reg(0, 0);
            }
            SYS_CREATE_HEAP => {
                let h = self.objects.create_heap(cpu, call.args[1]);
                cpu.set_reg(0, h);
            }
            SYS_HEAP_ALLOC => {
                let p = self.objects.heap_alloc(cpu, call.args[1], call.args[2]);
                cpu.set_reg(0, p);
            }
            SYS_HEAP_FREE => {
                let ok = self.objects.heap_free(call.args[1], call.args[2]);
                cpu.set_reg(0, ok as u64);
            }
            SYS_CREATE_EVENT => {
                cpu.set_reg(0, self.objects.create_event());
            }
            SYS_OPEN_FILE => {
                let h = self.objects.open_file(cpu, call.args[1], call.args[2]);
                cpu.set_reg(0, h);
            }
            SYS_CREATE_DEVICE => {
                let h = self.objects.create_device(cpu, call.args[1], call.args[2]);
                cpu.set_reg(0, h);
            }
            SYS_SET_CONTEXT => {
                let ctx = call.args[1];
                let r = self.objects.set_context(cpu, ctx);
                cpu.set_reg(0, r);
            }
            SYS_TICKS => {
                cpu.set_reg(0, self.ticks);
            }
            SYS_FRAME_END => {
                // Frame boundary: stop the current slice without tearing
                // down guest state. `GuestSession` clears `running` again
                // before the next frame, so the guest resumes here.
                cpu.running = false;
            }
            SYS_EXIT => {
                self.threads[self.current].state = THREAD_DEAD;
                if self.threads.iter().all(|t| t.state == THREAD_DEAD) {
                    return false;
                }
                self.schedule_next();
            }
            _ => {
                let msg = format!("unknown syscall #{}", call.args[0]);
                self.log.push(msg.clone());
                self.console.klog(msg);
            }
        }
        true
    }

    /// Round-robin step (delegates to [`scheduler::round_robin`]).
    pub fn schedule_next(&mut self) {
        round_robin(&mut self.threads, &mut self.current);
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
        assert_eq!(kernel.threads[0].state, THREAD_SLEEPING);
        assert_eq!(kernel.threads[0].sleep_ticks, 5);
    }

    #[test]
    fn preempt_tick_switches_ready_threads() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
        kernel.threads.push(Thread::from_cpu(1, &cpu));
        kernel.current = 0;
        assert!(kernel.preempt_tick(&mut cpu));
        assert_eq!(kernel.current, 1);
        // Single-threaded kernels never pre-empt.
        let mut solo = Kernel::new(&cpu);
        assert!(!solo.preempt_tick(&mut cpu));
    }

    #[test]
    fn scheduler_round_robins() {
        let cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
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

    #[test]
    fn extended_syscalls_dispatch() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        let mut kernel = Kernel::new(&cpu);
        // CreateHeap → handle in x0
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_CREATE_HEAP, 0, 0, 0, 0, 0, 0, 0],
            },
        );
        let heap = cpu.regs[0];
        assert_ne!(heap, 0);
        // HeapAlloc
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_HEAP_ALLOC, heap, 128, 0, 0, 0, 0, 0],
            },
        );
        assert!(cpu.regs[0] >= HEAP_BASE);
        // CreateEvent
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_CREATE_EVENT, 0, 0, 0, 0, 0, 0, 0],
            },
        );
        assert_ne!(cpu.regs[0], 0);
        // Suspend + Resume thread 0
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_SUSPEND, 0, 0, 0, 0, 0, 0, 0],
            },
        );
        assert_eq!(kernel.threads[0].state, THREAD_SUSPENDED);
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_RESUME, 0, 0, 0, 0, 0, 0, 0],
            },
        );
        assert_eq!(kernel.threads[0].state, THREAD_READY);
        // Ticks moved to svc#17
        kernel.handle_syscall(
            &mut cpu,
            &SvcCall {
                number: 0,
                args: [SYS_TICKS, 0, 0, 0, 0, 0, 0, 0],
            },
        );
        assert!(cpu.regs[0] > 0);
    }
}
