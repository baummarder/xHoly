//! Configuration subsystem.
//!
//! Two layers:
//! - [`settings`] — the emulation-facing `Config` (video/audio/input/
//!   emulation/profiles), TOML-persisted, used by the emulator core.
//! - Phase 11.5 infrastructure: [`infra`] (save/mods/cheats/benchmark/
//!   debug/cloud sections), [`merge`] (deep-merge), [`config_file`]
//!   (TOML/JSON load + mtime hot-reload), [`config_loader`]
//!   (CLI > file > defaults precedence), [`defaults`] (baseline TOML).

pub mod config_file;
pub mod config_loader;
pub mod defaults;
pub mod infra;
pub mod merge;
pub mod settings;

pub use config_file::{load_file, ConfigFileError, ConfigWatcher};
pub use config_loader::{into, load_effective, CliOverrides};
pub use defaults::DEFAULTS_TOML;
pub use infra::{
    BenchmarkSection, CheatSection, CloudSection, DebugSection, InfraConfig, ModSection,
    SaveSection,
};
pub use merge::{deep_merge, merge_layers, toml_to_value};
pub use settings::{AudioConfig, Config, EmulationConfig, InputConfig, Profile, VideoConfig};
