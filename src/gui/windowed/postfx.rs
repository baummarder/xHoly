//! GPU post-processing pipeline — Phase 9.
//!
//! Implements the WGSL passes from `shaders/postfx.wgsl` on top of
//! wgpu: bright-pass → separable blur (half-res) → composite
//! (bloom + tone map + FXAA-lite). All passes are fullscreen-triangle
//! draws; no vertex buffers, no depth.
//!
//! Texture ping-pong layout (all `Rgba8Unorm`, half resolution):
//!   scene (full res) → bright → blur_h → blur_v → composite → target

use crate::gui::postfx::PostFxConfig;

/// WGSL source (single module, multiple entry points).
pub const POSTFX_SHADER_WGSL: &str = include_str!("../../../shaders/postfx.wgsl");

/// Bind group 0: shared uniforms (`params` + `size_params`).
const PARAMS_BINDING_SIZE: u64 = 32; // 2 × vec4<f32>

/// GPU-side post-FX pipeline set. Created lazily by the renderer once
/// the target format is known; rebuilt on resize.
pub struct PostFx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Fragment pipelines (bright, blur, composite) sharing one module.
    bright_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    /// Group-0 layout (uniforms) + group-1 layout (source tex) are
    /// shared; composite adds group-2 (bloom tex).
    params_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    bloom_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    size_buffer: wgpu::Buffer,
    /// Half-res ping-pong textures: [bright/blur_h out, blur_v out].
    half_a: Option<wgpu::Texture>,
    half_b: Option<wgpu::Texture>,
    /// Cached views (recreated with the textures).
    half_a_view: Option<wgpu::TextureView>,
    half_b_view: Option<wgpu::TextureView>,
    /// Half-res dimensions of the ping-pong targets.
    half_w: u32,
    half_h: u32,
    config: PostFxConfig,
}

impl PostFx {
    /// Create pipelines + shared resources for `format`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        config: PostFxConfig,
    ) -> Result<Self, String> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xHoly PostFX Shader"),
            source: wgpu::ShaderSource::Wgsl(POSTFX_SHADER_WGSL.into()),
        });

        // group(0): two uniform buffers (params vec4 + size/dir vec4).
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xHoly PostFX Params BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // group(1) / group(2): texture + sampler pairs.
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xHoly PostFX Tex BGL"),
            entries: &[tex_entry(0), samp_entry(1)],
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xHoly PostFX Bloom BGL"),
            entries: &[tex_entry(0), samp_entry(1)],
        });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xHoly PostFX Composite Layout"),
            bind_group_layouts: &[&params_layout, &tex_layout, &bloom_layout],
            push_constant_ranges: &[],
        });
        let bright_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xHoly PostFX Bright Layout"),
            bind_group_layouts: &[&params_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xHoly PostFX Blur Layout"),
            bind_group_layouts: &[&params_layout, &tex_layout],
            push_constant_ranges: &[],
        });

        let make_pipeline = |label: &str,
                             layout: &wgpu::PipelineLayout,
                             entry: &str,
                             format: wgpu::TextureFormat|
         -> wgpu::RenderPipeline {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
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
            })
        };

        // Bright/blur run into Rgba8Unorm half-res targets.
        let half_format = wgpu::TextureFormat::Rgba8Unorm;
        let bright_pipeline = make_pipeline(
            "xHoly PostFX Bright",
            &bright_layout,
            "bright_fs",
            half_format,
        );
        let blur_pipeline =
            make_pipeline("xHoly PostFX Blur", &blur_layout, "blur_fs", half_format);
        let composite_pipeline = make_pipeline(
            "xHoly PostFX Composite",
            &composite_layout,
            "composite_fs",
            format,
        );

        let mk_buf = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: PARAMS_BINDING_SIZE / 2,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let params_buffer = mk_buf("xHoly PostFX Params");
        let size_buffer = mk_buf("xHoly PostFX Size/Dir");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xHoly PostFX Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            bright_pipeline,
            blur_pipeline,
            composite_pipeline,
            params_layout,
            tex_layout,
            bloom_layout,
            composite_layout,
            sampler,
            params_buffer,
            size_buffer,
            half_a: None,
            half_b: None,
            half_a_view: None,
            half_b_view: None,
            half_w: 0,
            half_h: 0,
            config,
        })
    }

    /// Update the config (e.g. settings panel change).
    pub fn set_config(&mut self, config: PostFxConfig) {
        self.config = config;
    }

    /// Ensure the half-res ping-pong textures exist for `w`×`h`.
    fn ensure_half_textures(&mut self, w: u32, h: u32) {
        let hw = (w / 2).max(1);
        let hh = (h / 2).max(1);
        if self.half_w == hw && self.half_h == hh && self.half_a.is_some() {
            return;
        }
        self.half_w = hw;
        self.half_h = hh;
        let mk = |label: &str| {
            let t = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: hw,
                    height: hh,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            (t, v)
        };
        let (ta, va) = mk("xHoly PostFX Half A");
        let (tb, vb) = mk("xHoly PostFX Half B");
        self.half_a = Some(ta);
        self.half_b = Some(tb);
        self.half_a_view = Some(va);
        self.half_b_view = Some(vb);
    }

    fn params_bind_group(&self) -> wgpu::BindGroup {
        // Two vec4 uniforms in two buffers — a single buffer would need
        // a 256-byte-aligned offset for binding 1.
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xHoly PostFX Params BG"),
            layout: &self.params_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.size_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn tex_bind_group(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xHoly PostFX Tex BG"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Write the shared params + per-pass size/direction into the
    /// uniform buffers. `dir` = (1,0) H-blur, (0,1) V-blur, else (0,0).
    fn write_params(&self, width: u32, height: u32, dir: (f32, f32)) {
        let u = self.config.uniforms(width, height);
        let params: [f32; 4] = [u[0], u[1], u[2], u[3]];
        let size: [f32; 4] = [u[4], u[5], dir.0, dir.1];
        self.queue
            .write_buffer(&self.params_buffer, 0, bytemuck_cast(&params));
        self.queue
            .write_buffer(&self.size_buffer, 0, bytemuck_cast(&size));
    }

    /// Run the full post chain.
    ///
    /// `scene_view`: full-res scene texture (the composited UI frame).
    /// `target_view`: final color attachment (surface or offscreen).
    /// `target_format`: format of `target_view` (for pipeline check).
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        scene_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let _ = self.composite_layout; // layout held for clarity/debug
        self.ensure_half_textures(width, height);
        let hw = self.half_w;
        let hh = self.half_h;

        // Pass 1: bright → half_a
        self.write_params(width, height, (0.0, 0.0));
        let params_bg = self.params_bind_group();
        let scene_bg = self.tex_bind_group(scene_view);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xHoly PostFX Bright Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.half_a_view.as_ref().unwrap(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.bright_pipeline);
            pass.set_bind_group(0, &params_bg, &[]);
            pass.set_bind_group(1, &scene_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: H blur half_a → half_b
        self.write_params(hw, hh, (1.0, 0.0));
        let params_bg = self.params_bind_group();
        let a_bg = self.tex_bind_group(self.half_a_view.as_ref().unwrap());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xHoly PostFX Blur H"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.half_b_view.as_ref().unwrap(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &params_bg, &[]);
            pass.set_bind_group(1, &a_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: V blur half_b → half_a
        self.write_params(hw, hh, (0.0, 1.0));
        let params_bg = self.params_bind_group();
        let b_bg = self.tex_bind_group(self.half_b_view.as_ref().unwrap());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xHoly PostFX Blur V"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.half_a_view.as_ref().unwrap(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &params_bg, &[]);
            pass.set_bind_group(1, &b_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 4: composite → target. Three bind groups:
        //   0 = uniforms, 1 = scene tex, 2 = bloom tex.
        self.write_params(width, height, (0.0, 0.0));
        let params_bg = self.params_bind_group();
        let scene_bg = self.tex_bind_group(scene_view);
        let bloom_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xHoly PostFX Bloom BG"),
            layout: &self.bloom_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        self.half_a_view.as_ref().unwrap(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xHoly PostFX Composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &params_bg, &[]);
            pass.set_bind_group(1, &scene_bg, &[]);
            pass.set_bind_group(2, &bloom_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// Zero-copy &[f32; N] → &[u8] for uniform uploads.
fn bytemuck_cast<const N: usize>(data: &[f32; N]) -> &[u8] {
    // Safe: f32 has no padding.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}
