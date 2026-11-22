//! Frame pacing & framerate control — Phase 7.
//!
//! [`FramePacer`] caps the GUI loop at `target_fps` using a deadline
//! derived from the last presented frame. With VSync enabled the
//! surface's `PresentMode::Fifo` does most of the work and the pacer
//! only trims overshoot; with VSync off the pacer sleeps the remainder
//! of each frame budget itself.
//!
//! The module is deliberately wgpu-free so it compiles and tests on
//! every target; [`FramePacer::present_mode`] (non-wasm only) maps the
//! pacing decision onto a `wgpu::PresentMode` for surface config.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How aggressively the loop is paced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FramePacingMode {
    /// VSync on → trust `PresentMode::Fifo`; VSync off → manual pacing.
    #[default]
    Auto,
    /// Always pace manually to `target_fps`, even with VSync on.
    Fixed,
    /// No limit — render as fast as the loop allows.
    Uncapped,
}

impl FramePacingMode {
    /// Parse a CLI/config string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "fixed" => Some(Self::Fixed),
            "uncapped" | "unlimited" | "off" => Some(Self::Uncapped),
            _ => None,
        }
    }
}

/// Frame-rate controller. Cheap to construct; all timing via `Instant`.
pub struct FramePacer {
    /// FPS the loop should hold (ignored in `Uncapped` mode).
    pub target_fps: u32,
    /// `1 / target_fps`; zero when uncapped.
    pub interval: Duration,
    /// Start time of the last presented frame.
    pub last_frame_time: Instant,
    /// Whether VSync is enabled (affects present mode + pacing policy).
    pub v_sync: bool,
    /// Pacing policy.
    pub mode: FramePacingMode,
    /// Total frames presented through this pacer.
    pub frame_count: u64,
    /// Recent frame durations (ring buffer, newest last) for stats.
    pub frame_times: VecDeque<Duration>,
}

/// How many recent frame durations feed the min/max/avg stats.
const STATS_WINDOW: usize = 240;

impl FramePacer {
    /// Create a pacer targeting `target_fps` (0 = uncapped).
    pub fn new(target_fps: u32, v_sync: bool) -> Self {
        Self::with_mode(target_fps, v_sync, FramePacingMode::Auto)
    }

    /// Create a pacer with an explicit pacing mode.
    pub fn with_mode(target_fps: u32, v_sync: bool, mode: FramePacingMode) -> Self {
        let interval = if target_fps > 0 {
            Duration::from_secs_f64(1.0 / f64::from(target_fps))
        } else {
            Duration::ZERO
        };
        Self {
            target_fps,
            interval,
            last_frame_time: Instant::now(),
            v_sync,
            mode,
            frame_count: 0,
            frame_times: VecDeque::with_capacity(STATS_WINDOW + 1),
        }
    }

    /// Whether manual pacing applies at all.
    pub fn is_capped(&self) -> bool {
        self.target_fps > 0 && self.mode != FramePacingMode::Uncapped
    }

    /// Mark the start of a new frame: stamps `last_frame_time` and
    /// records the elapsed time since the previous frame.
    pub fn record_frame_start(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_time);
        self.last_frame_time = now;
        if self.frame_times.len() == STATS_WINDOW {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(dt);
        self.frame_count += 1;
    }

    /// Whether a frame is due at `now`. Always true for uncapped and
    /// VSync-auto pacing (the present mode does the limiting); for
    /// manual pacing true once the interval has elapsed.
    pub fn should_present(&self, now: Instant) -> bool {
        if !self.is_capped() {
            return true;
        }
        if self.mode == FramePacingMode::Auto && self.v_sync {
            return true;
        }
        now.duration_since(self.last_frame_time) >= self.interval
    }

    /// Whether the loop should sleep to avoid overshooting the budget.
    /// Manual pacing (Fixed, or Auto with VSync off) sleeps; VSync-auto
    /// relies on `Fifo` but still trims when `target_fps` < refresh.
    pub fn should_sleep(&self) -> bool {
        self.is_capped()
    }

    /// Sleep until `last_frame_time + interval`, if that point is still
    /// in the future. No-op when uncapped or already behind schedule.
    /// Safe under VSync: if `Fifo` already delayed us past the deadline
    /// this returns immediately.
    pub fn sleep_until_next_frame(&mut self) {
        if !self.is_capped() {
            return;
        }
        let next = self.last_frame_time + self.interval;
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
    }

    /// Current FPS from the most recent frame duration.
    pub fn get_fps(&self) -> f32 {
        self.frame_times
            .back()
            .map(|d| {
                if d.is_zero() {
                    0.0
                } else {
                    1.0 / d.as_secs_f32()
                }
            })
            .unwrap_or(0.0)
    }

    /// Minimum FPS over the stats window.
    pub fn get_min_fps(&self) -> f32 {
        self.fps_extremes().0
    }

    /// Maximum FPS over the stats window.
    pub fn get_max_fps(&self) -> f32 {
        self.fps_extremes().1
    }

    /// Average FPS over the stats window.
    pub fn get_avg_fps(&self) -> f32 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let total: Duration = self.frame_times.iter().sum();
        let secs = total.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.frame_times.len() as f64 / secs) as f32
    }

    fn fps_extremes(&self) -> (f32, f32) {
        let mut min = f32::MAX;
        let mut max = 0.0f32;
        for d in &self.frame_times {
            if d.is_zero() {
                continue;
            }
            let fps = 1.0 / d.as_secs_f32();
            min = min.min(fps);
            max = max.max(fps);
        }
        if min == f32::MAX {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    /// Map the pacing decision onto a wgpu present mode (non-wasm only).
    ///
    /// - VSync auto/fixed → `Fifo` (vsync'd, max one frame in flight)
    /// - Uncapped or VSync off → `Immediate` when available, else `Fifo`
    #[cfg(not(target_arch = "wasm32"))]
    pub fn present_mode(&self, available: &[wgpu::PresentMode]) -> wgpu::PresentMode {
        use wgpu::PresentMode::*;
        if self.v_sync && self.mode != FramePacingMode::Uncapped && available.contains(&Fifo) {
            return Fifo;
        }
        if available.contains(&Immediate) {
            return Immediate;
        }
        if available.contains(&Mailbox) {
            return Mailbox;
        }
        available.first().copied().unwrap_or(Fifo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncapped_never_sleeps() {
        let mut p = FramePacer::with_mode(0, false, FramePacingMode::Uncapped);
        let t0 = Instant::now();
        for _ in 0..3 {
            p.record_frame_start();
            p.sleep_until_next_frame();
        }
        assert!(t0.elapsed() < Duration::from_millis(50));
        assert_eq!(p.frame_count, 3);
    }

    #[test]
    fn manual_pacing_holds_interval() {
        let mut p = FramePacer::with_mode(100, false, FramePacingMode::Fixed);
        // Warm up the clock.
        p.record_frame_start();
        std::thread::sleep(Duration::from_millis(2));
        let t0 = Instant::now();
        for _ in 0..5 {
            p.sleep_until_next_frame();
            p.record_frame_start();
        }
        // 5 frames at 100fps ≈ 50ms minimum.
        assert!(t0.elapsed() >= Duration::from_millis(40));
    }

    #[test]
    fn fps_stats_are_sane() {
        let mut p = FramePacer::with_mode(60, true, FramePacingMode::Auto);
        for _ in 0..10 {
            p.record_frame_start();
            std::thread::sleep(Duration::from_millis(8));
        }
        assert!(p.get_avg_fps() > 0.0);
        assert!(p.get_min_fps() <= p.get_max_fps() + f32::EPSILON);
        assert_eq!(p.frame_count, 10);
    }

    #[test]
    fn mode_parse() {
        assert_eq!(
            FramePacingMode::parse("fixed"),
            Some(FramePacingMode::Fixed)
        );
        assert_eq!(
            FramePacingMode::parse("UNCAPPED"),
            Some(FramePacingMode::Uncapped)
        );
        assert_eq!(FramePacingMode::parse("bogus"), None);
    }

    #[test]
    fn should_present_semantics() {
        let mut p = FramePacer::with_mode(1000, false, FramePacingMode::Fixed);
        p.record_frame_start();
        assert!(!p.should_present(Instant::now()));
        assert!(p.should_sleep());
        let p2 = FramePacer::with_mode(1000, true, FramePacingMode::Auto);
        assert!(p2.should_present(Instant::now()));
        // Auto+VSync still trims overshoot below the refresh rate.
        assert!(p2.should_sleep());
    }
}
