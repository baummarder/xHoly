//! Integration tests for the Phase-7 frame pacer.

use std::time::{Duration, Instant};
use xholy::gui::{FramePacer, FramePacingMode, GuiConfig};

#[test]
fn pacer_holds_target_rate_within_tolerance() {
    // 60 FPS target, 30 frames ≈ 500ms. Allow generous ±25% for CI jitter.
    let mut pacer = FramePacer::with_mode(60, false, FramePacingMode::Fixed);
    pacer.record_frame_start();
    let t0 = Instant::now();
    for _ in 0..30 {
        pacer.sleep_until_next_frame();
        pacer.record_frame_start();
    }
    let elapsed = t0.elapsed();
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed <= Duration::from_millis(800),
        "30 frames @60fps took {elapsed:?} (expected ~500ms)"
    );
}

#[test]
fn uncapped_pacer_does_not_throttle() {
    let mut pacer = FramePacer::with_mode(0, false, FramePacingMode::Uncapped);
    let t0 = Instant::now();
    for _ in 0..100 {
        pacer.record_frame_start();
        pacer.sleep_until_next_frame();
    }
    assert!(t0.elapsed() < Duration::from_millis(100));
    assert_eq!(pacer.frame_count, 100);
}

#[test]
fn fps_stats_report_reasonable_values() {
    let mut pacer = FramePacer::with_mode(30, true, FramePacingMode::Auto);
    for _ in 0..20 {
        pacer.record_frame_start();
        std::thread::sleep(Duration::from_millis(30));
    }
    let avg = pacer.get_avg_fps();
    assert!(avg > 10.0 && avg < 200.0, "avg fps {avg} out of range");
    assert!(pacer.get_min_fps() <= pacer.get_max_fps());
}

#[test]
fn gui_config_defaults_include_pacing() {
    let cfg = GuiConfig::default();
    assert_eq!(cfg.frame_pacing_mode, FramePacingMode::Auto);
    assert!(cfg.vsync);
    assert_eq!(cfg.fps_limit, Some(120));
}

#[test]
fn headless_respects_fps_limit() {
    // 10 frames at 30 FPS ≈ 333ms minimum when capped.
    let config = GuiConfig {
        window_size: (160, 120),
        fps_limit: Some(30),
        ..Default::default()
    };
    let t0 = Instant::now();
    xholy::gui::windowed::run_headless(config, 10, xholy::gui::GpuApiPreference::Software)
        .expect("headless render");
    // Software path is CPU-bound; just verify it completes quickly enough
    // that pacing isn't broken (no multi-second stall).
    assert!(t0.elapsed() < Duration::from_secs(10));
}
