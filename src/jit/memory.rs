//! Guest memory manager for the JIT runtime — Phase 13.
//!
//! Flat byte-addressable memory implementing the address-space layout
//! from the spec: code @0x400000, data @0x401000, heap @0x402000
//! (grows up), stack descending from 0x7FFF_FFFF. Bounds-checked —
//! out-of-range access returns [`JitError::MemoryOverflow`].

use super::{JitError, JitResult};

/// Code segment base (WASM code / translated blocks live here).
pub const CODE_BASE: u64 = 0x400000;
/// Data segment base.
pub const DATA_BASE: u64 = 0x401000;
/// Heap base; grows upward.
pub const HEAP_BASE: u64 = 0x402000;
/// Initial heap size: 64 KiB.
pub const HEAP_INIT: u64 = 64 * 1024;
/// Stack top (stack grows down).
pub const STACK_TOP: u64 = 0x7FFF_FFFF;
/// Stack size reserved below STACK_TOP.
pub const STACK_SIZE: u64 = 64 * 1024;

/// Flat guest memory with a managed heap.
#[derive(Debug)]
pub struct MemoryManager {
    /// Backing store covering [0, mem_size).
    mem: Vec<u8>,
    /// Current heap break (next free heap byte).
    heap_break: u64,
}

impl MemoryManager {
    /// New memory sized to cover code+data+heap+stack regions.
    /// `code_size`/`data_size` reserve the low segments.
    pub fn new(code_size: u64, data_size: u64) -> Self {
        let heap_end = HEAP_BASE + HEAP_INIT;
        let need = heap_end.max(STACK_TOP + 1);
        let _ = (code_size, data_size); // layout constants; segments overlap low mem
        Self {
            mem: vec![0u8; need as usize],
            heap_break: HEAP_BASE,
        }
    }

    /// Total memory size in bytes.
    pub fn size(&self) -> u64 {
        self.mem.len() as u64
    }

    /// Read `len` bytes at `addr` (little-endian for ints).
    pub fn read(&self, addr: u64, len: usize) -> JitResult<Vec<u8>> {
        let end = addr
            .checked_add(len as u64)
            .ok_or(JitError::MemoryOverflow(addr))?;
        if end > self.mem.len() as u64 {
            return Err(JitError::MemoryOverflow(addr));
        }
        Ok(self.mem[addr as usize..end as usize].to_vec())
    }

    /// Write `data` at `addr`.
    pub fn write(&mut self, addr: u64, data: &[u8]) -> JitResult<()> {
        let end = addr
            .checked_add(data.len() as u64)
            .ok_or(JitError::MemoryOverflow(addr))?;
        if end > self.mem.len() as u64 {
            return Err(JitError::MemoryOverflow(addr));
        }
        self.mem[addr as usize..end as usize].copy_from_slice(data);
        Ok(())
    }

    /// Read a u64.
    pub fn read_u64(&self, addr: u64) -> JitResult<u64> {
        let b = self.read(addr, 8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }

    /// Write a u64.
    pub fn write_u64(&mut self, addr: u64, v: u64) -> JitResult<()> {
        self.write(addr, &v.to_le_bytes())
    }

    /// Bump-allocate `size` bytes on the heap. Returns the address.
    pub fn heap_alloc(&mut self, size: u64) -> JitResult<u64> {
        let aligned = (self.heap_break + 15) & !15;
        let end = aligned
            .checked_add(size)
            .ok_or(JitError::MemoryOverflow(aligned))?;
        let limit = HEAP_BASE + HEAP_INIT;
        if end > limit {
            return Err(JitError::MemoryOverflow(end));
        }
        self.heap_break = end;
        Ok(aligned)
    }

    /// Current heap break.
    pub fn heap_break(&self) -> u64 {
        self.heap_break
    }

    /// Push `value` onto the guest stack (grows down from STACK_TOP).
    pub fn push(&mut self, sp: &mut u64, value: u64) -> JitResult<()> {
        *sp = sp.checked_sub(8).ok_or(JitError::MemoryOverflow(*sp))?;
        if *sp < STACK_TOP + 1 - STACK_SIZE {
            return Err(JitError::MemoryOverflow(*sp));
        }
        self.write_u64(*sp, value)
    }

    /// Pop from the guest stack.
    pub fn pop(&mut self, sp: &mut u64) -> JitResult<u64> {
        let v = self.read_u64(*sp)?;
        *sp += 8;
        if *sp > STACK_TOP + 1 {
            return Err(JitError::MemoryOverflow(*sp));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_roundtrip() {
        let mut m = MemoryManager::new(0, 0);
        m.write_u64(DATA_BASE, 0xDEADBEEF).unwrap();
        assert_eq!(m.read_u64(DATA_BASE).unwrap(), 0xDEADBEEF);
    }

    #[test]
    fn out_of_bounds_rejected() {
        let mut m = MemoryManager::new(0, 0);
        assert!(m.write_u64(u64::MAX - 4, 1).is_err());
        assert!(m.read(0, usize::MAX).is_err());
    }

    #[test]
    fn heap_and_stack() {
        let mut m = MemoryManager::new(0, 0);
        let a = m.heap_alloc(32).unwrap();
        assert!(a >= HEAP_BASE);
        let mut sp = STACK_TOP + 1;
        m.push(&mut sp, 42).unwrap();
        assert_eq!(m.pop(&mut sp).unwrap(), 42);
    }
}
