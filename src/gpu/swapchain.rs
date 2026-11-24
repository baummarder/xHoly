//! GPU swapchain + render-state abstractions (Phase 24).
//!
//! The Phase-21 platform decision selects the wgpu backend (D3D12 on
//! Windows, Vulkan on Linux, Metal on macOS). This module adds the
//! rendering-state layer a real guest draw needs:
//!
//! - depth/stencil descriptors (DSV, DSS, compare + write + stencil),
//! - indirect draw argument packing (`draw_indirect`),
//! - a stream-out buffer abstraction (wgpu has no D3D-style stream-out;
//!   this models the buffer and documents the fallback),
//! - a swapchain that owns a depth texture and a depth-enabled pipeline.
//!
//! Honest limit: guest draw-call translation is not complete — the
//! pipeline consumes translated WGSL from [`crate::gpu::shader`].

/// Depth/stencil comparison function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Always fail.
    Never,
    /// Less than.
    Less,
    /// Less or equal.
    LessEqual,
    /// Greater.
    Greater,
    /// Greater or equal.
    GreaterEqual,
    /// Equal.
    Equal,
    /// Not equal.
    NotEqual,
    /// Always pass.
    Always,
}

/// Stencil operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilOp {
    /// Keep the current value.
    Keep,
    /// Replace with the reference value.
    Replace,
    /// Increment, saturating.
    IncrementSaturate,
    /// Decrement, saturating.
    DecrementSaturate,
    /// Invert bits.
    Invert,
}

/// One stencil face state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StencilFaceState {
    /// Operation when the stencil test fails.
    pub fail: StencilOp,
    /// Operation when the depth test fails.
    pub depth_fail: StencilOp,
    /// Operation when both tests pass.
    pub pass: StencilOp,
    /// Comparison function.
    pub compare: CompareOp,
}

impl Default for StencilFaceState {
    fn default() -> Self {
        Self {
            fail: StencilOp::Keep,
            depth_fail: StencilOp::Keep,
            pass: StencilOp::Keep,
            compare: CompareOp::Always,
        }
    }
}

/// Full depth-stencil state (DSV + DSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthStencilState {
    /// Enable depth testing.
    pub depth_enabled: bool,
    /// Enable depth writes.
    pub depth_write: bool,
    /// Depth comparison.
    pub depth_compare: CompareOp,
    /// Enable stencil testing.
    pub stencil_enabled: bool,
    /// Stencil reference value.
    pub stencil_reference: u32,
    /// Stencil read mask.
    pub stencil_read_mask: u32,
    /// Stencil write mask.
    pub stencil_write_mask: u32,
    /// Front-face stencil state.
    pub front: StencilFaceState,
    /// Back-face stencil state.
    pub back: StencilFaceState,
}

impl Default for DepthStencilState {
    fn default() -> Self {
        Self {
            depth_enabled: true,
            depth_write: true,
            depth_compare: CompareOp::Less,
            stencil_enabled: false,
            stencil_reference: 0,
            stencil_read_mask: 0xFF,
            stencil_write_mask: 0xFF,
            front: StencilFaceState::default(),
            back: StencilFaceState::default(),
        }
    }
}

impl DepthStencilState {
    /// A disabled depth-stencil state (plain blit).
    pub fn disabled() -> Self {
        Self {
            depth_enabled: false,
            depth_write: false,
            stencil_enabled: false,
            ..Self::default()
        }
    }

    /// A typical opaque 3D state: less-equal test with depth writes.
    pub fn opaque_3d() -> Self {
        Self {
            depth_compare: CompareOp::LessEqual,
            ..Self::default()
        }
    }
}

/// Arguments for one indirect draw call (16 bytes, D3D/Vulkan layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndirectArgs {
    /// Number of vertices.
    pub vertex_count: u32,
    /// Number of instances.
    pub instance_count: u32,
    /// First vertex index.
    pub first_vertex: u32,
    /// First instance index.
    pub first_instance: u32,
}

impl IndirectArgs {
    /// Pack into the 16-byte buffer layout consumed by `draw_indirect`.
    pub fn as_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.vertex_count.to_le_bytes());
        out[4..8].copy_from_slice(&self.instance_count.to_le_bytes());
        out[8..12].copy_from_slice(&self.first_vertex.to_le_bytes());
        out[12..16].copy_from_slice(&self.first_instance.to_le_bytes());
        out
    }

    /// Decode from a 16-byte indirect buffer.
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        Self {
            vertex_count: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            instance_count: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            first_vertex: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            first_instance: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        }
    }
}

/// Description of a stream-out (transform-feedback) buffer.
///
/// D3D12 has real stream-out; wgpu does not. We model the layout and
/// let the host bind a storage buffer as the fallback target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOutDesc {
    /// Bytes per emitted vertex.
    pub stride: u32,
    /// Number of vertices the buffer holds.
    pub capacity: u32,
    /// Which vertex stream this captures.
    pub stream: u32,
}

impl StreamOutDesc {
    /// Total buffer size in bytes.
    pub fn byte_size(&self) -> u32 {
        self.stride.saturating_mul(self.capacity)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod wgpu_impl {
    use super::*;

    /// Convert the portable compare op to wgpu's enum.
    pub fn wgpu_compare(op: CompareOp) -> wgpu::CompareFunction {
        match op {
            CompareOp::Never => wgpu::CompareFunction::Never,
            CompareOp::Less => wgpu::CompareFunction::Less,
            CompareOp::LessEqual => wgpu::CompareFunction::LessEqual,
            CompareOp::Greater => wgpu::CompareFunction::Greater,
            CompareOp::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
            CompareOp::Equal => wgpu::CompareFunction::Equal,
            CompareOp::NotEqual => wgpu::CompareFunction::NotEqual,
            CompareOp::Always => wgpu::CompareFunction::Always,
        }
    }

    fn wgpu_stencil_op(op: StencilOp) -> wgpu::StencilOperation {
        match op {
            StencilOp::Keep => wgpu::StencilOperation::Keep,
            StencilOp::Replace => wgpu::StencilOperation::Replace,
            StencilOp::IncrementSaturate => wgpu::StencilOperation::IncrementClamp,
            StencilOp::DecrementSaturate => wgpu::StencilOperation::DecrementClamp,
            StencilOp::Invert => wgpu::StencilOperation::Invert,
        }
    }

    fn wgpu_face(state: StencilFaceState) -> wgpu::StencilFaceState {
        wgpu::StencilFaceState {
            compare: wgpu_compare(state.compare),
            fail_op: wgpu_stencil_op(state.fail),
            depth_fail_op: wgpu_stencil_op(state.depth_fail),
            pass_op: wgpu_stencil_op(state.pass),
        }
    }

    /// Convert our DSV/DSS description into a real wgpu depth-stencil state.
    pub fn wgpu_depth_stencil(state: DepthStencilState) -> Option<wgpu::DepthStencilState> {
        if !state.depth_enabled && !state.stencil_enabled {
            return None;
        }
        Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: state.depth_write,
            depth_compare: wgpu_compare(state.depth_compare),
            stencil: wgpu::StencilState {
                front: wgpu_face(state.front),
                back: wgpu_face(state.back),
                read_mask: state.stencil_read_mask,
                write_mask: state.stencil_write_mask,
            },
            bias: wgpu::DepthBiasState::default(),
        })
    }

    /// A swapchain-owned depth texture + converted state.
    pub struct SwapchainDepth {
        /// The GPU depth texture (Depth32Float).
        pub texture: wgpu::Texture,
        /// Stored view (kept alive for render-pass attachments).
        pub view: wgpu::TextureView,
        /// Converted pipeline state.
        pub state: Option<wgpu::DepthStencilState>,
        /// Reference value for stencil passes.
        pub stencil_reference: u32,
    }

    impl SwapchainDepth {
        /// Create a depth attachment for `width`×`height`.
        pub fn new(
            device: &wgpu::Device,
            width: u32,
            height: u32,
            state: DepthStencilState,
        ) -> Self {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xHoly Swapchain Depth"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Self {
                texture,
                view,
                state: wgpu_depth_stencil(state),
                stencil_reference: state.stencil_reference,
            }
        }

        /// Depth-stencil attachment for a render pass.
        pub fn attachment(&self) -> wgpu::RenderPassDepthStencilAttachment<'_> {
            wgpu::RenderPassDepthStencilAttachment {
                view: &self.view,
                depth_ops: self.state.as_ref().map(|_| wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }
        }
    }

    /// GPU buffer holding packed indirect draw arguments.
    pub struct IndirectDrawBuffer {
        /// The GPU buffer.
        pub buffer: wgpu::Buffer,
        /// Number of draws packed in the buffer.
        pub draws: u32,
    }

    impl IndirectDrawBuffer {
        /// Upload `args` as an indirect draw buffer.
        pub fn new(device: &wgpu::Device, args: &[IndirectArgs]) -> Self {
            let mut bytes = Vec::with_capacity(args.len() * 16);
            for a in args {
                bytes.extend_from_slice(&a.as_bytes());
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xHoly Indirect Draw"),
                size: bytes.len().max(16) as u64,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            Self {
                buffer,
                draws: args.len() as u32,
            }
        }
    }

    /// Storage buffer used as the stream-out fallback target.
    pub struct StreamOutBuffer {
        /// GPU storage buffer.
        pub buffer: wgpu::Buffer,
        /// Description of the captured stream.
        pub desc: StreamOutDesc,
    }

    impl StreamOutBuffer {
        /// Allocate a stream-out target.
        pub fn new(device: &wgpu::Device, desc: StreamOutDesc) -> Self {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xHoly Stream Out"),
                size: desc.byte_size().max(4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            Self { buffer, desc }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indirect_args_roundtrip() {
        let args = IndirectArgs {
            vertex_count: 36,
            instance_count: 2,
            first_vertex: 6,
            first_instance: 1,
        };
        assert_eq!(IndirectArgs::from_bytes(&args.as_bytes()), args);
    }

    #[test]
    fn depth_state_defaults_are_opaque_3d() {
        let s = DepthStencilState::default();
        assert!(s.depth_enabled && s.depth_write);
        assert_eq!(s.depth_compare, CompareOp::Less);
        assert!(!s.stencil_enabled);
    }

    #[test]
    fn disabled_state_has_no_depth() {
        assert!(!DepthStencilState::disabled().depth_enabled);
    }

    #[test]
    fn stream_out_size_is_stride_times_capacity() {
        let d = StreamOutDesc {
            stride: 32,
            capacity: 128,
            stream: 0,
        };
        assert_eq!(d.byte_size(), 4096);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wgpu_depth_state_conversion() {
        assert!(wgpu_impl::wgpu_depth_stencil(DepthStencilState::disabled()).is_none());
        assert!(wgpu_impl::wgpu_depth_stencil(DepthStencilState::opaque_3d()).is_some());
    }
}
