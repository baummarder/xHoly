//! Core emulation state: CPU, memory, and the frame-stepping loop.

use crate::core::{cpu, memory};
use crate::gpu::GpuContext;

/// Result of stepping a single emulated frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameResult {
    /// Whether the emulated game has signalled exit.
    pub finished: bool,
    /// Wall-clock time spent emulating this frame, in microseconds.
    pub elapsed_micros: u64,
}

/// The emulation core: owns CPU, memory, and timing state.
pub struct EmulationCore {
    cpu: cpu::Cpu,
    memory: memory::Memory,
    loaded_image: Option<std::path::PathBuf>,
    frame_count: u64,
}

impl EmulationCore {
    /// Create a core and attempt to load a game image.
    ///
    /// Image loading is intentionally permissive: any readable file or
    /// directory is accepted and mapped into guest memory. Full XVC/ISO
    /// parsing is tracked as future work (see docs/whitepaper.md).
    pub fn load(image_path: &std::path::Path) -> Self {
        let mut memory = memory::Memory::new();
        let loaded = match std::fs::read(image_path) {
            Ok(bytes) => {
                log::info!("xHoly: loaded {} bytes from {:?}", bytes.len(), image_path);
                memory.map_image(&bytes);
                Some(image_path.to_path_buf())
            }
            Err(e) => {
                log::warn!("xHoly: could not read {:?}: {}", image_path, e);
                None
            }
        };
        Self {
            cpu: cpu::Cpu::new(),
            memory,
            loaded_image: loaded,
            frame_count: 0,
        }
    }

    /// Whether a game image was successfully loaded.
    pub fn has_image(&self) -> bool {
        self.loaded_image.is_some()
    }

    /// Step one emulated frame: run the CPU until the next vblank, then
    /// present the GPU frame.
    pub fn step_frame(&mut self, gpu: &mut GpuContext) -> FrameResult {
        let start = std::time::Instant::now();
        self.cpu.run_until_vblank(&mut self.memory, gpu);
        self.frame_count += 1;
        FrameResult {
            finished: self.cpu.is_halted(),
            elapsed_micros: start.elapsed().as_micros() as u64,
        }
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}
