//! Mod/plugin system — Phase 11.5.
//!
//! Manifest-driven mod loading: parse `xholy.mod.toml`, resolve
//! dependencies (semver-ish), register in a local/remote registry,
//! and drive the plugin lifecycle Init → Enable → Update → Disable →
//! Unload. Mods are *data + hooks*, not native code — the executor
//! calls registered Rust callbacks.

pub mod mod_executor;
pub mod mod_loader;
pub mod mod_manifest;
pub mod mod_registry;
pub mod mod_resolver;

pub use mod_executor::{LifecycleState, ModExecutor, ModHooks};
pub use mod_loader::{load_manifest_file, LoadedMod, ModLoadError};
pub use mod_manifest::{ModDependency, ModManifest, Version};
pub use mod_registry::{ModRegistry, RegistryError};
pub use mod_resolver::{resolve, ResolutionError, ResolvedMod};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::Version::new(1, 0, 0);
    }
}
