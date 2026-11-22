//! Xbox-style virtual memory layout — Phase 10.
//!
//! Models the region table a real Xbox kernel would program into the
//! MMU. The ARM64 interpreter maps these VAs through `va_base`
//! translation; page tables / TLB are *not* simulated (documented).

/// One memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// Region name.
    pub name: &'static str,
    /// Start address (inclusive).
    pub base: u64,
    /// End address (exclusive).
    pub end: u64,
    /// Access permissions.
    pub permissions: RegionPerms,
    /// Human description.
    pub description: &'static str,
}

/// RWX-style permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionPerms {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    /// Supervisor (kernel) only.
    pub supervisor: bool,
}

impl MemoryRegion {
    /// Whether `addr` lies inside the region.
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.end
    }
    /// Region size in bytes.
    pub fn size(&self) -> u64 {
        self.end - self.base
    }
}

// Region bases (Xbox Series X flavored, simplified).
pub const KERNEL_BASE: u64 = 0x8000_0000_0000;
pub const KERNEL_END: u64 = KERNEL_BASE + 0x10_0000; // 1 MiB
pub const USER_BASE: u64 = 0x8000_0100_0000;
pub const USER_END: u64 = 0x8000_1000_0000; // ~256 MiB window
pub const DEVICE_BASE: u64 = 0x8000_1000_0000;
pub const DEVICE_END: u64 = 0x8000_2000_0000; // 256 MiB device aperture
pub const PAGED_POOL_BASE: u64 = 0x8000_2000_0000;
pub const PAGED_POOL_END: u64 = 0x8000_2100_0000; // 16 MiB pool

const KERNEL_PERMS: RegionPerms = RegionPerms {
    read: true,
    write: true,
    execute: true,
    supervisor: true,
};
const USER_PERMS: RegionPerms = RegionPerms {
    read: true,
    write: true,
    execute: true,
    supervisor: false,
};
const DEVICE_PERMS: RegionPerms = RegionPerms {
    read: true,
    write: true,
    execute: false,
    supervisor: true,
};
const POOL_PERMS: RegionPerms = RegionPerms {
    read: true,
    write: true,
    execute: false,
    supervisor: true,
};

/// The full region table, low → high.
pub fn layout() -> Vec<MemoryRegion> {
    vec![
        MemoryRegion {
            name: "kernel",
            base: KERNEL_BASE,
            end: KERNEL_END,
            permissions: KERNEL_PERMS,
            description: "Kernel code + data",
        },
        MemoryRegion {
            name: "user",
            base: USER_BASE,
            end: USER_END,
            permissions: USER_PERMS,
            description: "Game process address space",
        },
        MemoryRegion {
            name: "device",
            base: DEVICE_BASE,
            end: DEVICE_END,
            permissions: DEVICE_PERMS,
            description: "GPU / storage / display aperture",
        },
        MemoryRegion {
            name: "paged-pool",
            base: PAGED_POOL_BASE,
            end: PAGED_POOL_END,
            permissions: POOL_PERMS,
            description: "Paged kernel heap",
        },
    ]
}

/// Find the region containing `addr`.
pub fn region_of(addr: u64) -> Option<MemoryRegion> {
    layout().into_iter().find(|r| r.contains(addr))
}

/// Which privilege level may access `addr` (supervisor = kernel mode).
pub fn accessible_by(addr: u64, supervisor: bool) -> bool {
    match region_of(addr) {
        Some(r) => supervisor || !r.permissions.supervisor,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_covers_prompt_regions() {
        let regions = layout();
        assert_eq!(regions.len(), 4);
        assert!(regions[0].contains(KERNEL_BASE));
        assert!(regions[1].contains(USER_BASE));
        assert!(regions[2].contains(DEVICE_BASE));
        assert!(regions[3].contains(PAGED_POOL_BASE));
        // Kernel region is supervisor-only.
        assert!(regions[0].permissions.supervisor);
        assert!(!regions[1].permissions.supervisor);
    }

    #[test]
    fn region_lookup_and_perms() {
        assert_eq!(region_of(USER_BASE).unwrap().name, "user");
        assert!(accessible_by(USER_BASE, false)); // user can touch user
        assert!(!accessible_by(KERNEL_BASE, false)); // user can't touch kernel
        assert!(accessible_by(KERNEL_BASE, true));
        assert!(!accessible_by(0xDEAD_BEEF, true)); // unmapped
    }
}
