//! Kernel object tables + extended syscalls — Phase 10.
//!
//! Extends the Phase-8 syscall set (svc#0–8) with the prompt's
//! svc#8–16: Suspend/Resume, CreateHeap/HeapAlloc/HeapFree,
//! CreateEvent, OpenFile, CreateDevice, SetContext.
//!
//! Heaps are bump allocators over a guest-memory region; events,
//! files and devices are handle tables (handles are small integers —
//! the real Xbox kernel uses opaque pointers, we use ids for clarity).

use crate::core::cpu::interpreter::Arm64Cpu;

/// First guest address handed out by the heap allocator (inside the
/// user region, above the loaded image).
pub const HEAP_BASE: u64 = 0x8000_0200_0000;
/// Heap arena size.
pub const HEAP_ARENA_SIZE: u64 = 0x10_0000; // 1 MiB

/// A bump-allocator heap.
#[derive(Debug)]
pub struct Heap {
    /// Heap handle (returned to the guest).
    pub handle: u64,
    /// Next free guest address.
    pub next: u64,
    /// Arena end.
    pub end: u64,
    /// Live allocations (ptr → size) for HeapFree bookkeeping.
    pub allocs: Vec<(u64, u64)>,
}

/// Kernel-side object tables.
#[derive(Debug, Default)]
pub struct KernelObjects {
    /// Allocated heaps.
    pub heaps: Vec<Heap>,
    /// Event handles (ids; state not yet modeled).
    pub events: Vec<u64>,
    /// Open file handles (paths seen via OpenFile).
    pub files: Vec<String>,
    /// Device handles (names).
    pub devices: Vec<String>,
    /// Suspended thread ids.
    pub suspended: Vec<u64>,
    /// Next synthetic handle value.
    next_handle: u64,
    /// Bump pointer for the shared heap arena.
    arena_next: u64,
}

impl KernelObjects {
    /// New tables; heap arena starts at HEAP_BASE.
    pub fn new() -> Self {
        Self {
            heaps: Vec::new(),
            events: Vec::new(),
            files: Vec::new(),
            devices: Vec::new(),
            suspended: Vec::new(),
            next_handle: 1,
            arena_next: HEAP_BASE,
        }
    }

    fn alloc_handle(&mut self) -> u64 {
        let h = self.next_handle;
        self.next_handle += 1;
        h
    }

    /// svc#10 CreateHeap — returns a handle; arena is shared.
    pub fn create_heap(&mut self, _cpu: &mut Arm64Cpu, _arg: u64) -> u64 {
        let handle = self.alloc_handle();
        self.heaps.push(Heap {
            handle,
            next: self.arena_next,
            end: HEAP_BASE + HEAP_ARENA_SIZE,
            allocs: Vec::new(),
        });
        handle
    }

    /// svc#11 HeapAlloc — bump-allocate `size` bytes in the heap.
    /// Returns guest pointer or 0 on exhaustion.
    pub fn heap_alloc(&mut self, cpu: &mut Arm64Cpu, heap: u64, size: u64) -> u64 {
        let size = size.max(8);
        let Some(h) = self.heaps.iter_mut().find(|h| h.handle == heap) else {
            return 0;
        };
        let aligned = (h.next + 15) & !15;
        if aligned + size > h.end {
            return 0;
        }
        // Zero the allocation in guest memory (best-effort).
        for i in 0..size.min(4096) {
            let _ = cpu.mem_write_pub(aligned + i, 0, 1);
        }
        h.next = aligned + size;
        h.allocs.push((aligned, size));
        // Advance the shared arena pointer past this heap's usage.
        if self.arena_next < h.next {
            self.arena_next = h.next;
        }
        aligned
    }

    /// svc#12 HeapFree — mark the allocation freed (bump heaps can't
    /// reclaim, but we drop the bookkeeping entry).
    pub fn heap_free(&mut self, heap: u64, ptr: u64) -> bool {
        let Some(h) = self.heaps.iter_mut().find(|h| h.handle == heap) else {
            return false;
        };
        let before = h.allocs.len();
        h.allocs.retain(|(p, _)| *p != ptr);
        h.allocs.len() != before
    }

    /// svc#13 CreateEvent — returns an event handle.
    pub fn create_event(&mut self) -> u64 {
        let h = self.alloc_handle();
        self.events.push(h);
        h
    }

    /// svc#14 OpenFile — registers a path, returns a file handle.
    pub fn open_file(&mut self, cpu: &Arm64Cpu, path_ptr: u64, path_len: u64) -> u64 {
        let mut path = String::new();
        for i in 0..path_len.min(260) {
            match cpu.mem_read_pub(path_ptr + i, 1) {
                Ok(0) | Err(_) => break,
                Ok(b) => path.push(b as u8 as char),
            }
        }
        let h = self.alloc_handle();
        self.files.push(format!("{h}:{path}"));
        h
    }

    /// svc#15 CreateDevice — registers a device name.
    pub fn create_device(&mut self, cpu: &Arm64Cpu, name_ptr: u64, name_len: u64) -> u64 {
        let mut name = String::new();
        for i in 0..name_len.min(64) {
            match cpu.mem_read_pub(name_ptr + i, 1) {
                Ok(0) | Err(_) => break,
                Ok(b) => name.push(b as u8 as char),
            }
        }
        let h = self.alloc_handle();
        self.devices.push(format!("{h}:{name}"));
        h
    }

    /// svc#8 Suspend / svc#9 Resume on a thread id.
    pub fn suspend(&mut self, thread_id: u64) -> bool {
        self.suspended.push(thread_id);
        true
    }

    /// svc#9 Resume.
    pub fn resume(&mut self, thread_id: u64) -> bool {
        self.suspended.retain(|&t| t != thread_id);
        true
    }

    /// svc#16 SetContext — stub: acknowledge only.
    pub fn set_context(&mut self, _cpu: &mut Arm64Cpu, _ctx: u64) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_alloc_and_free() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        let mut objs = KernelObjects::new();
        let heap = objs.create_heap(&mut cpu, 0);
        assert_ne!(heap, 0);
        let p = objs.heap_alloc(&mut cpu, heap, 256);
        assert!(p >= HEAP_BASE);
        assert!(objs.heap_free(heap, p));
        assert!(!objs.heap_free(heap, 0xDEAD));
    }

    #[test]
    fn events_files_devices_handles() {
        let mut cpu = Arm64Cpu::new(64 * 1024);
        cpu.memory[0x100] = b'/';
        cpu.memory[0x101] = b'd';
        cpu.memory[0x102] = b'e';
        cpu.memory[0x103] = b'v';
        cpu.memory[0x104] = 0;
        let mut objs = KernelObjects::new();
        let ev = objs.create_event();
        let f = objs.open_file(&cpu, 0x100, 4);
        let d = objs.create_device(&cpu, 0x100, 4);
        assert!(ev > 0 && f > 0 && d > 0);
        assert_eq!(objs.events.len(), 1);
        assert_eq!(objs.files.len(), 1);
        assert_eq!(objs.devices.len(), 1);
        assert!(objs.suspend(7) && objs.resume(7));
        assert!(objs.suspended.is_empty());
    }
}
