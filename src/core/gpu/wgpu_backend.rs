//! wgpu-based GPU backend — Phase 2.
//!
//! Headless implementation: creates a device, uploads the bit-texture,
//! renders the fullscreen triangle through [`SHADER_WGSL`], and reads the
//! target texture back to an RGBA8 buffer. No window/surface required,
//! which keeps `cargo test` working on CI and Pi.

use crate::core::gpu::{bytes_to_bitmap, GpuBackend, GpuFrame, SHADER_WGSL, TEX_HEIGHT};

/// wgpu backend state.
pub struct WgpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    width: u32,
    height: u32,
    /// Reusable readback buffer (sized for the current target).
    readback: Option<(wgpu::Buffer, u32)>, // (buffer, bytes_per_row padded)
}

impl WgpuBackend {
    /// Create instance → adapter → device → pipeline.
    fn create_device() -> Result<(wgpu::Device, wgpu::Queue), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
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
                label: Some("xHoly GPU Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| format!("device request failed: {e}"))?;
        Ok((device, queue))
    }

    fn create_pipeline(
        device: &wgpu::Device,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xHoly Render Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xHoly BGL"),
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
            label: Some("xHoly Pipeline Layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xHoly Render Pipeline"),
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
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
            label: Some("xHoly Sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        (pipeline, bgl, sampler)
    }
}

impl GpuBackend for WgpuBackend {
    fn init(width: u32, height: u32) -> Result<Self, String> {
        let (device, queue) = Self::create_device()?;
        let (pipeline, bind_group_layout, sampler) = Self::create_pipeline(&device);
        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            width: width.max(1),
            height: height.max(1),
            readback: None,
        })
    }

    fn render(&mut self, frame: &GpuFrame) -> Result<Vec<u8>, String> {
        let (tex_w, tex_h, bits) = bytes_to_bitmap(&frame.text);
        let tex_w = tex_w.max(1);
        let tex_h = tex_h.max(1);

        // Upload R8 bitmap.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xHoly Text Texture"),
            size: wgpu::Extent3d {
                width: tex_w,
                height: tex_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bits,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(tex_w),
                rows_per_image: Some(tex_h),
            },
            wgpu::Extent3d {
                width: tex_w,
                height: tex_h,
                depth_or_array_layers: 1,
            },
        );
        let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xHoly Bind Group"),
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

        // Offscreen RGBA8 target.
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xHoly Target"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Readback buffer (256-byte row alignment required by wgpu).
        let bytes_per_row = (self.width * 4).div_ceil(256) * 256;
        let buf_size = bytes_per_row * self.height;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xHoly Readback"),
            size: buf_size as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xHoly Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xHoly Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: frame.clear_color[0] as f64,
                            g: frame.clear_color[1] as f64,
                            b: frame.clear_color[2] as f64,
                            a: frame.clear_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        // Map and copy rows (strip row padding).
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| "readback map failed".to_string())?
            .map_err(|e| format!("buffer map error: {e}"))?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height as usize {
            let row = &data[y * bytes_per_row as usize
                ..y * bytes_per_row as usize + (self.width * 4) as usize];
            out.extend_from_slice(row);
        }
        drop(data);
        readback.unmap();
        self.readback = Some((readback, bytes_per_row));
        Ok(out)
    }

    fn dispose(&mut self) {
        // wgpu objects drop automatically; nothing explicit needed.
    }
}

// Keep TEX_HEIGHT referenced for API symmetry with mod.rs docs.
const _TEX_H: u32 = TEX_HEIGHT;
