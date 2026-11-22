//! Easter egg engine — timing, animation loop, once-per-session guard.

use std::io::Write;
use std::time::{Duration, Instant};

use super::config::EasterEggConfig;
use super::display::{self, EggInfo};
use super::session_state::SessionState;

/// Frame budget: ~30 fps animation regardless of duration.
const FRAME_MS: u64 = 33;

/// The start screen.
pub struct EasterEgg {
    config: EasterEggConfig,
    state: &'static SessionState,
}

impl EasterEgg {
    /// New egg with default session state.
    pub fn new(config: EasterEggConfig) -> Self {
        Self {
            config,
            state: SessionState::global(),
        }
    }

    /// New egg bound to a specific session state (tests).
    pub fn with_state(config: EasterEggConfig, state: &'static SessionState) -> Self {
        Self { config, state }
    }

    /// Render the egg for `duration_ms`, once per session.
    ///
    /// Returns `true` when the egg was actually displayed. Honors the
    /// once-per-session guard unless the caller already bypassed it
    /// (see [`cli`](super::cli)).
    pub fn show_if_needed(&self) -> bool {
        if !self.state.mark_shown() {
            return false;
        }
        self.show();
        true
    }

    /// Render the egg unconditionally (force path / explicit flag).
    pub fn show(&self) {
        let info = self.collect_info();
        let start = Instant::now();
        let total = Duration::from_millis(self.config.duration_ms.max(1));
        let mut frame = 0u64;

        let mut out = std::io::stderr().lock();
        loop {
            let elapsed = start.elapsed();
            if elapsed >= total {
                break;
            }
            let t = (elapsed.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0);
            let _ = render_to(&mut out, &self.config, &info, t);
            frame += 1;
            let target = Duration::from_millis(frame * FRAME_MS);
            if start.elapsed() < target {
                std::thread::sleep(target - start.elapsed());
            }
        }
        // Clear the egg area so the real app starts on a clean console.
        let _ = write!(
            out,
            "\x1b[{}A\x1b[J",
            display::render_frame(&self.config, &info, 0.5).len()
        );
        let _ = out.flush();
    }

    /// Best-effort GPU name + commit hash (never fails the egg).
    fn collect_info(&self) -> EggInfo {
        EggInfo {
            gpu_name: self.config.show_gpu_info.then(detect_gpu_name).flatten(),
            commit: self.config.show_commit_hash.then(detect_commit).flatten(),
        }
    }
}

/// Draw one frame with ANSI colors into `out`.
fn render_to(
    out: &mut impl Write,
    config: &EasterEggConfig,
    info: &EggInfo,
    t: f64,
) -> std::io::Result<()> {
    use display::{fade, BG, BOLD, GOLD, RESET};
    let lines = display::render_frame(config, info, t);
    let f = fade(t);
    // Fade via dim attribute: bright gold mid-way, darker at edges.
    let intensity = if f < 0.5 { 2 } else { 1 }; // faint ↔ bold-ish
    for line in &lines {
        if line.trim().is_empty() {
            writeln!(out)?;
            continue;
        }
        write!(out, "{BG}{GOLD}")?;
        if intensity == 1 {
            write!(out, "{BOLD}")?;
        }
        writeln!(out, "{line}{RESET}")?;
    }
    out.flush()
}

/// Ask wgpu for the default adapter name (short timeout-ish; best effort).
fn detect_gpu_name() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        Some(adapter.get_info().name)
    }
}

/// `git rev-parse --short HEAD`, best effort.
fn detect_commit() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn test_state() -> &'static SessionState {
        static S: OnceLock<SessionState> = OnceLock::new();
        S.get_or_init(SessionState::default)
    }

    #[test]
    fn shows_once_per_session() {
        test_state().reset();
        // Minimal config + tiny duration: show once, second call skipped.
        let mut cfg = EasterEggConfig::minimal();
        cfg.duration_ms = 20;
        let egg = EasterEgg::with_state(cfg, test_state());
        assert!(egg.show_if_needed());
        assert!(!egg.show_if_needed());
    }

    #[test]
    fn duration_is_respected() {
        let mut cfg = EasterEggConfig::minimal();
        cfg.duration_ms = 60;
        let egg = EasterEgg::with_state(cfg, test_state());
        let start = Instant::now();
        egg.show();
        assert!(start.elapsed() >= Duration::from_millis(55));
    }

    #[test]
    fn show_does_not_block_forever() {
        let mut cfg = EasterEggConfig::minimal();
        cfg.duration_ms = 30;
        let egg = EasterEgg::with_state(cfg, test_state());
        egg.show(); // returns — app continues
    }
}
