//! Kinect device emulation.
//!
//! Xbox One titles using Kinect for Xbox One get depth/color/IR skeleton
//! tracking. This module exposes a stub provider that can be fed synthetic
//! or recorded Kinect frames.

/// A single Kinect frame (depth + color + body indices).
#[derive(Debug, Clone, Default)]
pub struct KinectFrame {
    pub width: u32,
    pub height: u32,
    /// 16-bit depth values, millimeters.
    pub depth: Vec<u16>,
    /// RGBA color image.
    pub color: Vec<u8>,
    /// Body index per pixel (0 = no body).
    pub body_index: Vec<u8>,
}

/// Kinect sensor provider.
pub struct Kinect {
    enabled: bool,
}

impl Kinect {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Return the most recent frame (empty stub frame when disabled).
    pub fn latest_frame(&self) -> KinectFrame {
        KinectFrame::default()
    }
}
