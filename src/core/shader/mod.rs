//! Game-shader translation — Phase 10.
//!
//! Pipeline: DXBC/DXIL container → [`dxil_parser`] →
//! [`hlsl_compiler`] IR → [`spirv_generator`] + [`wgsl_emitter`] →
//! [`shader_cache`] artifact.
//!
//! Honest scope: real LLVM bitcode decoding is *not* implemented —
//! [`hlsl_compiler::lower`] recognizes a host-generated mini-format
//! (`XHLS`) for tests/tools and reports `Unsupported` for retail DXIL,
//! which routes to the WGSL blit fallback. The container/signature
//! parsing, IR, SPIR-V serialization, WGSL emission, caching and
//! resource-binding model are all real.

pub mod dxil_parser;
pub mod hlsl_compiler;
pub mod shader_binder;
pub mod shader_cache;
pub mod spirv_generator;
pub mod wgsl_emitter;

pub use shader_binder::{ResourceBinding, ResourceKind, ResourceTable};
pub use shader_cache::{compile_shader, GameShaderCache, ShaderArtifact};
