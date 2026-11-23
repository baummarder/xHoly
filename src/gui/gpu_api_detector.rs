//! GPU API auto-detection for the xHoly GUI — Phase 6.
//!
//! Probes wgpu backends in priority order (Vulkan → OpenGL → Software)
//! and reports which GPU API is usable, plus adapter/driver info for
//! the info panel. The result can be persisted to
//! `~/.config/xholy/gpu_config.toml` and overridden via CLI
//! (`--gpu-api`) or [`GuiConfig::gpu_api`].

pub use super::config::{GpuApi, GpuApiPreference};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Result of a successful probe.
#[derive(Debug, Clone)]
pub struct GpuDetection {
    /// Which API won.
    pub api: GpuApi,
    /// wgpu backend that produced it (for BackendBit logging).
    pub backend: String,
    /// Adapter name (e.g. "NVIDIA GeForce RTX 4070").
    pub adapter_name: String,
    /// Driver info string.
    pub driver: String,
    /// Whether this was an explicit user override.
    pub forced: bool,
}

impl GpuDetection {
    /// Short description for the GUI info panel.
    pub fn describe(&self) -> String {
        format!("{} · {}", self.api_name(), self.adapter_name)
    }

    fn api_name(&self) -> &'static str {
        match self.api {
            GpuApi::D3D12 => "Direct3D 12",
            GpuApi::Vulkan => "Vulkan",
            GpuApi::Metal => "Metal",
            GpuApi::OpenGL => "OpenGL",
            GpuApi::Software => "Software",
        }
    }
}

/// Operating system with a distinct primary GPU API (Phase 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPlatform {
    /// Direct3D 12 is the primary API.
    Windows,
    /// Vulkan is the primary API.
    Linux,
    /// Metal is the primary API.
    MacOS,
}

impl GpuPlatform {
    /// Short label for logs/config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOS => "macos",
        }
    }
}

/// The platform this binary was compiled for.
pub fn host_platform() -> GpuPlatform {
    #[cfg(target_os = "windows")]
    {
        GpuPlatform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        GpuPlatform::MacOS
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        GpuPlatform::Linux
    }
}

/// Primary GPU API for a platform: Windows → D3D12, Linux → Vulkan,
/// macOS → Metal.
pub fn preferred_api(platform: GpuPlatform) -> GpuApi {
    match platform {
        GpuPlatform::Windows => GpuApi::D3D12,
        GpuPlatform::Linux => GpuApi::Vulkan,
        GpuPlatform::MacOS => GpuApi::Metal,
    }
}

/// Probe order for a platform. The primary API comes first; wgpu's
/// other hardware backends follow before the software floor.
pub fn fallback_order(platform: GpuPlatform) -> &'static [GpuApi] {
    match platform {
        GpuPlatform::Windows => &[
            GpuApi::D3D12,
            GpuApi::Vulkan,
            GpuApi::OpenGL,
            GpuApi::Software,
        ],
        GpuPlatform::Linux => &[GpuApi::Vulkan, GpuApi::OpenGL, GpuApi::Software],
        GpuPlatform::MacOS => &[
            GpuApi::Metal,
            GpuApi::Vulkan, // MoltenVK via wgpu
            GpuApi::OpenGL,
            GpuApi::Software,
        ],
    }
}

/// Resolved native backend for a platform (Phase 21).
///
/// This scaffold does not link a native D3D12/Metal device layer; the
/// native API is represented by wgpu's corresponding backend bit. The
/// decision (which API wins on which OS) is nevertheless real and
/// platform-independent testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformBackend {
    /// Primary native API.
    pub api: GpuApi,
    /// wgpu backend bits implementing `api` here.
    pub wgpu_backends: wgpu::Backends,
    /// Human-readable backend label.
    pub label: &'static str,
}

/// Pick the primary backend for `platform`.
pub fn select_platform_backend(platform: GpuPlatform) -> PlatformBackend {
    let api = preferred_api(platform);
    PlatformBackend {
        api,
        wgpu_backends: backends_for(api),
        label: match api {
            GpuApi::D3D12 => "d3d12 (wgpu)",
            GpuApi::Vulkan => "vulkan (wgpu)",
            GpuApi::Metal => "metal (wgpu)",
            GpuApi::OpenGL => "opengl (wgpu)",
            GpuApi::Software => "software",
        },
    }
}

/// Detect the best available GPU API.
///
/// Auto mode uses [`fallback_order`] for [`host_platform`]; each probe
/// creates a wgpu instance restricted to one backend and requests an
/// adapter with no surface (works headless).
pub fn detect_gpu_api() -> GpuDetection {
    detect_with_preference(GpuApiPreference::Auto)
}

/// Detect honoring a user preference (Auto probes; others force).
pub fn detect_with_preference(pref: GpuApiPreference) -> GpuDetection {
    match pref {
        GpuApiPreference::D3D12 => probe_single(GpuApi::D3D12, wgpu::Backends::DX12, true),
        GpuApiPreference::Metal => probe_single(GpuApi::Metal, wgpu::Backends::METAL, true),
        GpuApiPreference::Vulkan => probe_single(GpuApi::Vulkan, wgpu::Backends::VULKAN, true),
        GpuApiPreference::OpenGL => probe_single(GpuApi::OpenGL, wgpu::Backends::GL, true),
        GpuApiPreference::Software => GpuDetection {
            api: GpuApi::Software,
            backend: "none".into(),
            adapter_name: "CPU rasterizer".into(),
            driver: "none".into(),
            forced: true,
        },
        GpuApiPreference::Auto => {
            for &api in fallback_order(host_platform()) {
                if api == GpuApi::Software {
                    break;
                }
                let d = probe_single(api, backends_for(api), false);
                if d.api == api && !d.adapter_name.is_empty() {
                    return d;
                }
            }
            GpuDetection {
                api: GpuApi::Software,
                backend: "none".into(),
                adapter_name: "CPU rasterizer".into(),
                driver: "none".into(),
                forced: false,
            }
        }
    }
}

/// Probe one backend; on failure returns a Software detection.
fn probe_single(api: GpuApi, backends: wgpu::Backends, forced: bool) -> GpuDetection {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }));
    match adapter {
        Some(a) => {
            let info = a.get_info();
            GpuDetection {
                api,
                backend: format!("{:?}", info.backend),
                adapter_name: info.name,
                driver: info.driver_info,
                forced,
            }
        }
        None => GpuDetection {
            api: GpuApi::Software,
            backend: "none".into(),
            adapter_name: String::new(),
            driver: String::new(),
            forced,
        },
    }
}

/// Map a detected API to wgpu backend flags for instance creation.
pub fn backends_for(api: GpuApi) -> wgpu::Backends {
    match api {
        GpuApi::D3D12 => wgpu::Backends::DX12,
        GpuApi::Vulkan => wgpu::Backends::VULKAN,
        GpuApi::Metal => wgpu::Backends::METAL,
        GpuApi::OpenGL => wgpu::Backends::GL,
        // Software has no GPU backend; caller should use the CPU path.
        GpuApi::Software => wgpu::Backends::empty(),
    }
}

// ---------------------------------------------------------------------------
// Persistence: ~/.config/xholy/gpu_config.toml
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct GpuConfigFile {
    api: String,
    adapter_name: String,
    detected_at: String,
}

impl Default for GpuConfigFile {
    fn default() -> Self {
        Self {
            api: "auto".into(),
            adapter_name: String::new(),
            detected_at: String::new(),
        }
    }
}

fn gpu_config_path() -> PathBuf {
    crate::config::Config::default_path()
        .parent()
        .unwrap()
        .join("gpu_config.toml")
}

/// Load the persisted GPU preference (`auto`/`vulkan`/`opengl`/`software`).
pub fn load_preference() -> GpuApiPreference {
    std::fs::read_to_string(gpu_config_path())
        .ok()
        .and_then(|t| toml::from_str::<GpuConfigFile>(&t).ok())
        .and_then(|c| GpuApiPreference::parse(&c.api))
        .unwrap_or_default()
}

/// Persist the detected API + adapter info.
pub fn save_detection(d: &GpuDetection) {
    let cfg = GpuConfigFile {
        api: match d.api {
            GpuApi::D3D12 => "d3d12",
            GpuApi::Vulkan => "vulkan",
            GpuApi::Metal => "metal",
            GpuApi::OpenGL => "opengl",
            GpuApi::Software => "software",
        }
        .into(),
        adapter_name: d.adapter_name.clone(),
        detected_at: chrono::Local::now().to_rfc3339(),
    };
    let path = gpu_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_parse() {
        assert_eq!(
            GpuApiPreference::parse("vulkan"),
            Some(GpuApiPreference::Vulkan)
        );
        assert_eq!(
            GpuApiPreference::parse("GL"),
            Some(GpuApiPreference::OpenGL)
        );
        assert_eq!(
            GpuApiPreference::parse("cpu"),
            Some(GpuApiPreference::Software)
        );
        assert_eq!(
            GpuApiPreference::parse("d3d12"),
            Some(GpuApiPreference::D3D12)
        );
        assert_eq!(
            GpuApiPreference::parse("DX12"),
            Some(GpuApiPreference::D3D12)
        );
        assert_eq!(
            GpuApiPreference::parse("metal"),
            Some(GpuApiPreference::Metal)
        );
        assert_eq!(GpuApiPreference::parse("bogus"), None);
    }

    #[test]
    fn auto_detect_returns_api() {
        let d = detect_gpu_api();
        // Must always return something; Software is the floor.
        match d.api {
            GpuApi::D3D12 | GpuApi::Vulkan | GpuApi::Metal | GpuApi::OpenGL | GpuApi::Software => {}
        }
    }

    #[test]
    fn forced_software() {
        let d = detect_with_preference(GpuApiPreference::Software);
        assert_eq!(d.api, GpuApi::Software);
        assert!(d.forced);
    }

    #[test]
    fn backends_mapping() {
        assert_eq!(backends_for(GpuApi::D3D12), wgpu::Backends::DX12);
        assert_eq!(backends_for(GpuApi::Vulkan), wgpu::Backends::VULKAN);
        assert_eq!(backends_for(GpuApi::Metal), wgpu::Backends::METAL);
        assert_eq!(backends_for(GpuApi::OpenGL), wgpu::Backends::GL);
        assert_eq!(backends_for(GpuApi::Software), wgpu::Backends::empty());
    }

    #[test]
    fn platform_decision_windows_d3d12_linux_vulkan_mac_metal() {
        assert_eq!(preferred_api(GpuPlatform::Windows), GpuApi::D3D12);
        assert_eq!(preferred_api(GpuPlatform::Linux), GpuApi::Vulkan);
        assert_eq!(preferred_api(GpuPlatform::MacOS), GpuApi::Metal);
    }

    #[test]
    fn platform_fallback_order_ends_in_software() {
        assert_eq!(
            fallback_order(GpuPlatform::Windows),
            &[
                GpuApi::D3D12,
                GpuApi::Vulkan,
                GpuApi::OpenGL,
                GpuApi::Software
            ]
        );
        assert_eq!(
            fallback_order(GpuPlatform::Linux),
            &[GpuApi::Vulkan, GpuApi::OpenGL, GpuApi::Software]
        );
        assert_eq!(
            fallback_order(GpuPlatform::MacOS).first(),
            Some(&GpuApi::Metal)
        );
        for platform in [GpuPlatform::Windows, GpuPlatform::Linux, GpuPlatform::MacOS] {
            assert_eq!(fallback_order(platform).last(), Some(&GpuApi::Software));
        }
    }

    #[test]
    fn platform_backend_maps_native_api_to_wgpu() {
        let win = select_platform_backend(GpuPlatform::Windows);
        assert_eq!(win.api, GpuApi::D3D12);
        assert_eq!(win.wgpu_backends, wgpu::Backends::DX12);
        let linux = select_platform_backend(GpuPlatform::Linux);
        assert_eq!(linux.api, GpuApi::Vulkan);
        assert_eq!(linux.wgpu_backends, wgpu::Backends::VULKAN);
        let mac = select_platform_backend(GpuPlatform::MacOS);
        assert_eq!(mac.api, GpuApi::Metal);
        assert_eq!(mac.wgpu_backends, wgpu::Backends::METAL);
    }

    #[test]
    fn host_platform_has_a_primary_api() {
        let platform = host_platform();
        assert_eq!(
            preferred_api(platform),
            select_platform_backend(platform).api
        );
        assert!(fallback_order(platform).contains(&preferred_api(platform)));
    }
}
