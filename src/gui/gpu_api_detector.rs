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
            GpuApi::Vulkan => "Vulkan",
            GpuApi::OpenGL => "OpenGL",
            GpuApi::Software => "Software",
        }
    }
}

/// Detect the best available GPU API.
///
/// Order: Vulkan → OpenGL → Software. Each probe creates a wgpu
/// instance restricted to that backend and requests an adapter with no
/// surface (works headless).
pub fn detect_gpu_api() -> GpuDetection {
    detect_with_preference(GpuApiPreference::Auto)
}

/// Detect honoring a user preference (Auto probes; others force).
pub fn detect_with_preference(pref: GpuApiPreference) -> GpuDetection {
    match pref {
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
            for (api, backends) in [
                (GpuApi::Vulkan, wgpu::Backends::VULKAN),
                (GpuApi::OpenGL, wgpu::Backends::GL),
            ] {
                let d = probe_single(api, backends, false);
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
        GpuApi::Vulkan => wgpu::Backends::VULKAN,
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
            GpuApi::Vulkan => "vulkan",
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
        assert_eq!(GpuApiPreference::parse("bogus"), None);
    }

    #[test]
    fn auto_detect_returns_api() {
        let d = detect_gpu_api();
        // Must always return something; Software is the floor.
        match d.api {
            GpuApi::Vulkan | GpuApi::OpenGL | GpuApi::Software => {}
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
        assert_eq!(backends_for(GpuApi::Vulkan), wgpu::Backends::VULKAN);
        assert_eq!(backends_for(GpuApi::OpenGL), wgpu::Backends::GL);
        assert_eq!(backends_for(GpuApi::Software), wgpu::Backends::empty());
    }
}
