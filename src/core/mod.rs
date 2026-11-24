//! Core emulation subsystem: CPU, memory, and frame loop.

pub mod boot;
pub mod cpu;
pub mod emulation;
pub mod execution;
pub mod gpu;
pub mod hle;
pub mod kernel;
pub mod loader;
pub mod memory;
pub mod native;
#[cfg(any(test, feature = "legacy-xbox360"))]
pub mod ppc;
pub mod shader;

pub use emulation::{EmulationCore, FrameResult};
pub use memory::Memory;
