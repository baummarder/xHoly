//! Gen9 `xmem` foundation — guest-address-space heap (Phase 28).
//!
//! This is xHoly's own guest memory bookkeeping. It is **not** a copy of
//! any Microsoft implementation and it does **not** decode the XMem
//! attribute bitfields: the public GDK documentation shows the call shape
//! (`XMemAlloc(size, attributes)` / `XMemFree(pointer, attributes)`) but
//! the `XALLOC_ATTRIBUTES` layout lives in access-restricted reference
//! pages, so attributes are recorded verbatim and left uninterpreted.
//!
//! Guest addresses and host memory are kept strictly apart: every
//! allocation has a guest address inside [`XMEM_HEAP_BASE`]..+`size` and a
//! separate host-side backing buffer that is never exposed to the guest.

use std::collections::BTreeMap;
use std::fmt;

/// Guest address of the first byte of the HLE heap region.
///
/// Placed directly above the 1 GiB resident window of
/// [`crate::core::memory::regions::TITLE`] so it cannot overlap a mapped
/// game image.
pub const XMEM_HEAP_BASE: u64 = 0x0000_0200_4000_0000;

/// Size of the HLE heap region (8 GiB of the title window).
pub const XMEM_HEAP_SIZE: u64 = 8 * 1024 * 1024 * 1024;

/// Alignment used by the XMem dispatch shims.
///
/// The real alignment is encoded in the (undecoded) XMem attribute word;
/// 16 bytes is xHoly's documented fallback, not a claim about Xbox
/// behaviour.
pub const DEFAULT_ALIGNMENT: u64 = 16;

/// Resident host-memory cap for a single heap.
///
/// xHoly scaffold policy: reporting `OutOfMemory` instead of exhausting
/// host RAM. Not an Xbox limit.
pub const RESIDENT_LIMIT: u64 = 1024 * 1024 * 1024;

/// Lifecycle state of an allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationState {
    /// Allocation is live and its backing buffer exists.
    Active,
    /// Allocation was freed; the record is kept to detect double frees.
    Freed,
}

/// Protection metadata tracked per allocation (xHoly-managed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// Reads and writes allowed.
    ReadWrite,
    /// Reads allowed, writes rejected.
    ReadOnly,
    /// Neither reads nor writes allowed.
    NoAccess,
}

impl Protection {
    /// Whether a write through [`GuestHeap::write`] is permitted.
    pub fn allows_write(self) -> bool {
        matches!(self, Self::ReadWrite)
    }

    /// Whether a read through [`GuestHeap::read`] is permitted.
    pub fn allows_read(self) -> bool {
        !matches!(self, Self::NoAccess)
    }
}

/// Bookkeeping record for one guest allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    /// Size in bytes.
    pub size: u64,
    /// Alignment the guest address honours.
    pub alignment: u64,
    /// Raw XMem attribute word as passed by the caller (not decoded).
    pub attributes: u64,
    /// Lifecycle state.
    pub state: AllocationState,
    /// Protection metadata.
    pub protection: Protection,
}

/// Errors produced by the guest heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapError {
    /// Zero-sized allocation or zero-length access.
    InvalidSize,
    /// Alignment is not a power of two.
    InvalidAlignment,
    /// Region or resident cap would be exceeded.
    OutOfMemory,
    /// Address is outside every active allocation.
    InvalidAddress,
    /// Protection metadata forbids this access.
    AccessDenied,
    /// Freeing an address that was already freed.
    DoubleFree,
    /// Freeing an address that was never allocated.
    NotAllocated,
}

impl fmt::Display for HeapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::InvalidSize => "invalid size",
            Self::InvalidAlignment => "alignment must be a power of two",
            Self::OutOfMemory => "guest heap exhausted",
            Self::InvalidAddress => "address outside any active allocation",
            Self::AccessDenied => "protection metadata denies this access",
            Self::DoubleFree => "allocation already freed",
            Self::NotAllocated => "address is not the base of a live allocation",
        };
        f.write_str(text)
    }
}

/// Snapshot of heap usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeapStats {
    /// Number of live allocations.
    pub active: usize,
    /// Number of freed allocations still recorded.
    pub freed: usize,
    /// Bytes currently handed out to live allocations.
    pub bytes_used: u64,
    /// Bytes of host backing memory currently resident.
    pub resident_bytes: u64,
}

/// Guest-visible metadata for one live allocation (M35 diagnostics).
///
/// Derived from the existing allocation records: guest base and size,
/// alignment, the raw attribute word (never decoded) and protection.
/// No host backing address is ever returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationInfo {
    /// Guest VA of the allocation base.
    pub base: u64,
    /// Size in bytes.
    pub size: u64,
    /// Alignment used for the allocation.
    pub alignment: u64,
    /// Raw XMem attribute word, recorded verbatim.
    pub attributes: u64,
    /// Current protection metadata.
    pub protection: Protection,
}

/// A bump-allocated guest heap with per-allocation host backing buffers.
#[derive(Debug)]
pub struct GuestHeap {
    base: u64,
    size: u64,
    /// Next free offset from `base` (monotonic; freed space is not reused).
    cursor: u64,
    /// Guest base address → allocation metadata.
    allocations: BTreeMap<u64, Allocation>,
    /// Guest base address → host backing buffer.
    backing: BTreeMap<u64, Vec<u8>>,
    bytes_used: u64,
}

fn align_up(value: u64, alignment: u64) -> Option<u64> {
    let mask = alignment - 1;
    value.checked_add(mask).map(|v| v & !mask)
}

impl GuestHeap {
    /// Create an empty heap covering `[base, base + size)`.
    pub fn new(base: u64, size: u64) -> Self {
        Self {
            base,
            size,
            cursor: 0,
            allocations: BTreeMap::new(),
            backing: BTreeMap::new(),
            bytes_used: 0,
        }
    }

    /// Create the default heap used by the HLE dispatch runtime.
    pub fn with_defaults() -> Self {
        Self::new(XMEM_HEAP_BASE, XMEM_HEAP_SIZE)
    }

    /// First guest address of the heap region.
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Size of the heap region in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Allocate `size` bytes with the given alignment, recording the raw
    /// XMem `attributes` word verbatim.
    pub fn alloc(&mut self, size: u64, alignment: u64, attributes: u64) -> Result<u64, HeapError> {
        if size == 0 {
            return Err(HeapError::InvalidSize);
        }
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(HeapError::InvalidAlignment);
        }
        let absolute = self
            .base
            .checked_add(self.cursor)
            .ok_or(HeapError::OutOfMemory)?;
        let aligned = align_up(absolute, alignment).ok_or(HeapError::OutOfMemory)?;
        let end_offset = (aligned - self.base)
            .checked_add(size)
            .ok_or(HeapError::OutOfMemory)?;
        if end_offset > self.size {
            return Err(HeapError::OutOfMemory);
        }
        if self.bytes_used.saturating_add(size) > RESIDENT_LIMIT {
            return Err(HeapError::OutOfMemory);
        }
        let host_size = usize::try_from(size).map_err(|_| HeapError::OutOfMemory)?;
        let addr = aligned;
        self.cursor = end_offset;
        self.bytes_used += size;
        self.allocations.insert(
            addr,
            Allocation {
                size,
                alignment,
                attributes,
                state: AllocationState::Active,
                protection: Protection::ReadWrite,
            },
        );
        self.backing.insert(addr, vec![0u8; host_size]);
        Ok(addr)
    }

    /// Free a live allocation. `addr` must be the exact base address.
    pub fn free(&mut self, addr: u64) -> Result<(), HeapError> {
        let Some(allocation) = self.allocations.get_mut(&addr) else {
            return Err(HeapError::NotAllocated);
        };
        if allocation.state == AllocationState::Freed {
            return Err(HeapError::DoubleFree);
        }
        allocation.state = AllocationState::Freed;
        self.backing.remove(&addr);
        self.bytes_used = self.bytes_used.saturating_sub(allocation.size);
        Ok(())
    }

    /// Look up `(base, allocation)` for the allocation whose range
    /// contains `addr`.
    fn entry_containing(&self, addr: u64) -> Option<(u64, &Allocation)> {
        self.allocations
            .range(..=addr)
            .next_back()
            .map(|(base, alloc)| (*base, alloc))
            .filter(|(base, alloc)| addr < base.saturating_add(alloc.size))
    }

    /// Look up the allocation whose range contains `addr`.
    pub fn allocation_containing(&self, addr: u64) -> Option<&Allocation> {
        self.entry_containing(addr).map(|(_, alloc)| alloc)
    }

    /// Look up the allocation registered at exactly `addr`.
    pub fn allocation_at(&self, addr: u64) -> Option<&Allocation> {
        self.allocations.get(&addr)
    }

    /// Query metadata for the live allocation containing `addr`.
    ///
    /// Any address inside the allocation resolves to its base. Unknown or
    /// freed addresses fail with `InvalidAddress` rather than returning a
    /// fabricated record.
    pub fn query(&self, addr: u64) -> Result<AllocationInfo, HeapError> {
        let (base, alloc) = self
            .entry_containing(addr)
            .ok_or(HeapError::InvalidAddress)?;
        if alloc.state != AllocationState::Active {
            return Err(HeapError::InvalidAddress);
        }
        Ok(AllocationInfo {
            base,
            size: alloc.size,
            alignment: alloc.alignment,
            attributes: alloc.attributes,
            protection: alloc.protection,
        })
    }

    /// Check that `[addr, addr + len)` lies inside one live allocation.
    pub fn validate_range(&self, addr: u64, len: u64) -> Result<(), HeapError> {
        if len == 0 {
            return Err(HeapError::InvalidSize);
        }
        let end = addr.checked_add(len).ok_or(HeapError::InvalidAddress)?;
        let (base, alloc) = self
            .entry_containing(addr)
            .ok_or(HeapError::InvalidAddress)?;
        if alloc.state != AllocationState::Active {
            return Err(HeapError::InvalidAddress);
        }
        if end > base.saturating_add(alloc.size) {
            return Err(HeapError::InvalidAddress);
        }
        Ok(())
    }

    /// Read `len` bytes from guest memory.
    pub fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, HeapError> {
        if len == 0 {
            return Err(HeapError::InvalidSize);
        }
        self.validate_range(addr, len as u64)?;
        let (base, alloc) = self
            .entry_containing(addr)
            .ok_or(HeapError::InvalidAddress)?;
        if !alloc.protection.allows_read() {
            return Err(HeapError::AccessDenied);
        }
        let offset = (addr - base) as usize;
        let buf = self.backing.get(&base).ok_or(HeapError::NotAllocated)?;
        buf.get(offset..offset + len)
            .map(<[u8]>::to_vec)
            .ok_or(HeapError::InvalidAddress)
    }

    /// Write `data` into guest memory.
    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), HeapError> {
        if data.is_empty() {
            return Err(HeapError::InvalidSize);
        }
        self.validate_range(addr, data.len() as u64)?;
        let (base, alloc) = self
            .entry_containing(addr)
            .ok_or(HeapError::InvalidAddress)?;
        if !alloc.protection.allows_write() {
            return Err(HeapError::AccessDenied);
        }
        let offset = (addr - base) as usize;
        let buf = self.backing.get_mut(&base).ok_or(HeapError::NotAllocated)?;
        let slot = buf
            .get_mut(offset..offset + data.len())
            .ok_or(HeapError::InvalidAddress)?;
        slot.copy_from_slice(data);
        Ok(())
    }

    /// Update protection metadata for the allocation containing `addr`.
    pub fn set_protection(&mut self, addr: u64, protection: Protection) -> Result<(), HeapError> {
        let base = self
            .entry_containing(addr)
            .filter(|(_, alloc)| alloc.state == AllocationState::Active)
            .map(|(base, _)| base)
            .ok_or(HeapError::InvalidAddress)?;
        let alloc = self
            .allocations
            .get_mut(&base)
            .ok_or(HeapError::InvalidAddress)?;
        alloc.protection = protection;
        Ok(())
    }

    /// Usage snapshot.
    pub fn stats(&self) -> HeapStats {
        let (active, freed) = self
            .allocations
            .values()
            .fold((0usize, 0usize), |(a, f), alloc| match alloc.state {
                AllocationState::Active => (a + 1, f),
                AllocationState::Freed => (a, f + 1),
            });
        HeapStats {
            active,
            freed,
            bytes_used: self.bytes_used,
            resident_bytes: self.bytes_used,
        }
    }
}

impl crate::core::memory::GuestMemory for GuestHeap {
    fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
        self.read(addr, len).ok()
    }

    fn write_bytes(&mut self, addr: u64, data: &[u8]) -> bool {
        self.write(addr, data).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap() -> GuestHeap {
        GuestHeap::new(XMEM_HEAP_BASE, 64 * 1024)
    }

    #[test]
    fn allocates_aligned_addresses_inside_the_region() {
        let mut h = heap();
        let a = h.alloc(100, 16, 0x1234).unwrap();
        let b = h.alloc(10, 64, 0).unwrap();
        assert_eq!(a, XMEM_HEAP_BASE);
        assert_eq!(b % 64, 0, "64-byte alignment must hold");
        assert!(b >= a + 100);
        assert!(a >= XMEM_HEAP_BASE && b < XMEM_HEAP_BASE + 64 * 1024);
    }

    #[test]
    fn records_attributes_verbatim_without_decoding() {
        let mut h = heap();
        let a = h.alloc(8, 16, 0xDEAD_BEEF_CAFE_F00D).unwrap();
        let alloc = h.allocation_at(a).unwrap();
        assert_eq!(alloc.attributes, 0xDEAD_BEEF_CAFE_F00D);
        assert_eq!(alloc.state, AllocationState::Active);
        assert_eq!(alloc.protection, Protection::ReadWrite);
    }

    #[test]
    fn rejects_zero_size_and_bad_alignment() {
        let mut h = heap();
        assert_eq!(h.alloc(0, 16, 0), Err(HeapError::InvalidSize));
        assert_eq!(h.alloc(16, 3, 0), Err(HeapError::InvalidAlignment));
        assert_eq!(h.alloc(16, 0, 0), Err(HeapError::InvalidAlignment));
        assert_eq!(h.alloc(u64::MAX, 16, 0), Err(HeapError::OutOfMemory));
    }

    #[test]
    fn region_and_resident_limits_are_enforced() {
        let mut h = heap();
        assert_eq!(h.alloc(64 * 1024 + 1, 16, 0), Err(HeapError::OutOfMemory));
        let mut big = GuestHeap::new(XMEM_HEAP_BASE, XMEM_HEAP_SIZE);
        assert_eq!(
            big.alloc(RESIDENT_LIMIT + 1, 16, 0),
            Err(HeapError::OutOfMemory)
        );
    }

    #[test]
    fn free_lifecycle_detects_double_free_and_unknown_addresses() {
        let mut h = heap();
        let a = h.alloc(32, 16, 0).unwrap();
        assert_eq!(h.free(a + 1), Err(HeapError::NotAllocated));
        assert_eq!(h.free(a), Ok(()));
        assert_eq!(h.free(a), Err(HeapError::DoubleFree));
        assert_eq!(h.free(0x1), Err(HeapError::NotAllocated));
        let stats = h.stats();
        assert_eq!(stats.active, 0);
        assert_eq!(stats.freed, 1);
        assert_eq!(stats.bytes_used, 0);
    }

    #[test]
    fn read_write_roundtrip_with_bounds_checks() {
        let mut h = heap();
        let a = h.alloc(16, 16, 0).unwrap();
        h.write(a, b"hello xHoly").unwrap();
        assert_eq!(h.read(a, 11).unwrap(), b"hello xHoly");
        // Writing past the end of the allocation must fail.
        assert_eq!(
            h.write(a + 12, b"overflow!"),
            Err(HeapError::InvalidAddress)
        );
        assert_eq!(h.read(a + 16, 1), Err(HeapError::InvalidAddress));
        assert_eq!(h.read(0xdead, 4), Err(HeapError::InvalidAddress));
        assert_eq!(h.read(a, 0), Err(HeapError::InvalidSize));
        // Freed memory is no longer addressable.
        h.free(a).unwrap();
        assert_eq!(h.read(a, 4), Err(HeapError::InvalidAddress));
        assert_eq!(h.write(a, b"x"), Err(HeapError::InvalidAddress));
    }

    #[test]
    fn protection_metadata_is_enforced() {
        let mut h = heap();
        let a = h.alloc(16, 16, 0).unwrap();
        h.set_protection(a, Protection::ReadOnly).unwrap();
        assert_eq!(h.write(a, b"nope"), Err(HeapError::AccessDenied));
        assert_eq!(h.read(a, 2).unwrap(), vec![0, 0]);
        h.set_protection(a, Protection::NoAccess).unwrap();
        assert_eq!(h.read(a, 2), Err(HeapError::AccessDenied));
        assert_eq!(
            h.set_protection(a + 4096, Protection::ReadOnly),
            Err(HeapError::InvalidAddress)
        );
    }

    #[test]
    fn validate_range_rejects_crossing_allocation_bounds() {
        let mut h = heap();
        let a = h.alloc(32, 16, 0).unwrap();
        assert_eq!(h.validate_range(a, 32), Ok(()));
        assert_eq!(h.validate_range(a + 31, 2), Err(HeapError::InvalidAddress));
        assert_eq!(h.validate_range(a - 1, 2), Err(HeapError::InvalidAddress));
        assert_eq!(h.validate_range(a, 0), Err(HeapError::InvalidSize));
    }

    #[test]
    fn implements_the_guest_memory_trait() {
        use crate::core::memory::GuestMemory;
        let mut h = heap();
        let a = h.alloc(32, 16, 0).unwrap();
        assert!(h.write_u64(a, 0x0123_4567_89AB_CDEF));
        assert_eq!(h.read_u64(a), Some(0x0123_4567_89AB_CDEF));
        // Guest addresses inside the region but without an allocation
        // never resolve, and neither do addresses outside the region.
        assert_eq!(h.read_bytes(XMEM_HEAP_BASE + 0x8000, 8), None);
        assert!(!h.write_bytes(XMEM_HEAP_BASE + 0x8000, b"x"));
        assert_eq!(h.read_bytes(XMEM_HEAP_BASE - 8, 8), None);
        // Freed memory is no longer accessible through the trait.
        h.free(a).unwrap();
        assert_eq!(h.read_bytes(a, 8), None);
    }
}
