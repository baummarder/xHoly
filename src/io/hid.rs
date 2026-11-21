//! HID abstraction layer.
//!
//! Provides a uniform FFI-style interface over platform HID APIs
//! (Windows HID, IOKit on macOS, evdev on Linux) so game controllers,
//! racing wheels, flight sticks, and the Kinect can be enumerated and
//! polled identically across hosts.

/// A discovered HID device.
#[derive(Debug, Clone)]
pub struct HidDevice {
    /// Platform device path or `/dev/input/event*` node.
    pub path: String,
    /// Vendor ID.
    pub vendor_id: u16,
    /// Product ID.
    pub product_id: u16,
    /// Human-readable product string.
    pub product: String,
}

/// Enumerate HID devices. Scaffold returns an empty list; real
/// implementation calls into the platform HID stack.
pub fn enumerate() -> Vec<HidDevice> {
    Vec::new()
}

/// Read a report from `device`. Returns `None` on failure.
pub fn read_report(_device: &HidDevice, _buf: &mut [u8]) -> Option<usize> {
    None
}
