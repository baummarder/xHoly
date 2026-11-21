//! Core emulation subsystem: CPU, memory, and frame loop.

pub mod cpu;
pub mod emulation;
pub mod memory;

pub use emulation::{EmulationCore, FrameResult};
pub use memory::Memory;
