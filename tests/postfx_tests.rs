//! Phase-9 integration tests: post-FX pipeline, widgets, textures,
//! shader cache.

use xholy::gui::postfx::{AaMethod, PostFxConfig, ToneMapping};
use xholy::gui::textures::{load_rgba_at_offset, RgbaImage, TextureAtlas};
use xholy::gui::widgets::{Rect, WidgetEvent, WidgetTree};
use xholy::gui::windowed::ui::Framebuffer;
use xholy::gui::{AaMethod as Aa2, GpuApiPreference, GuiConfig};

#[test]
fn postfx_config_from_cli_surface() {
    // Mirrors what main.rs builds from --bloom/--tone-mapping/--aa.
    let cfg = PostFxConfig {
        bloom: true,
        bloom_strength: 0.8,
        tone_mapping: ToneMapping::Acs,
        aa: AaMethod::Fxaa,
        ..Default::default()
    };
    assert!(cfg.active());
    let u = cfg.uniforms(800, 600);
    assert_eq!(u[2], 3.0); // ACES index
    assert!((u[4] - 1.0 / 800.0).abs() < 1e-6);
    let _ = Aa2::Off; // re-export sanity
}

#[test]
fn widget_tree_full_interaction() {
    let mut t = WidgetTree::new();
    let btn = t.button(
        "OK",
        Rect {
            x: 10,
            y: 10,
            w: 60,
            h: 24,
        },
        1,
    );
    let panel = {
        use xholy::gui::widgets::Widget;
        t.push(Widget::Panel {
            id: 0,
            rect: Rect {
                x: 0,
                y: 0,
                w: 200,
                h: 100,
            },
            z: 0,
            accent: false,
        });
        0
    };
    let _ = panel;

    // Hover
    assert!(t.update((20, 20), false, false).is_empty());
    // Click
    t.update((20, 20), true, true);
    let ev = t.update((20, 20), false, false);
    assert!(ev.contains(&WidgetEvent::Clicked(btn)));

    // Draw into framebuffer
    let mut fb: Framebuffer = vec![0u8; 200 * 100 * 4];
    t.draw(&mut fb, 200, 100);
    let i = (20 * 200 + 20) * 4;
    assert_ne!(&fb[i..i + 3], &[0, 0, 0]);
}

#[test]
fn texture_atlas_and_iso_offset_load() {
    let mut atlas = TextureAtlas::new(128);
    for i in 0..4 {
        let img = RgbaImage::solid(8, 8, [i * 60, 0, 0, 255]);
        assert!(atlas.insert(&format!("s{i}"), &img).is_some());
    }
    assert!(atlas.get("s0").is_some());
    assert!(atlas.get("missing").is_none());

    // ISO-offset load: write a fake "disc" with known pixels.
    let dir = std::env::temp_dir().join("xholy_iso_tex");
    std::fs::create_dir_all(&dir).unwrap();
    let iso = dir.join("game.iso");
    let mut data = vec![0u8; 4096];
    for i in 0..(4 * 4 * 4) {
        data[2048 + i] = 0xAB;
    }
    std::fs::write(&iso, &data).unwrap();
    let img = load_rgba_at_offset(&iso, 2048, 4, 4).unwrap();
    assert_eq!(img.pixels[0], 0xAB);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shader_cache_syncs_known_shaders() {
    let dir = std::env::temp_dir().join("xholy_sc_it");
    let _ = std::fs::remove_dir_all(&dir);
    let mut cache = xholy::gui::shader_cache::ShaderCache::open(&dir).unwrap();
    assert!(cache.sync().unwrap() > 0);
    assert!(cache.read("gui.wgsl").unwrap().contains("vs_main"));
    assert!(cache.read("postfx.wgsl").unwrap().contains("composite_fs"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_postfx_produces_frame() {
    // PostFX on (default) — exercises bright/blur/composite passes.
    let config = GuiConfig {
        window_size: (320, 200),
        fps_limit: None,
        postfx: PostFxConfig::default(),
        ..Default::default()
    };
    let fb = xholy::gui::windowed::run_headless(config.clone(), 2, GpuApiPreference::Auto)
        .expect("headless postfx render");
    assert_eq!(fb.len(), 320 * 200 * 4);
    // UI has non-black pixels (gold toolbar text etc.).
    assert!(fb.chunks(4).any(|p| p[0] > 200 && p[1] > 150));

    // PostFX fully off — plain blit path still works.
    let mut off_cfg = config;
    off_cfg.postfx = PostFxConfig {
        bloom: false,
        tone_mapping: ToneMapping::Linear,
        aa: AaMethod::Off,
        ..Default::default()
    };
    assert!(!off_cfg.postfx.active());
    let fb2 = xholy::gui::windowed::run_headless(off_cfg, 1, GpuApiPreference::Auto)
        .expect("headless plain render");
    assert_eq!(fb2.len(), 320 * 200 * 4);
}
