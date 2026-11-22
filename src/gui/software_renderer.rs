//! Software (CPU) frame renderer — last-resort GUI fallback.
//!
//! Keeps the most recent composited RGBA framebuffer in memory. Used
//! for `--gui-software`, headless tests without a GPU, and as the
//! floor of the auto-detection chain. In windowed mode a true CPU
//! blit to screen would need a GL context anyway, so this backend is
//! primarily for headless rendering and framebuffer verification.

use super::renderer::GuiFrameRenderer;
use crate::gui::gpu_api_detector::GpuApi;

/// CPU framebuffer renderer.
#[derive(Debug, Default)]
pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    /// Last composited frame (RGBA8).
    last_frame: Vec<u8>,
}

impl SoftwareRenderer {
    /// Create a software renderer at the given size.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            last_frame: Vec::new(),
        }
    }

    /// Last composited frame.
    pub fn last_frame(&self) -> &[u8] {
        &self.last_frame
    }
}

impl GuiFrameRenderer for SoftwareRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    fn render_frame(&mut self, rgba: &[u8], _width: u32, _height: u32) -> Result<(), String> {
        // Validate size loosely, then store.
        let expected = (self.width * self.height * 4) as usize;
        if rgba.len() != expected {
            return Err(format!(
                "software renderer: expected {expected} bytes, got {}",
                rgba.len()
            ));
        }
        self.last_frame.clear();
        self.last_frame.extend_from_slice(rgba);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "software (CPU)"
    }

    fn gpu_api(&self) -> GpuApi {
        GpuApi::Software
    }

    fn is_fallback(&self) -> bool {
        true
    }

    fn warning(&self) -> Option<&'static str> {
        Some("Running on software renderer — performance may be lower")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_frame() {
        let mut r = SoftwareRenderer::new(4, 4);
        let px = vec![255u8; 4 * 4 * 4];
        r.render_frame(&px, 4, 4).unwrap();
        assert_eq!(r.last_frame().len(), 64);
        assert_eq!(r.gpu_api(), GpuApi::Software);
        assert!(r.is_fallback());
    }

    #[test]
    fn rejects_wrong_size() {
        let mut r = SoftwareRenderer::new(4, 4);
        assert!(r.render_frame(&[0u8; 8], 4, 4).is_err());
    }
}
