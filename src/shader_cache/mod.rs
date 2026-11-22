//! Shader cache — Phase 11.5.
//!
//! Central shader cache *above* `core::shader::shader_cache` (which
//! caches DXIL→SPIR-V/WGSL artifacts). This layer caches *any* keyed
//! blob (compiled pipelines, translated shaders, pipeline state)
//! with FNV-1a keys, LRU eviction, and optional disk persistence.

pub mod cache;
pub mod cache_manager;
pub mod cache_store;

pub use cache::{CacheKey, ShaderCache};
pub use cache_manager::{CacheManager, CacheStats};
pub use cache_store::{CacheStore, MemoryStore};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::CacheStats::default();
    }
}
