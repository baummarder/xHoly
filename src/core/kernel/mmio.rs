//! MMIO region registry — Phase 10.
//!
//! Models the memory-mapped I/O apertures a guest would program:
//! GPU framebuffer, audio controller, storage (NVMe-ish), display.
//! Accesses are logged and can be hooked by devices; the console
//! UART region is handled by the CPU's `console_base` mechanism, so
//! it is *not* duplicated here.

/// A registered MMIO window.
#[derive(Debug, Clone)]
pub struct MmioRegion {
    /// Device/region name.
    pub name: String,
    /// Base guest address.
    pub base: u64,
    /// Size in bytes.
    pub size: u64,
    /// Whether reads are allowed.
    pub readable: bool,
    /// Whether writes are allowed.
    pub writable: bool,
}

impl MmioRegion {
    /// Contains `addr`?
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.base + self.size
    }
}

/// Standard Xbox-flavored MMIO apertures.
pub const GPU_FB_BASE: u64 = 0x8000_1000_0000; // inside DEVICE region
pub const GPU_FB_SIZE: u64 = 0x08_0000_0000; // 512 MiB aperture
pub const AUDIO_BASE: u64 = 0x8000_1800_0000;
pub const AUDIO_SIZE: u64 = 0x10_0000;
pub const STORAGE_BASE: u64 = 0x8000_1900_0000;
pub const STORAGE_SIZE: u64 = 0x10_0000;
pub const DISPLAY_BASE: u64 = 0x8000_1A00_0000;
pub const DISPLAY_SIZE: u64 = 0x10_0000;

/// Registry of MMIO regions.
#[derive(Debug, Default)]
pub struct MmioSpace {
    regions: Vec<MmioRegion>,
    /// Log of accesses (addr, is_write, size) — capped.
    pub access_log: Vec<(u64, bool, u32)>,
}

impl MmioSpace {
    /// Fresh space with the standard device apertures registered.
    pub fn with_defaults() -> Self {
        let mut s = Self::default();
        s.register(MmioRegion {
            name: "gpu-fb".into(),
            base: GPU_FB_BASE,
            size: GPU_FB_SIZE,
            readable: true,
            writable: true,
        });
        s.register(MmioRegion {
            name: "audio".into(),
            base: AUDIO_BASE,
            size: AUDIO_SIZE,
            readable: true,
            writable: true,
        });
        s.register(MmioRegion {
            name: "storage".into(),
            base: STORAGE_BASE,
            size: STORAGE_SIZE,
            readable: true,
            writable: true,
        });
        s.register(MmioRegion {
            name: "display".into(),
            base: DISPLAY_BASE,
            size: DISPLAY_SIZE,
            readable: true,
            writable: true,
        });
        s
    }

    /// Register a region.
    pub fn register(&mut self, region: MmioRegion) {
        self.regions.push(region);
    }

    /// Region containing `addr`, if any.
    pub fn region_at(&self, addr: u64) -> Option<&MmioRegion> {
        self.regions.iter().find(|r| r.contains(addr))
    }

    /// Record an access (called by the CPU's memory hooks).
    pub fn note_access(&mut self, addr: u64, is_write: bool, size: u32) {
        if self.access_log.len() < 4096 {
            self.access_log.push((addr, is_write, size));
        }
    }

    /// Validate a prospective access.
    pub fn check(&self, addr: u64, is_write: bool, size: u32) -> Result<&MmioRegion, String> {
        let region = self
            .region_at(addr)
            .ok_or_else(|| format!("no MMIO region at {addr:#x}"))?;
        if is_write && !region.writable {
            return Err(format!("MMIO {} not writable", region.name));
        }
        if !is_write && !region.readable {
            return Err(format!("MMIO {} not readable", region.name));
        }
        if addr + size as u64 > region.base + region.size {
            return Err(format!("MMIO {} access out of bounds", region.name));
        }
        Ok(region)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_regions_resolve() {
        let mmio = MmioSpace::with_defaults();
        assert_eq!(mmio.region_at(GPU_FB_BASE).unwrap().name, "gpu-fb");
        assert!(mmio.check(GPU_FB_BASE, true, 4).is_ok());
        assert!(mmio.check(AUDIO_BASE + 0x100, false, 4).is_ok());
        assert!(mmio.check(0x1234, true, 4).is_err()); // unmapped
    }
}
