//! GPU subsystem: shader translation, renderer backend, and fallback.

pub mod passthrough;
pub mod shader;
pub mod translation;

pub use passthrough::GpuContext;
