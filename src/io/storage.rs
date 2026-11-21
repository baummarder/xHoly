//! Storage device emulation: virtual HDD/USB/media access.

use std::path::{Path, PathBuf};

/// A mounted virtual storage device.
#[derive(Debug, Clone)]
pub struct StorageDevice {
    /// Host directory backing the device.
    pub mount_point: PathBuf,
    /// Device kind reported to the guest OS.
    pub kind: DeviceKind,
    /// Read-only flag.
    pub read_only: bool,
}

/// Kind of storage device presented to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Internal title storage (game partition).
    InternalHdd,
    /// USB mass-storage device.
    Usb,
    /// Optical disc drive (DVD/Blu-ray emulation).
    Optical,
    /// Original-Xbox-style HDD partition.
    LegacyHdd,
}

/// Mount a host directory as a guest storage device.
pub fn mount(host_dir: &Path, kind: DeviceKind, read_only: bool) -> StorageDevice {
    StorageDevice {
        mount_point: host_dir.to_path_buf(),
        kind,
        read_only,
    }
}
