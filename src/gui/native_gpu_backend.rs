//! Native GPU backend abstraction — Phase 21.
//!
//! Selects the platform's primary graphics API (D3D12 on Windows,
//! Vulkan on Linux, Metal on macOS) and exposes the wgpu backend bits
//! that implement it in this scaffold. No native D3D12/Metal device
//! layer is linked — see STATUS.md for the honest limit.

use super::config::GpuApi;
use super::gpu_api_detector::{
    host_platform, select_platform_backend, GpuPlatform, PlatformBackend,
};

/// Concrete backend selection for one platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeGpuBackend {
    /// Platform the selection was made for.
    pub platform: GpuPlatform,
    /// Resolved API and wgpu backend bits.
    pub selection: PlatformBackend,
}

impl NativeGpuBackend {
    /// Select the native backend for `platform`.
    pub fn new(platform: GpuPlatform) -> Self {
        Self {
            platform,
            selection: select_platform_backend(platform),
        }
    }

    /// Select the backend for the host platform.
    pub fn for_host() -> Self {
        Self::new(host_platform())
    }

    /// Primary API (D3D12 / Vulkan / Metal).
    pub fn api(&self) -> GpuApi {
        self.selection.api
    }

    /// wgpu backend bits to use for device/instance creation.
    pub fn backends(&self) -> wgpu::Backends {
        self.selection.wgpu_backends
    }

    /// Stable label for the info panel / logs.
    pub fn label(&self) -> &'static str {
        self.selection.label
    }

    /// Whether a hardware API was selected (false for software floor).
    pub fn is_native(&self) -> bool {
        self.api() != GpuApi::Software
    }

    /// Create a wgpu instance restricted to the selected backend.
    pub fn instance(&self) -> wgpu::Instance {
        wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: self.backends(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_backend_is_d3d12() {
        let backend = NativeGpuBackend::new(GpuPlatform::Windows);
        assert_eq!(backend.api(), GpuApi::D3D12);
        assert_eq!(backend.backends(), wgpu::Backends::DX12);
        assert!(backend.is_native());
        assert_eq!(backend.label(), "d3d12 (wgpu)");
    }

    #[test]
    fn linux_backend_is_vulkan() {
        let backend = NativeGpuBackend::new(GpuPlatform::Linux);
        assert_eq!(backend.api(), GpuApi::Vulkan);
        assert_eq!(backend.backends(), wgpu::Backends::VULKAN);
    }

    #[test]
    fn macos_backend_is_metal() {
        let backend = NativeGpuBackend::new(GpuPlatform::MacOS);
        assert_eq!(backend.api(), GpuApi::Metal);
        assert_eq!(backend.backends(), wgpu::Backends::METAL);
    }

    #[test]
    fn host_backend_has_hardware_bits() {
        let backend = NativeGpuBackend::for_host();
        assert!(!backend.backends().is_empty());
        assert!(backend.is_native());
    }
}
