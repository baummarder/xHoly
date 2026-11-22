//! Framebuffer with FBO-fusion — Phase 12.
//!
//! [`FrameBuffer`] bundles a color attachment + optional depth
//! attachment into one reusable offscreen target. Multiple render
//! passes (game → post-FX → UI) can share it: the "fusion" is that
//! one [`wgpu::Texture`] backs every pass instead of allocating a
//! texture per pass.
//!
//! CPU-side parts (desc math, clear color) are testable headless;
//! GPU creation requires a `wgpu::Device`.

/// Clear color used across the engine (dark Xbox theme).
pub const DEFAULT_CLEAR: wgpu_color::Color = wgpu_color::Color {
    r: 0.08,
    g: 0.09,
    b: 0.10,
    a: 1.0,
};

/// Tiny color wrapper so this module's pure helpers don't need
/// `wgpu` types at the type level on wasm (wgpu is still used for
/// the GPU half below).
pub mod wgpu_color {
    /// RGBA clear color.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Color {
        pub r: f64,
        pub g: f64,
        pub b: f64,
        pub a: f64,
    }
}

/// Descriptor for a framebuffer (pure data — testable anywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBufferDesc {
    pub width: u32,
    pub height: u32,
    /// Color format.
    pub color_format: FrameBufferFormat,
    /// Create a depth attachment?
    pub depth: bool,
    /// MSAA sample count (1 or 4).
    pub samples: u32,
}

/// Supported color formats (subset of wgpu formats we actually use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameBufferFormat {
    Rgba8Unorm,
    Bgra8Unorm,
    Rgba8UnormSrgb,
}

impl FrameBufferDesc {
    /// 1080p RGBA8 single-sample, no depth.
    pub fn hd() -> Self {
        Self {
            width: 1920,
            height: 1080,
            color_format: FrameBufferFormat::Rgba8Unorm,
            depth: false,
            samples: 1,
        }
    }
    /// Sanity-check dims.
    pub fn valid(&self) -> bool {
        self.width > 0 && self.height > 0 && matches!(self.samples, 1 | 2 | 4 | 8)
    }
    /// wgpu color format mapping.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wgpu_format(&self) -> wgpu::TextureFormat {
        match self.color_format {
            FrameBufferFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
            FrameBufferFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            FrameBufferFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        }
    }
    /// wgpu depth format when depth is enabled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wgpu_depth_format(&self) -> wgpu::TextureFormat {
        wgpu::TextureFormat::Depth24Plus
    }
}

/// GPU framebuffer: color + optional depth, created once and reused
/// across passes (the "fusion" — resize reallocates, passes share).
#[cfg(not(target_arch = "wasm32"))]
pub struct FrameBuffer {
    /// Color texture.
    pub color: wgpu::Texture,
    /// Color view.
    pub color_view: wgpu::TextureView,
    /// Depth texture (None when `depth` was false).
    pub depth: Option<wgpu::Texture>,
    /// Depth view.
    pub depth_view: Option<wgpu::TextureView>,
    /// Current size.
    pub width: u32,
    pub height: u32,
    /// Sample count.
    pub samples: u32,
    /// Format.
    pub format: FrameBufferFormat,
}

#[cfg(not(target_arch = "wasm32"))]
impl FrameBuffer {
    /// Create from descriptor.
    pub fn new(device: &wgpu::Device, desc: FrameBufferDesc) -> Self {
        assert!(desc.valid(), "invalid FrameBufferDesc");
        let size = wgpu::Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xHoly FrameBuffer Color"),
            size,
            mip_level_count: 1,
            sample_count: desc.samples,
            dimension: wgpu::TextureDimension::D2,
            format: desc.wgpu_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth, depth_view) = if desc.depth {
            let d = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xHoly FrameBuffer Depth"),
                size,
                mip_level_count: 1,
                sample_count: desc.samples,
                dimension: wgpu::TextureDimension::D2,
                format: desc.wgpu_depth_format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let v = d.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(d), Some(v))
        } else {
            (None, None)
        };
        Self {
            color,
            color_view,
            depth,
            depth_view,
            width: desc.width,
            height: desc.height,
            samples: desc.samples,
            format: desc.color_format,
        }
    }

    /// Recreate at a new size (keeps format/samples/depth flag).
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.width && height == self.height {
            return;
        }
        let desc = FrameBufferDesc {
            width,
            height,
            color_format: self.format,
            depth: self.depth_view.is_some(),
            samples: self.samples,
        };
        *self = Self::new(device, desc);
    }

    /// Begin a render pass on this framebuffer.
    /// `load` chooses clear vs. load (fusion: later passes Load).
    pub fn begin_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        clear: wgpu_color::Color,
        load: bool,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xHoly FrameBuffer Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if load {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r,
                            g: clear.g,
                            b: clear.b,
                            a: clear.a,
                        })
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: self.depth_view.as_ref().map(|v| {
                wgpu::RenderPassDepthStencilAttachment {
                    view: v,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desc_validation() {
        assert!(FrameBufferDesc::hd().valid());
        assert!(!FrameBufferDesc {
            width: 0,
            ..FrameBufferDesc::hd()
        }
        .valid());
        assert!(!FrameBufferDesc {
            samples: 3,
            ..FrameBufferDesc::hd()
        }
        .valid());
    }

    #[test]
    fn clear_color_is_dark_xbox_theme() {
        const {
            assert!(DEFAULT_CLEAR.r < 0.2 && DEFAULT_CLEAR.b < 0.2);
            assert!(DEFAULT_CLEAR.a == 1.0);
        }
    }

    #[test]
    fn format_mapping() {
        let d = FrameBufferDesc::hd();
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert_eq!(d.wgpu_format(), wgpu::TextureFormat::Rgba8Unorm);
            assert_eq!(d.wgpu_depth_format(), wgpu::TextureFormat::Depth24Plus);
        }
    }
}
