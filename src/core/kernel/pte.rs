//! Page-table layer (Phase 24 foundation).
//!
//! A small but real software page-table model: encoded 64-bit PTEs with
//! present/write/execute/user bits, accessed + dirty tracking, memory
//! attributes, and explicit page faults. This is the host-side memory
//! layer a retail-boot path needs before any MMU simulation can run.

use std::collections::BTreeMap;
use std::fmt;

/// Bit flags encoded in a 64-bit PTE.
pub mod bits {
    /// Page is present / valid.
    pub const PRESENT: u64 = 1 << 0;
    /// Software may write.
    pub const WRITE: u64 = 1 << 1;
    /// Instruction fetches allowed.
    pub const EXECUTE: u64 = 1 << 2;
    /// User-mode accessible.
    pub const USER: u64 = 1 << 3;
    /// Page was accessed (set by the walker).
    pub const ACCESSED: u64 = 1 << 4;
    /// Page was written (set by the walker).
    pub const DIRTY: u64 = 1 << 5;
    /// Copy-on-write marker.
    pub const COW: u64 = 1 << 6;
    /// Guarded page (access always faults).
    pub const GUARDED: u64 = 1 << 7;
    /// Cache-inhibited.
    pub const NO_CACHE: u64 = 1 << 8;
    /// Physical address mask (4 KiB-aligned).
    pub const PA_MASK: u64 = !0xFFFu64;
}

/// Decoded page attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageAttributes {
    /// Writable.
    pub write: bool,
    /// Executable.
    pub execute: bool,
    /// User accessible.
    pub user: bool,
    /// Cache inhibited (device-like memory).
    pub no_cache: bool,
    /// Copy-on-write.
    pub cow: bool,
    /// Guarded — every access faults.
    pub guarded: bool,
}

impl Default for PageAttributes {
    fn default() -> Self {
        Self {
            write: true,
            execute: false,
            user: true,
            no_cache: false,
            cow: false,
            guarded: false,
        }
    }
}

/// One software page-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTableEntry {
    /// Virtual page number (already page-aligned).
    pub va: u64,
    /// Physical page base (page-aligned).
    pub pa: u64,
    /// Present flag.
    pub present: bool,
    /// Access flags.
    pub attrs: PageAttributes,
    /// Whether the page was accessed since mapping.
    pub accessed: bool,
    /// Whether the page was written since mapping.
    pub dirty: bool,
}

impl PageTableEntry {
    /// Encode into the 64-bit hardware-style PTE word.
    pub fn encode(&self) -> u64 {
        let mut word = self.pa & bits::PA_MASK;
        if self.present {
            word |= bits::PRESENT;
        }
        if self.attrs.write {
            word |= bits::WRITE;
        }
        if self.attrs.execute {
            word |= bits::EXECUTE;
        }
        if self.attrs.user {
            word |= bits::USER;
        }
        if self.accessed {
            word |= bits::ACCESSED;
        }
        if self.dirty {
            word |= bits::DIRTY;
        }
        if self.attrs.cow {
            word |= bits::COW;
        }
        if self.attrs.guarded {
            word |= bits::GUARDED;
        }
        if self.attrs.no_cache {
            word |= bits::NO_CACHE;
        }
        word
    }

    /// Decode a hardware-style PTE word for a virtual address.
    pub fn decode(va: u64, word: u64) -> Self {
        Self {
            va,
            pa: word & bits::PA_MASK,
            present: word & bits::PRESENT != 0,
            attrs: PageAttributes {
                write: word & bits::WRITE != 0,
                execute: word & bits::EXECUTE != 0,
                user: word & bits::USER != 0,
                cow: word & bits::COW != 0,
                guarded: word & bits::GUARDED != 0,
                no_cache: word & bits::NO_CACHE != 0,
            },
            accessed: word & bits::ACCESSED != 0,
            dirty: word & bits::DIRTY != 0,
        }
    }
}

/// Kind of memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// Data read.
    Read,
    /// Data write.
    Write,
    /// Instruction fetch.
    Execute,
}

/// Why an access faulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// No PTE for the VA.
    NotPresent,
    /// Write to a read-only page.
    WriteViolation,
    /// Fetch from a non-executable page.
    ExecuteViolation,
    /// User access to a supervisor page.
    PrivilegeViolation,
    /// Guarded page.
    Guarded,
}

/// One recorded page fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFault {
    /// Faulting VA.
    pub va: u64,
    /// Access that faulted.
    pub access: AccessKind,
    /// Reason.
    pub kind: FaultKind,
}

impl fmt::Display for PageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "page fault ({:?}) at {:#x} on {:?}",
            self.kind, self.va, self.access
        )
    }
}

/// A sparse software page table.
pub struct PageTable {
    /// Page size in bytes (4 KiB by default).
    pub page_size: u64,
    /// Present entries, keyed by virtual page base.
    entries: BTreeMap<u64, PageTableEntry>,
    /// Fault history (capped at 64).
    pub faults: Vec<PageFault>,
}

impl PageTable {
    /// Create a table with the given page size (must be a power of two).
    pub fn new(page_size: u64) -> Self {
        assert!(page_size.is_power_of_two() && page_size >= 0x1000);
        Self {
            page_size,
            entries: BTreeMap::new(),
            faults: Vec::new(),
        }
    }

    fn page_base(&self, va: u64) -> u64 {
        va & !(self.page_size - 1)
    }

    /// Map a virtual page to a physical page with attributes.
    pub fn map(&mut self, va: u64, pa: u64, attrs: PageAttributes) {
        let base = self.page_base(va);
        let pa_base = pa & !(self.page_size - 1);
        self.entries.insert(
            base,
            PageTableEntry {
                va: base,
                pa: pa_base,
                present: true,
                attrs,
                accessed: false,
                dirty: false,
            },
        );
    }

    /// Remove a mapping.
    pub fn unmap(&mut self, va: u64) {
        self.entries.remove(&self.page_base(va));
    }

    /// Number of present pages.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a PTE by VA.
    pub fn entry(&self, va: u64) -> Option<&PageTableEntry> {
        self.entries.get(&self.page_base(va))
    }

    /// Translate `va` for `access`, updating accessed/dirty bits and
    /// recording a fault on failure.
    pub fn translate(&mut self, va: u64, access: AccessKind, user: bool) -> Result<u64, PageFault> {
        let base = self.page_base(va);
        let offset = va - base;
        let fault = |kind| PageFault { va, access, kind };
        let Some(entry) = self.entries.get_mut(&base) else {
            let f = fault(FaultKind::NotPresent);
            self.push_fault(f);
            return Err(f);
        };
        if !entry.present {
            let f = fault(FaultKind::NotPresent);
            self.push_fault(f);
            return Err(f);
        }
        if entry.attrs.guarded {
            let f = fault(FaultKind::Guarded);
            self.push_fault(f);
            return Err(f);
        }
        if user && !entry.attrs.user {
            let f = fault(FaultKind::PrivilegeViolation);
            self.push_fault(f);
            return Err(f);
        }
        match access {
            AccessKind::Write if !entry.attrs.write => {
                let f = fault(FaultKind::WriteViolation);
                self.push_fault(f);
                return Err(f);
            }
            AccessKind::Execute if !entry.attrs.execute => {
                let f = fault(FaultKind::ExecuteViolation);
                self.push_fault(f);
                return Err(f);
            }
            _ => {}
        }
        entry.accessed = true;
        if access == AccessKind::Write {
            entry.dirty = true;
        }
        Ok(entry.pa + offset)
    }

    fn push_fault(&mut self, fault: PageFault) {
        if self.faults.len() == 64 {
            self.faults.remove(0);
        }
        self.faults.push(fault);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pte_encode_decode_roundtrip() {
        let attrs = PageAttributes {
            write: true,
            execute: true,
            user: false,
            no_cache: true,
            cow: true,
            guarded: false,
        };
        let pte = PageTableEntry {
            va: 0x2000,
            pa: 0x9000,
            present: true,
            attrs,
            accessed: true,
            dirty: true,
        };
        let decoded = PageTableEntry::decode(0x2000, pte.encode());
        assert_eq!(decoded, pte);
    }

    #[test]
    fn translate_sets_accessed_and_dirty() {
        let mut table = PageTable::new(0x1000);
        table.map(0x2000, 0x9000, PageAttributes::default());
        assert_eq!(table.translate(0x2010, AccessKind::Read, true), Ok(0x9010));
        assert!(!table.entry(0x2000).unwrap().dirty);
        table.translate(0x2020, AccessKind::Write, true).unwrap();
        let pte = table.entry(0x2000).unwrap();
        assert!(pte.accessed && pte.dirty);
    }

    #[test]
    fn violations_record_faults() {
        let mut table = PageTable::new(0x1000);
        table.map(
            0x3000,
            0xA000,
            PageAttributes {
                write: false,
                ..Default::default()
            },
        );
        assert_eq!(
            table.translate(0x3000, AccessKind::Write, true),
            Err(PageFault {
                va: 0x3000,
                access: AccessKind::Write,
                kind: FaultKind::WriteViolation
            })
        );
        assert_eq!(table.faults.len(), 1);
        assert_eq!(
            table.translate(0x5000, AccessKind::Read, true),
            Err(PageFault {
                va: 0x5000,
                access: AccessKind::Read,
                kind: FaultKind::NotPresent
            })
        );
        assert_eq!(table.faults.len(), 2);
    }

    #[test]
    fn unmap_removes_translation() {
        let mut table = PageTable::new(0x1000);
        table.map(0x1000, 0xB000, PageAttributes::default());
        assert_eq!(table.len(), 1);
        table.unmap(0x1100);
        assert!(table.is_empty());
    }
}
