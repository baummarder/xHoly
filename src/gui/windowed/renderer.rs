//! wgpu renderer for the windowed GUI — Phase 5.
//!
//! Two modes:
//! - **Surface**: renders the composited RGBA framebuffer to the winit
//!   window's swapchain (real windowed mode).
//! - **Offscreen**: renders to a texture and reads pixels back (headless
//!   mode, tests, `--gui-headless`).
//!
//! The UI is composited in software ([`crate::gui::windowed::ui`]) and
//! uploaded as an `Rgba8Unorm` texture each frame; the shader is a
//! fullscreen triangle that samples it 1:1.

/// WGSL source for the GUI blit shader.
pub const GUI_SHADER_WGSL: &str = include_str!("../../../shaders/gui.wgsl");

/// Rendering backend selection.
pub enum RenderTarget {
    /// Windowed: draw to a wgpu surface.
    Surface(wgpu::Surface<'static>),
    /// Headless: draw to an offscreen texture + readback.
    Offscreen,
}

/// The windowed GUI renderer.
pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target: RenderTarget,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    width: u32,
    height: u32,
    /// Reusable frame texture (recreated on resize).
    frame_texture: Option<wgpu::Texture>,
    /// Phase-9 post-processing: config + lazily-built GPU pipeline.
    postfx_config: crate::gui::postfx::PostFxConfig,
    postfx: Option<super::postfx::PostFx>,
    /// Offscreen final-output texture (post-FX path readback).
    present_texture: Option<wgpu::Texture>,
    /// Number of guest frames uploaded through the direct GPU path.
    guest_frames_uploaded: u64,
}

impl WgpuRenderer {
    /// Create a renderer for a winit window surface.
    ///
    /// `vsync` selects the present mode: on → `Fifo` (vsync'd), off →
    /// `Immediate`/`Mailbox` when the surface supports them so the
    /// [`FramePacer`](crate::gui::FramePacer) can pace manually.
    pub fn new_surface(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<Self, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "no suitable wgpu adapter for surface".to_string())?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("xHoly GUI Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("device request failed: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // FramePacer maps vsync on/off onto the best available mode.
        let probe = crate::gui::FramePacer::new(60, vsync);
        let present_mode = probe.present_mode(&caps.present_modes);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let (pipeline, bgl, sampler) = Self::create_pipeline(&device, format);
        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout: bgl,
            sampler,
            target: RenderTarget::Surface(surface),
            surface_config: Some(config),
            width: width.max(1),
            height: height.max(1),
            frame_texture: None,
            postfx_config: crate::gui::postfx::PostFxConfig::default(),
            postfx: None,
            present_texture: None,
            guest_frames_uploaded: 0,
        })
    }

    /// Create a headless offscreen renderer (no window needed).
    pub fn new_offscreen(width: u32, height: u32) -> Result<Self, String> {
        Self::new_offscreen_with_backends(width, height, wgpu::Backends::PRIMARY)
    }

    /// Offscreen renderer restricted to specific wgpu backends
    /// (e.g. `Backends::VULKAN` or `Backends::GL` per GPU detection).
    pub fn new_offscreen_with_backends(
        width: u32,
        height: u32,
        backends: wgpu::Backends,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "no suitable wgpu adapter".to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("xHoly GUI Headless Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("device request failed: {e}"))?;

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (pipeline, bgl, sampler) = Self::create_pipeline(&device, format);
        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout: bgl,
            sampler,
            target: RenderTarget::Offscreen,
            surface_config: None,
            width: width.max(1),
            height: height.max(1),
            frame_texture: None,
            postfx_config: crate::gui::postfx::PostFxConfig::default(),
            postfx: None,
            present_texture: None,
            guest_frames_uploaded: 0,
        })
    }

    /// Upload a guest framebuffer frame straight to a wgpu texture and
    /// draw it — the guest pixels never pass through the CPU UI
    /// compositor (Phase 23). The frame is held by `GuestSession` when
    /// the guest writes nothing, and `read_guest_frame` bounds-checks the
    /// guest VA against the memory-layer region table.
    pub fn render_guest_frame(
        &mut self,
        frame: &crate::core::execution::GuestFrame,
    ) -> Result<(), String> {
        self.guest_frames_uploaded += 1;
        // The existing texture upload + fullscreen shader blit is already
        // a direct RGBA→GPU path; feeding guest pixels here bypasses
        // `ui::composite` entirely.
        self.render_rgba(&frame.pixels, frame.width, frame.height)
    }

    /// How many frames went through the direct guest→GPU path.
    pub fn guest_frames_uploaded(&self) -> u64 {
        self.guest_frames_uploaded
    }

    /// Configure the Phase-9 post-processing pipeline (bloom, tone
    /// mapping, FXAA). Pass `PostFxConfig::default()` for the standard
    /// look; a fully-disabled config falls back to the plain blit.
    pub fn set_postfx(&mut self, config: crate::gui::postfx::PostFxConfig) {
        self.postfx_config = config;
        self.postfx = None; // rebuilt lazily with the right format
    }

    /// Format of the final render target (surface or offscreen).
    fn target_format(&self) -> wgpu::TextureFormat {
        match (&self.target, &self.surface_config) {
            (RenderTarget::Surface(_), Some(cfg)) => cfg.format,
            _ => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// Lazily build the post-FX pipeline set for the current target.
    fn ensure_postfx(&mut self) -> Option<&mut super::postfx::PostFx> {
        if !self.postfx_config.active() {
            return None;
        }
        if self.postfx.is_none() {
            let format = self.target_format();
            self.postfx =
                super::postfx::PostFx::new(&self.device, &self.queue, format, self.postfx_config)
                    .map_err(|e| log::warn!("xHoly: postfx init failed: {e}"))
                    .ok();
        }
        self.postfx.as_mut()
    }

    /// Resize the render target (surface config or offscreen dims).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.frame_texture = None;
        self.present_texture = None;
        if let (RenderTarget::Surface(surface), Some(config)) =
            (&self.target, &mut self.surface_config)
        {
            config.width = self.width;
            config.height = self.height;
            surface.configure(&self.device, config);
        }
    }

    /// Upload `rgba` (width×height×4) and draw one fullscreen frame.
    ///
    /// For surface mode this presents to the window; for offscreen mode
    /// it renders to a texture (read back via [`Self::readback`]).
    pub fn render_rgba(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        debug_assert_eq!(rgba.len(), (width * height * 4) as usize);

        // (Re)create the frame texture when size changes.
        let need_tex = self
            .frame_texture
            .as_ref()
            .map(|t| t.width() != width || t.height() != height)
            .unwrap_or(true);
        if need_tex {
            self.frame_texture = Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xHoly GUI Frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        let texture = self.frame_texture.as_ref().unwrap();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xHoly GUI Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Phase 9: when post-FX is active the uploaded frame texture IS
        // the scene input — the composite pass samples it directly, so
        // no extra blit is needed before the post chain.
        let postfx_active = self.postfx_config.active();

        match &self.target {
            RenderTarget::Surface(surface) => {
                let frame = surface
                    .get_current_texture()
                    .map_err(|e| format!("surface texture: {e}"))?;
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("xHoly GUI Encoder"),
                        });
                if postfx_active {
                    if let Some(fx) = self.ensure_postfx() {
                        fx.render(&mut encoder, &tex_view, &view, width, height);
                    } else {
                        // PostFX failed to build — plain blit fallback.
                        self.blit_pass(&mut encoder, &bind_group, &view);
                    }
                } else {
                    self.blit_pass(&mut encoder, &bind_group, &view);
                }
                self.queue.submit([encoder.finish()]);
                frame.present();
                Ok(())
            }
            RenderTarget::Offscreen => {
                // Persistent output texture so post-FX results can be
                // read back (and so readback works for both paths).
                let need_out = self
                    .present_texture
                    .as_ref()
                    .map(|t| t.width() != width || t.height() != height)
                    .unwrap_or(true);
                if need_out {
                    self.present_texture =
                        Some(self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("xHoly GUI Present Target"),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        }));
                }
                let target_tex = self.present_texture.as_ref().unwrap();
                let view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("xHoly GUI Offscreen Encoder"),
                        });
                if postfx_active {
                    if let Some(fx) = self.ensure_postfx() {
                        fx.render(&mut encoder, &tex_view, &view, width, height);
                    } else {
                        self.blit_pass(&mut encoder, &bind_group, &view);
                    }
                } else {
                    self.blit_pass(&mut encoder, &bind_group, &view);
                }
                self.queue.submit([encoder.finish()]);
                Ok(())
            }
        }
    }

    /// Plain fullscreen blit of the frame texture into `view`.
    fn blit_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        view: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xHoly GUI Blit Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Read back the last offscreen frame (headless mode / tests).
    /// For surface mode this returns an error — readback isn't wired.
    pub fn readback(&mut self, width: u32, height: u32) -> Result<Vec<u8>, String> {
        if !matches!(self.target, RenderTarget::Offscreen) {
            return Err("readback only supported in offscreen mode".into());
        }
        // Post-FX path renders into present_texture; plain path also
        // renders there now (persistent offscreen target).
        let texture = self
            .present_texture
            .as_ref()
            .or(self.frame_texture.as_ref())
            .ok_or_else(|| "no frame rendered yet".to_string())?;
        let bytes_per_row = (width * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xHoly GUI Readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xHoly GUI Readback Encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| "map failed".to_string())?
            .map_err(|e| e.to_string())?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height as usize {
            let row = &data
                [y * bytes_per_row as usize..y * bytes_per_row as usize + (width * 4) as usize];
            out.extend_from_slice(row);
        }
        drop(data);
        buf.unmap();
        Ok(out)
    }

    fn create_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xHoly GUI Shader"),
            source: wgpu::ShaderSource::Wgsl(GUI_SHADER_WGSL.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xHoly GUI BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xHoly GUI Pipeline Layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xHoly GUI Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xHoly GUI Sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        (pipeline, bgl, sampler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::execution::GuestFrame;
    use crate::gui::postfx::{AaMethod, PostFxConfig, ToneMapping};

    #[test]
    fn guest_frame_uploads_directly_to_gpu() {
        let Ok(mut renderer) = WgpuRenderer::new_offscreen(160, 120) else {
            eprintln!("skipping GPU test (no adapter)");
            return;
        };
        // Plain blit so the readback contains the guest pixels unchanged.
        renderer.set_postfx(PostFxConfig {
            bloom: false,
            bloom_strength: 0.0,
            bloom_threshold: 1.0,
            tone_mapping: ToneMapping::Linear,
            aa: AaMethod::Off,
        });
        let mut pixels = vec![0u8; 160 * 120 * 4];
        pixels[0..4].copy_from_slice(&[200, 40, 40, 255]);
        let frame = GuestFrame {
            width: 160,
            height: 120,
            seq: 7,
            pixels,
        };
        renderer.render_guest_frame(&frame).expect("guest blit");
        assert_eq!(renderer.guest_frames_uploaded(), 1);
        let out = renderer.readback(160, 120).expect("readback");
        assert_eq!(out.len(), 160 * 120 * 4);
        assert_eq!(&out[0..4], &[200, 40, 40, 255]);
    }
}
