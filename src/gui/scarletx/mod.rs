//! ScarletX UI shell (Phase 24).
//!
//! A fresh, responsive UI layer that is deliberately separate from the
//! legacy software compositor. It owns three things:
//!
//! - a harmonious palette (trans-flag accents on deep navy, with coral
//!   and mint for status — the earlier trans-flag request stays primary),
//! - a proportional layout that adapts from 640×360 up to 4K,
//! - boot progress + renderer feedback views driven by the emulator.
//!
//! It renders into an RGBA8 framebuffer so it works headless in tests
//! and can be uploaded to the wgpu swapchain unchanged.

use crate::theme::{self, Rgba};

/// Harmonised ScarletX palette.
pub mod palette {
    use super::*;
    /// Window background (deep navy).
    pub const BG: Rgba = [0x0B, 0x0F, 0x1A, 0xFF];
    /// Raised panel.
    pub const PANEL: Rgba = [0x15, 0x1B, 0x2C, 0xFF];
    /// Interactive surface.
    pub const SURFACE: Rgba = [0x20, 0x2A, 0x42, 0xFF];
    /// Primary accent — trans blue (kept from the earlier UI request).
    pub const ACCENT: Rgba = theme::TRANS_BLUE;
    /// Secondary accent — trans pink.
    pub const ACCENT_2: Rgba = theme::TRANS_PINK;
    /// Success / healthy renderer.
    pub const MINT: Rgba = [0x4F, 0xD1, 0xA5, 0xFF];
    /// Warning / fallback renderer.
    pub const CORAL: Rgba = [0xFF, 0x8A, 0x6B, 0xFF];
    /// Body text.
    pub const TEXT: Rgba = [0xEE, 0xF3, 0xFF, 0xFF];
    /// Muted text.
    pub const MUTED: Rgba = [0x94, 0xA2, 0xC0, 0xFF];
}

/// Integer rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// Whether `(x, y)` lies inside.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.w)
            && y >= self.y
            && y < self.y.saturating_add(self.h)
    }
}

/// Resolved responsive layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Top bar with title + status.
    pub header: Rect,
    /// Game list / boot log.
    pub sidebar: Rect,
    /// Main preview / render target.
    pub stage: Rect,
    /// Renderer statistics strip.
    pub footer: Rect,
    /// Primary action buttons.
    pub buttons: Vec<(Rect, &'static str)>,
}

/// Compute a responsive layout for any framebuffer size.
///
/// Sidebar/stage split is 24/76 on wide screens, stacked on narrow
/// ones; all panels keep at least a few pixels of padding.
pub fn layout(width: u32, height: u32) -> Layout {
    let pad = (width.min(height) / 48).clamp(4, 20);
    let header_h = (height / 9).clamp(28, 72);
    let footer_h = (height / 10).clamp(22, 56);
    let header = Rect {
        x: 0,
        y: 0,
        w: width,
        h: header_h,
    };
    let content_y = header_h + pad;
    let content_h = height.saturating_sub(header_h + footer_h + 2 * pad);

    let (sidebar, stage) = if width >= height * 3 / 2 {
        let sidebar_w = (width / 4).clamp(120, 320);
        (
            Rect {
                x: pad,
                y: content_y,
                w: sidebar_w.saturating_sub(pad),
                h: content_h,
            },
            Rect {
                x: sidebar_w + pad,
                y: content_y,
                w: width.saturating_sub(sidebar_w + 2 * pad),
                h: content_h,
            },
        )
    } else {
        let split = content_h / 3;
        (
            Rect {
                x: pad,
                y: content_y,
                w: width.saturating_sub(2 * pad),
                h: split.saturating_sub(pad),
            },
            Rect {
                x: pad,
                y: content_y + split,
                w: width.saturating_sub(2 * pad),
                h: content_h.saturating_sub(split + pad),
            },
        )
    };

    let footer = Rect {
        x: 0,
        y: height.saturating_sub(footer_h),
        w: width,
        h: footer_h,
    };
    let labels = ["BOOT", "PLAY", "SETTINGS"];
    let bw = (width / 9).clamp(48, 120);
    let gap = (width / 64).clamp(4, 16);
    let total = bw * labels.len() as u32 + gap * (labels.len() as u32 - 1);
    let start = width.saturating_sub(total) / 2;
    let buttons = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            (
                Rect {
                    x: start + i as u32 * (bw + gap),
                    y: footer.y + (footer.h.saturating_sub(20)) / 2,
                    w: bw,
                    h: 20,
                },
                *label,
            )
        })
        .collect();
    Layout {
        header,
        sidebar,
        stage,
        footer,
        buttons,
    }
}

/// Stages of a retail boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStage {
    /// Console BIOS / bootloader.
    Bios,
    /// XEX2 header + directory.
    XexHeader,
    /// AES/LZX image decode.
    Decode,
    /// PowerPC guest execution.
    PowerPC,
    /// Kernel + XAM services.
    Kernel,
    /// Shader compilation.
    Shaders,
    /// Guest is running.
    Ready,
    /// Boot failed.
    Failed,
}

impl BootStage {
    /// Human-readable stage label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bios => "BIOS",
            Self::XexHeader => "XEX HEADER",
            Self::Decode => "DECODE",
            Self::PowerPC => "POWERPC",
            Self::Kernel => "KERNEL + XAM",
            Self::Shaders => "SHADERS",
            Self::Ready => "READY",
            Self::Failed => "FAILED",
        }
    }
}

/// Boot progress reported by the emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootProgress {
    /// Current stage.
    pub stage: BootStage,
    /// Completion in 0..=100.
    pub percent: u8,
    /// Optional detail line.
    pub message: String,
}

impl Default for BootProgress {
    fn default() -> Self {
        Self {
            stage: BootStage::Bios,
            percent: 0,
            message: String::new(),
        }
    }
}

impl BootProgress {
    /// Advance to the next stage with a given percentage.
    pub fn advance(&mut self, stage: BootStage, percent: u8) {
        self.stage = stage;
        self.percent = percent.min(100);
    }

    /// Whether boot finished successfully.
    pub fn is_ready(&self) -> bool {
        self.stage == BootStage::Ready && self.percent == 100
    }
}

/// Feedback reported by the GPU renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RendererFeedback {
    /// API label (e.g. "D3D12", "Vulkan", "Metal", "Software").
    pub api: &'static str,
    /// Frames per second.
    pub fps: f32,
    /// Draw calls submitted this frame.
    pub draw_calls: u32,
    /// Triangles submitted this frame.
    pub triangles: u32,
    /// Frame time in milliseconds.
    pub frame_ms: f32,
}

impl Default for RendererFeedback {
    fn default() -> Self {
        Self {
            api: "Software",
            fps: 0.0,
            draw_calls: 0,
            triangles: 0,
            frame_ms: 0.0,
        }
    }
}

/// Draw the ScarletX shell into an RGBA8 framebuffer.
pub fn render(
    fb: &mut [u8],
    width: u32,
    height: u32,
    boot: &BootProgress,
    feedback: &RendererFeedback,
) -> Layout {
    let l = layout(width, height);
    fill(fb, width, 0, 0, width, height, palette::BG);
    fill(
        fb,
        width,
        l.header.x,
        l.header.y,
        l.header.w,
        l.header.h,
        palette::PANEL,
    );
    fill(
        fb,
        width,
        l.header.x,
        l.header.y + l.header.h - 2,
        l.header.w,
        2,
        palette::ACCENT,
    );
    fill(
        fb,
        width,
        l.sidebar.x,
        l.sidebar.y,
        l.sidebar.w,
        l.sidebar.h,
        palette::PANEL,
    );
    fill(
        fb,
        width,
        l.stage.x,
        l.stage.y,
        l.stage.w,
        l.stage.h,
        palette::BG,
    );
    fill(
        fb,
        width,
        l.footer.x,
        l.footer.y,
        l.footer.w,
        l.footer.h,
        palette::PANEL,
    );

    // Boot progress bar in the sidebar.
    let bar = Rect {
        x: l.sidebar.x + 8,
        y: l.sidebar.y + 24,
        w: l.sidebar.w.saturating_sub(16),
        h: 8,
    };
    fill(fb, width, bar.x, bar.y, bar.w, bar.h, palette::SURFACE);
    let done = bar.w * boot.percent as u32 / 100;
    let bar_color = if boot.stage == BootStage::Failed {
        palette::CORAL
    } else {
        palette::ACCENT
    };
    fill(fb, width, bar.x, bar.y, done, bar.h, bar_color);

    // Stage + renderer status are encoded as coloured stripes so the
    // shell is testable without a font dependency.
    let status_color = if feedback.fps >= 30.0 {
        palette::MINT
    } else {
        palette::CORAL
    };
    let sy = l.footer.y + 4;
    fill(fb, width, 8, sy, 4, 8, status_color);
    for (rect, _) in &l.buttons {
        fill(fb, width, rect.x, rect.y, rect.w, rect.h, palette::SURFACE);
        fill(fb, width, rect.x, rect.y, rect.w, 2, palette::ACCENT_2);
    }
    l
}

fn fill(fb: &mut [u8], w: u32, x: u32, y: u32, rw: u32, rh: u32, color: Rgba) {
    if w == 0 {
        return;
    }
    let h = (fb.len() / 4 / w as usize) as u32;
    let x0 = x.min(w);
    let x1 = x.saturating_add(rw).min(w);
    let y0 = y.min(h);
    let y1 = y.saturating_add(rh).min(h);
    for yy in y0..y1 {
        let start = (yy as usize * w as usize + x0 as usize) * 4;
        let end = (yy as usize * w as usize + x1 as usize) * 4;
        for px in fb[start..end].as_chunks_mut::<4>().0 {
            px.copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_responsive_and_consistent() {
        let wide = layout(1920, 1080);
        assert!(wide.stage.w > wide.sidebar.w);
        assert_eq!(wide.buttons.len(), 3);
        let narrow = layout(640, 800);
        assert!(
            narrow.stage.y > narrow.sidebar.y,
            "narrow layout stacks panels"
        );
        assert!(narrow.header.h >= 28 && narrow.footer.h >= 22);
        for l in [wide, narrow] {
            assert!(l.stage.w > 0 && l.stage.h > 0);
            assert!(l.buttons.iter().all(|(r, _)| r.w > 0 && r.h > 0));
        }
    }

    #[test]
    fn boot_progress_advances_and_detects_ready() {
        let mut boot = BootProgress::default();
        assert_eq!(boot.stage, BootStage::Bios);
        boot.advance(BootStage::PowerPC, 60);
        assert_eq!(boot.percent, 60);
        boot.advance(BootStage::Ready, 200);
        assert!(boot.is_ready());
        assert_eq!(boot.percent, 100);
    }

    #[test]
    fn render_fills_a_framebuffer() {
        let (w, h) = (320, 200);
        let mut fb = vec![0u8; (w * h * 4) as usize];
        let boot = BootProgress {
            stage: BootStage::Decode,
            percent: 45,
            message: String::new(),
        };
        let feedback = RendererFeedback {
            api: "D3D12",
            fps: 60.0,
            draw_calls: 12,
            triangles: 480,
            frame_ms: 16.6,
        };
        let layout = render(&mut fb, w, h, &boot, &feedback);
        assert_eq!(fb.len(), (w * h * 4) as usize);
        assert!(fb.as_chunks::<4>().0.contains(&palette::ACCENT));
        assert_eq!(layout.header.w, w);
    }
}
