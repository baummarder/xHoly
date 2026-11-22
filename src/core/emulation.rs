//! Core emulation state: CPU, memory, and the frame-stepping loop.

use crate::core::{cpu, memory};
use crate::gpu::GpuContext;
use crate::jit::{JitConfig, JitError, WasmJitEmulator};

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
    /// Optional x86-64 → WASM JIT branch (Phase 14). Runs parallel to
    /// the ARM64-style `cpu` interpreter — never hybridized.
    jit: Option<WasmJitEmulator>,
    jit_config: JitConfig,
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
            jit: None,
            jit_config: JitConfig::default(),
        }
    }

    /// Whether a game image was successfully loaded.
    pub fn has_image(&self) -> bool {
        self.loaded_image.is_some()
    }

    /// Attach the x86-64 JIT branch (enabled via [`JitConfig`]).
    pub fn enable_jit(&mut self, config: JitConfig) {
        self.jit_config = config;
        self.jit = Some(WasmJitEmulator::new());
    }

    /// Current JIT configuration.
    pub fn jit_config(&self) -> &JitConfig {
        &self.jit_config
    }

    /// Access the JIT emulator (if enabled).
    pub fn jit(&self) -> Option<&WasmJitEmulator> {
        self.jit.as_ref()
    }

    /// Access the JIT emulator mutably (e.g. to install a syscall
    /// handler or inspect interpreter state).
    pub fn jit_mut(&mut self) -> Option<&mut WasmJitEmulator> {
        self.jit.as_mut()
    }

    /// Whether the JIT branch is enabled.
    pub fn jit_enabled(&self) -> bool {
        self.jit_config.enabled
    }

    /// Compile and run an x86-64 program.
    ///
    /// - JIT enabled → compile to WASM and execute in the runtime.
    /// - JIT disabled or runtime failure with
    ///   `fallback_to_interpreter` → scalar [`X86Interpreter`] fallback
    ///   over the same decoded subset.
    /// - JIT disabled and fallback off → [`JitError::FallbackDisabled`].
    ///
    /// Note: the ARM64-style `cpu` interpreter executes a different
    /// (RV32-like) guest ISA and cannot run x86-64 code — the scalar
    /// fallback inside the JIT subsystem is the interpreter path.
    pub fn compile_and_run(&mut self, program: &[u8]) -> Result<u64, JitError> {
        if let Some(jit) = self.jit.as_mut() {
            return jit.run_program(program, &self.jit_config);
        }
        // No JIT attached: still honor the fallback policy via a
        // transient emulator.
        let mut jit = WasmJitEmulator::new();
        jit.run_program(program, &self.jit_config)
    }

    /// Explicit interpreter fallback (ignores `enabled`, still honors
    /// `fallback_to_interpreter` policy via the JIT's scalar engine).
    pub fn fallback_to_interpreter(&mut self, program: &[u8]) -> Result<u64, JitError> {
        let cfg = JitConfig {
            enabled: false,
            fallback_to_interpreter: true,
            ..self.jit_config.clone()
        };
        let mut jit = self.jit.take().unwrap_or_default();
        let r = jit.run_program(program, &cfg);
        self.jit = Some(jit);
        r
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
