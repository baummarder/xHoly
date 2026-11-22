//! Integration tests for GPU API detection and renderer selection.

use xholy::gui::gpu_api_detector::{
    backends_for, detect_gpu_api, detect_with_preference, GpuApi, GpuApiPreference,
};
use xholy::gui::renderer::GuiFrameRenderer;
use xholy::gui::software_renderer::SoftwareRenderer;

#[test]
fn auto_detection_returns_valid_api() {
    let d = detect_gpu_api();
    // Must resolve to one of the three APIs; Software is the floor.
    match d.api {
        GpuApi::Vulkan | GpuApi::OpenGL | GpuApi::Software => {}
    }
    // Adapter name is non-empty for hardware APIs, "CPU rasterizer" for SW.
    assert!(!d.adapter_name.is_empty() || d.api == GpuApi::Software);
}

#[test]
fn forced_software_detection() {
    let d = detect_with_preference(GpuApiPreference::Software);
    assert_eq!(d.api, GpuApi::Software);
    assert!(d.forced);
    assert_eq!(d.adapter_name, "CPU rasterizer");
}

#[test]
fn preference_parse_all_variants() {
    for (s, expected) in [
        ("auto", GpuApiPreference::Auto),
        ("vulkan", GpuApiPreference::Vulkan),
        ("opengl", GpuApiPreference::OpenGL),
        ("gl", GpuApiPreference::OpenGL),
        ("software", GpuApiPreference::Software),
        ("cpu", GpuApiPreference::Software),
    ] {
        assert_eq!(GpuApiPreference::parse(s), Some(expected), "input: {s}");
    }
    assert_eq!(GpuApiPreference::parse("directx12"), None);
}

#[test]
fn backends_mapping_is_correct() {
    assert_eq!(backends_for(GpuApi::Vulkan), wgpu::Backends::VULKAN);
    assert_eq!(backends_for(GpuApi::OpenGL), wgpu::Backends::GL);
    assert!(backends_for(GpuApi::Software).is_empty());
}

#[test]
fn software_renderer_fallback_trait() {
    let mut r = SoftwareRenderer::new(8, 8);
    let px = vec![0xFFu8; 8 * 8 * 4];
    r.render_frame(&px, 8, 8).unwrap();
    assert!(r.is_fallback());
    assert_eq!(r.gpu_api(), GpuApi::Software);
    assert!(r.warning().unwrap().contains("software"));
}

#[test]
fn headless_gui_renders_with_detection() {
    // Auto mode: whatever the probe finds, headless must produce a
    // framebuffer of the configured size.
    let config = xholy::gui::GuiConfig {
        window_size: (160, 120),
        fps_limit: None,
        ..Default::default()
    };
    let fb = xholy::gui::windowed::run_headless(config, 1, GpuApiPreference::Auto)
        .expect("headless render");
    assert_eq!(fb.len(), 160 * 120 * 4);
}

#[test]
fn headless_gui_forced_software() {
    let config = xholy::gui::GuiConfig {
        window_size: (160, 120),
        fps_limit: None,
        ..Default::default()
    };
    let fb = xholy::gui::windowed::run_headless(config, 1, GpuApiPreference::Software)
        .expect("software headless render");
    assert_eq!(fb.len(), 160 * 120 * 4);
}
