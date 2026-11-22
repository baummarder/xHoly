//! Phase 12 performance smoke tests — CPU-side pipeline costs.
//!
//! These bound the pure-Rust parts of the render path (atlas packing,
//! post-FX uniform math, tone mapping, framebuffer setup). GPU timing
//! is covered indirectly by the headless renderer tests in
//! `postfx_tests.rs`; real FPS numbers depend on hardware and are not
//! asserted here.

use std::time::Instant;
use xholy::gui::postfx::{AaMethod, PostFxConfig, ToneMapping};
use xholy::gui::textures::{RgbaImage, TextureAtlas};
use xholy::gui::windowed::frame_buffer::{FrameBufferDesc, DEFAULT_CLEAR};

/// 1080p atlas packing of 256 32×32 sprites stays well under the
/// 20 ms texture-atlas budget.
#[test]
fn atlas_pack_1080p_budget() {
    let mut atlas = TextureAtlas::new(2048);
    let sprite = RgbaImage::solid(32, 32, [200, 150, 50, 255]);
    let start = Instant::now();
    let mut packed = 0;
    for i in 0..256 {
        if atlas.insert(&format!("s{i}"), &sprite).is_some() {
            packed += 1;
        }
    }
    let elapsed = start.elapsed();
    assert!(packed > 0);
    assert!(
        elapsed.as_millis() < 20,
        "atlas pack took {}ms (>20ms budget)",
        elapsed.as_millis()
    );
}

/// PostFX uniform computation for a full HD frame is trivially cheap.
#[test]
fn postfx_uniforms_cheap() {
    let cfg = PostFxConfig {
        bloom: true,
        bloom_strength: 0.6,
        bloom_threshold: 0.75,
        tone_mapping: ToneMapping::Acs,
        aa: AaMethod::Fxaa,
    };
    let start = Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(cfg.uniforms(1920, 1080));
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 50,
        "10k uniform evals took {}ms",
        elapsed.as_millis()
    );
}

/// Tone mapping a full 1080p RGBA frame on CPU stays in the low-ms
/// range — this is the software-fallback cost ceiling.
#[test]
fn tonemap_1080p_frame_budget() {
    let cfg = PostFxConfig {
        tone_mapping: ToneMapping::Filmic,
        ..Default::default()
    };
    let px: Vec<f32> = (0..1920 * 1080)
        .map(|i| (i % 1024) as f32 / 1024.0 * 4.0)
        .collect();
    let start = Instant::now();
    let mut acc = 0.0f32;
    for &c in &px {
        acc += cfg.tonemap(c);
    }
    std::hint::black_box(acc);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 100,
        "1080p tonemap took {}ms",
        elapsed.as_millis()
    );
}

/// Framebuffer descriptor math is constant-time and validates.
#[test]
fn framebuffer_desc_overhead() {
    let start = Instant::now();
    for _ in 0..100_000 {
        let d = FrameBufferDesc {
            width: 1920,
            height: 1080,
            ..FrameBufferDesc::hd()
        };
        std::hint::black_box(d.valid());
    }
    assert!(start.elapsed().as_millis() < 50);
    const {
        assert!(DEFAULT_CLEAR.a == 1.0);
    }
}
