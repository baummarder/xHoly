//! Storage device model — NVMe/HDD/SSD with latency profiles.

/// Drive technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    /// NVMe SSD (fastest, lowest latency).
    Nvme,
    /// SATA SSD.
    SataSsd,
    /// HDD (slow, high latency).
    Hdd,
}

/// Errors.
#[derive(Debug)]
pub enum StorageError {
    /// Device not found.
    UnknownDevice(String),
    /// Device offline/not mounted.
    Offline(String),
    /// IO error.
    Io(std::io::Error),
    /// Capacity exceeded.
    Full,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDevice(d) => write!(f, "unknown device {d}"),
            Self::Offline(d) => write!(f, "device {d} offline"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Full => write!(f, "device full"),
        }
    }
}
impl std::error::Error for StorageError {}
impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A storage device.
#[derive(Debug, Clone)]
pub struct StorageDevice {
    /// Device name/id.
    pub name: String,
    /// Kind.
    pub kind: DriveKind,
    /// Capacity bytes.
    pub capacity: u64,
    /// Used bytes.
    pub used: u64,
    /// Sequential read MB/s.
    pub seq_read_mbps: f64,
    /// Sequential write MB/s.
    pub seq_write_mbps: f64,
    /// Average read latency µs.
    pub read_latency_us: u64,
    /// Mounted?
    pub mounted: bool,
}

impl StorageDevice {
    /// New device.
    pub fn new(name: impl Into<String>, kind: DriveKind, capacity_gb: u64) -> Self {
        let (seq_read, seq_write, lat) = match kind {
            DriveKind::Nvme => (3500.0, 3000.0, 80),
            DriveKind::SataSsd => (550.0, 520.0, 120),
            DriveKind::Hdd => (160.0, 160.0, 8000),
        };
        Self {
            name: name.into(),
            kind,
            capacity: capacity_gb * 1024 * 1024 * 1024,
            used: 0,
            seq_read_mbps: seq_read,
            seq_write_mbps: seq_write,
            read_latency_us: lat,
            mounted: true,
        }
    }

    /// Free space.
    pub fn free(&self) -> u64 {
        self.capacity.saturating_sub(self.used)
    }
    /// Usage fraction 0..1.
    pub fn usage(&self) -> f64 {
        if self.capacity == 0 {
            0.0
        } else {
            self.used as f64 / self.capacity as f64
        }
    }
    /// Can fit `bytes`?
    pub fn can_fit(&self, bytes: u64) -> bool {
        self.free() >= bytes
    }
    /// Estimate read time for `bytes` (ms).
    pub fn est_read_ms(&self, bytes: u64) -> f64 {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        let bw_ms = mb / self.seq_read_mbps * 1000.0;
        let lat_ms = self.read_latency_us as f64 / 1000.0;
        bw_ms + lat_ms
    }
}

/// Storage manager: named devices.
#[derive(Debug, Default)]
pub struct StorageManager {
    devices: std::collections::BTreeMap<String, StorageDevice>,
}

impl StorageManager {
    /// New manager.
    pub fn new() -> Self {
        Self::default()
    }
    /// Register device.
    pub fn add(&mut self, d: StorageDevice) {
        self.devices.insert(d.name.clone(), d);
    }
    /// Get device.
    pub fn get(&self, name: &str) -> Option<&StorageDevice> {
        self.devices.get(name)
    }
    /// Get mutable.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut StorageDevice> {
        self.devices.get_mut(name)
    }
    /// Remove.
    pub fn remove(&mut self, name: &str) -> Option<StorageDevice> {
        self.devices.remove(name)
    }
    /// All device names.
    pub fn names(&self) -> Vec<&str> {
        self.devices.keys().map(|s| s.as_str()).collect()
    }
    /// Fastest device by seq read.
    pub fn fastest(&self) -> Option<&StorageDevice> {
        self.devices.values().max_by(|a, b| {
            a.seq_read_mbps
                .partial_cmp(&b.seq_read_mbps)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
    /// Total capacity/free.
    pub fn totals(&self) -> (u64, u64) {
        self.devices
            .values()
            .fold((0, 0), |(c, f), d| (c + d.capacity, f + d.free()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_profiles() {
        let nvme = StorageDevice::new("nvme0", DriveKind::Nvme, 1000);
        let hdd = StorageDevice::new("hdd0", DriveKind::Hdd, 4000);
        assert!(nvme.seq_read_mbps > hdd.seq_read_mbps);
        assert!(nvme.read_latency_us < hdd.read_latency_us);
        assert!(nvme.can_fit(nvme.capacity - 1));
    }

    #[test]
    fn read_time_estimate() {
        let d = StorageDevice::new("s", DriveKind::Nvme, 100);
        let ms = d.est_read_ms(100 * 1024 * 1024); // 100 MiB
        assert!(ms > 0.0 && ms < 1000.0);
    }

    #[test]
    fn manager_totals() {
        let mut m = StorageManager::new();
        m.add(StorageDevice::new("a", DriveKind::Nvme, 100));
        m.add(StorageDevice::new("b", DriveKind::Hdd, 200));
        let (cap, free) = m.totals();
        assert_eq!(cap, 300 * 1024 * 1024 * 1024);
        assert_eq!(free, cap);
        assert_eq!(m.fastest().unwrap().name, "a");
    }
}
