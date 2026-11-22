//! xHoly — research emulator/simulator architecture.
//!
//! This crate implements the host-side core of the xHoly project:
//! a hybrid Rust/Scala emulation stack targeting Xbox Series S|X and
//! Xbox One game images. See `docs/whitepaper.md` for the architecture.

pub mod audio;
pub mod benchmark;
pub mod cheat_engine;
pub mod cloud;
pub mod config;
pub mod core;
pub mod debug_menu;
pub mod easter_egg;
pub mod gpu;
pub mod gui;
pub mod input;
pub mod io;
pub mod jit;
pub mod logging;
pub mod mod_system;
pub mod multiplayer;
pub mod network;
pub mod optical;
pub mod runtime;
pub mod save;
pub mod shader_cache;
pub mod storage;
pub mod telemetry;
pub mod utils;

use std::time::Duration;

/// Top-level emulation handle returned by [`Emulator::new`].
pub struct Emulator {
    core: core::EmulationCore,
    gpu: gpu::GpuContext,
    config: config::Config,
}

impl Emulator {
    /// Create an emulator instance with the given game image path.
    pub fn new(image_path: impl AsRef<std::path::Path>, config: config::Config) -> Self {
        let core = core::EmulationCore::load(image_path.as_ref());
        let gpu = gpu::GpuContext::new(&config);
        Self { core, gpu, config }
    }

    /// Run the emulation loop until the game exits or `max_frames` is hit.
    /// Returns the number of frames rendered.
    pub fn run(&mut self, max_frames: Option<u64>) -> u64 {
        let mut frames = 0u64;
        while max_frames.is_none_or(|m| frames < m) {
            let frame = self.core.step_frame(&mut self.gpu);
            frames += 1;
            if frame.finished {
                break;
            }
            // Frame pacing: target ~60 FPS (16.666 ms per frame).
            let frame_micros = 16_666u64;
            let remaining = frame_micros.saturating_sub(frame.elapsed_micros);
            if remaining > 0 {
                std::thread::sleep(Duration::from_micros(remaining));
            }
        }
        frames
    }

    /// Active configuration (exposed for hot-reload consumers).
    pub fn config(&self) -> &config::Config {
        &self.config
    }

    /// Enable the x86-64 → WASM JIT branch on the emulation core.
    /// The JIT runs parallel to the ARM64 interpreter and is used by
    /// [`core::EmulationCore::compile_and_run`] for x86-64 guest code.
    pub fn enable_jit(&mut self, jit_config: jit::JitConfig) {
        self.core.enable_jit(jit_config);
    }

    /// Access the emulation core (e.g. for `compile_and_run`).
    pub fn core_mut(&mut self) -> &mut core::EmulationCore {
        &mut self.core
    }

    /// Take a screenshot of the current framebuffer and write it to `path`.
    pub fn screenshot(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let rgba = self.gpu.readback_rgba();
        crate::io::screenshot::write_png(path.as_ref(), rgba, self.gpu.width(), self.gpu.height())
    }
}
