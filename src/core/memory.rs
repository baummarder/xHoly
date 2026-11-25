//! Guest memory management.
//!
//! Mimics the Xbox kernel memory layout: a large flat address space with
//! reserved regions for the hypervisor, GPU carve-out, and title memory.
//! Uses a bump-style arena over a flat buffer for the research scaffold.

use std::ops::{Index, IndexMut, Range};

/// Guest-visible memory size: 16 GiB addressable window.
/// (Xbox Series S|X has 10/16 GB of unified memory; we reserve the full
/// 64-bit-style layout with a sparse flat buffer for now.)
pub const GUEST_MEMORY_SIZE: usize = 16 * 1024 * 1024 * 1024;

/// Guest physical address type.
pub type GuestAddr = u64;

/// Reserved regions within the guest address space.
pub mod regions {
    /// Hypervisor / kernel reservation at the bottom of memory.
    pub const HYPERVISOR: u64 = 0x0000_0000_0000_0000;
    /// GPU carve-out (VRAM equivalent on unified memory).
    pub const GPU_CARVEOUT: u64 = 0x0000_0100_0000_0000;
    /// Title (game) memory region.
    pub const TITLE: u64 = 0x0000_0200_0000_0000;
    /// I/O aperture for memory-mapped device registers.
    pub const IO_APERTURE: u64 = 0x0000_FF00_0000_0000;
}

/// A flat guest memory arena.
///
/// The scaffold uses a 1 GiB resident buffer backed by the host OS; the
/// full 16 GiB address space is represented as an index range for future
/// sparse/paged allocation.
pub struct Memory {
    /// Resident backing store (1 GiB for the scaffold).
    pub(crate) data: Vec<u8>,
    /// Guest address where `data` is mapped.
    pub(crate) base: GuestAddr,
    image_size: usize,
}

impl Memory {
    /// Create guest memory mapped at [`regions::TITLE`].
    pub fn new() -> Self {
        Self {
            data: vec![0u8; 1024 * 1024 * 1024],
            base: regions::TITLE,
            image_size: 0,
        }
    }

    /// Map a loaded game image at the start of title memory.
    pub fn map_image(&mut self, image: &[u8]) {
        let n = image.len().min(self.data.len());
        self.data[..n].copy_from_slice(&image[..n]);
        self.image_size = n;
    }

    /// Size of the loaded image in bytes.
    pub fn image_size(&self) -> usize {
        self.image_size
    }

    /// Read a little-endian value of `size` bytes at `addr`.
    pub fn read(&self, addr: GuestAddr, size: usize) -> Option<u64> {
        let off = self.offset(addr)?;
        let bytes = self.data.get(off..off.checked_add(size)?)?;
        let mut v = 0u64;
        for (i, b) in bytes.iter().enumerate() {
            v |= (*b as u64) << (8 * i);
        }
        Some(v)
    }

    /// Write a little-endian `value` of `size` bytes at `addr`.
    pub fn write(&mut self, addr: GuestAddr, value: u64, size: usize) {
        if let Some(off) = self.offset(addr) {
            if let Some(dst) = self.data.get_mut(off..off + size) {
                for (i, b) in dst.iter_mut().enumerate() {
                    *b = (value >> (8 * i)) as u8;
                }
            }
        }
    }

    fn offset(&self, addr: GuestAddr) -> Option<usize> {
        let rel = addr.checked_sub(self.base)?;
        usize::try_from(rel).ok().filter(|o| *o < self.data.len())
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte-level access to a guest address space.
///
/// This is the seam the guest CPU fetches and executes through: every
/// address is a **guest** address, and implementations never accept or
/// return host pointers. Implementations exist for the flat resident
/// window ([`Memory`]), the xmem guest heap
/// ([`crate::core::hle::xmem::GuestHeap`]) and the routed façade used
/// by the guest execution pipeline.
pub trait GuestMemory {
    /// Read `len` bytes at `addr`, or `None` when the range is not
    /// readable (unmapped, out of bounds, protection denied).
    fn read_bytes(&self, addr: GuestAddr, len: usize) -> Option<Vec<u8>>;

    /// Write `data` at `addr`; `false` when the range is not writable.
    fn write_bytes(&mut self, addr: GuestAddr, data: &[u8]) -> bool;

    /// Read one little-endian `u64`.
    fn read_u64(&self, addr: GuestAddr) -> Option<u64> {
        let bytes = self.read_bytes(addr, 8)?;
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }

    /// Write one little-endian `u64`.
    fn write_u64(&mut self, addr: GuestAddr, value: u64) -> bool {
        self.write_bytes(addr, &value.to_le_bytes())
    }
}

impl GuestMemory for Memory {
    fn read_bytes(&self, addr: GuestAddr, len: usize) -> Option<Vec<u8>> {
        let off = self.offset(addr)?;
        let end = off.checked_add(len)?;
        self.data.get(off..end).map(<[u8]>::to_vec)
    }

    fn write_bytes(&mut self, addr: GuestAddr, data: &[u8]) -> bool {
        let Some(off) = self.offset(addr) else {
            return false;
        };
        let Some(end) = off.checked_add(data.len()) else {
            return false;
        };
        match self.data.get_mut(off..end) {
            Some(dst) => {
                dst.copy_from_slice(data);
                true
            }
            None => false,
        }
    }
}

impl Index<Range<GuestAddr>> for Memory {
    type Output = [u8];
    fn index(&self, range: Range<GuestAddr>) -> &[u8] {
        &self.data[range.start as usize..range.end as usize]
    }
}

impl IndexMut<Range<GuestAddr>> for Memory {
    fn index_mut(&mut self, range: Range<GuestAddr>) -> &mut [u8] {
        &mut self.data[range.start as usize..range.end as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_read_write() {
        let mut mem = Memory::new();
        let addr = regions::TITLE;
        mem.write(addr, 0xdead_beef, 4);
        assert_eq!(mem.read(addr, 4), Some(0xdead_beef));
    }

    #[test]
    fn out_of_range_reads_none() {
        let mem = Memory::new();
        assert_eq!(mem.read(regions::HYPERVISOR, 8), None);
    }

    #[test]
    fn image_mapping() {
        let mut mem = Memory::new();
        mem.map_image(b"\x01\x02\x03\x04");
        assert_eq!(mem.read(regions::TITLE, 4), Some(0x0403_0201));
    }

    #[test]
    fn guest_memory_trait_reads_and_writes_bytes() {
        let mut mem = Memory::new();
        assert!(mem.write_bytes(regions::TITLE, b"guest"));
        assert_eq!(
            mem.read_bytes(regions::TITLE, 5).as_deref(),
            Some(b"guest".as_slice())
        );
        assert!(mem.write_u64(regions::TITLE + 0x10, 0x0123_4567_89AB_CDEF));
        assert_eq!(
            mem.read_u64(regions::TITLE + 0x10),
            Some(0x0123_4567_89AB_CDEF)
        );
        // Guest addresses outside the resident window never resolve.
        assert_eq!(mem.read_bytes(regions::HYPERVISOR, 4), None);
        assert!(!mem.write_bytes(regions::IO_APERTURE, b"nope"));
        // A range crossing the end of the window is rejected as a whole.
        let near_end = regions::TITLE + (1024 * 1024 * 1024) - 4;
        assert_eq!(mem.read_bytes(near_end, 8), None);
        assert!(!mem.write_bytes(near_end, b"12345"));
    }
}
