//! Debug overlay flags — what extra geometry/info is drawn.

/// Overlay render flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayFlags {
    /// Show FPS counter.
    pub fps: bool,
    /// Show frame-time graph.
    pub frametime_graph: bool,
    /// Wireframe mode (polygon edges only).
    pub wireframe: bool,
    /// Show render-target thumbnails.
    pub render_targets: bool,
    /// Show GPU/CPU occupancy bars.
    pub occupancy: bool,
    /// Show memory regions map.
    pub memory_map: bool,
}

/// The overlay state (flags + per-frame data).
#[derive(Debug, Default)]
pub struct DebugOverlay {
    /// Render flags.
    pub flags: OverlayFlags,
    /// Last FPS.
    pub fps: f64,
    /// Last frame ms.
    pub frame_ms: f64,
    /// GPU busy % (0..100, stub).
    pub gpu_busy: f64,
    /// CPU busy % (0..100, stub).
    pub cpu_busy: f64,
}

impl DebugOverlay {
    /// New overlay.
    pub fn new() -> Self {
        Self {
            flags: OverlayFlags {
                fps: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Update with a frame sample.
    pub fn tick(&mut self, fps: f64, frame_ms: f64) {
        self.fps = fps;
        self.frame_ms = frame_ms;
    }

    /// Is anything enabled?
    pub fn any(&self) -> bool {
        self.flags.fps
            || self.flags.frametime_graph
            || self.flags.wireframe
            || self.flags.render_targets
            || self.flags.occupancy
            || self.flags.memory_map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_tick() {
        let mut o = DebugOverlay::new();
        o.tick(60.0, 16.7);
        assert_eq!(o.fps, 60.0);
        assert!(o.any());
        o.flags = OverlayFlags::default();
        assert!(!o.any());
    }
}
