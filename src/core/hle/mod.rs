//! HLE API registry (metadata only) — Phase 27.
//!
//! Tracks which Xbox modules/functions xHoly claims to support and how
//! that claim is evidenced. **No function signatures, ordinals, calling
//! conventions or return values are invented here.** The only ordinals
//! we reference are the already-verified XAM ordinals in
//! [`crate::core::kernel::xam_abi`], and those stay in that module.

use serde::{Deserialize, Serialize};

/// Implementation state of a module or function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationState {
    /// Fully implemented and covered by tests.
    Implemented,
    /// Some behaviour exists; important parts are missing.
    PartiallyImplemented,
    /// Recognised but deliberately not implemented.
    Unsupported,
    /// No reliable information available.
    Unknown,
}

impl ImplementationState {
    /// Stable display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Implemented => "Implemented",
            Self::PartiallyImplemented => "Partially Implemented",
            Self::Unsupported => "Unsupported",
            Self::Unknown => "Unknown",
        }
    }
}

/// Metadata for one imported module (DLL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleMeta {
    /// DLL name as it appears in the PE import table (lower-case).
    pub name: String,
    /// Short, factual description of the module's role.
    pub role: String,
    /// Implementation state in xHoly.
    pub state: ImplementationState,
    /// Source that documents the claim (repo file or public docs).
    pub evidence: String,
}

/// Metadata for one imported function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionMeta {
    /// Owning module name (lower-case).
    pub module: String,
    /// Exported symbol name. No ordinal guesses.
    pub name: String,
    /// Implementation state in xHoly.
    pub state: ImplementationState,
    /// Evidence for the state; empty only for `Unknown`.
    pub evidence: String,
}

/// A modular, data-only HLE registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HleRegistry {
    /// Known modules.
    pub modules: Vec<ModuleMeta>,
    /// Known functions.
    pub functions: Vec<FunctionMeta>,
}

impl HleRegistry {
    /// Build the registry seeded from verified project sources.
    pub fn with_known_entries() -> Self {
        let modules = vec![
            ModuleMeta {
                name: "xam.xex".into(),
                role: "Xbox XAM user/profile services".into(),
                state: ImplementationState::Unknown,
                evidence: "ordinals in src/core/kernel/xam_abi.rs come from the Xbox 360 table in Xenia; not verified for Series X|S (ADR 0001)".into(),
            },
            ModuleMeta {
                name: "xgameruntime.dll".into(),
                role: "Game Core runtime".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README (HLE target); no xHoly implementation".into(),
            },
            ModuleMeta {
                name: "d3d12_xs.dll".into(),
                role: "Xbox D3D12 graphics runtime".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README (HLE target); no guest draw translation".into(),
            },
            ModuleMeta {
                name: "xmem.dll".into(),
                role: "Xbox memory services".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README (HLE target); no xHoly implementation".into(),
            },
            ModuleMeta {
                name: "gameinput.dll".into(),
                role: "GameInput controller API".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README (HLE target); no xHoly implementation".into(),
            },
            ModuleMeta {
                name: "msvcp140.dll".into(),
                role: "MSVC C++ runtime (host library)".into(),
                state: ImplementationState::Unsupported,
                evidence: "not an Xbox API; xHoly does not emulate a host CRT".into(),
            },
        ];
        let functions = vec![
            FunctionMeta {
                module: "xam.xex".into(),
                name: "XamUserGetSigninState".into(),
                state: ImplementationState::Unknown,
                evidence: "Xbox 360 XAM ordinal 0x210 (Xenia table); no verified Series X|S signature (ADR 0001)".into(),
            },
            FunctionMeta {
                module: "xam.xex".into(),
                name: "XamInputGetState".into(),
                state: ImplementationState::Unknown,
                evidence: "Xbox 360 XAM ordinal 0x191 (Xenia table); no verified Series X|S signature (ADR 0001)".into(),
            },
            FunctionMeta {
                module: "d3d12_xs.dll".into(),
                name: "D3D12XboxCreateDevice".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README; xHoly has no guest device shim".into(),
            },
            FunctionMeta {
                module: "gameinput.dll".into(),
                name: "GameInputCreate".into(),
                state: ImplementationState::Unsupported,
                evidence: "ScarlettX README (verified ordinal #1); xHoly has no shim".into(),
            },
        ];
        Self { modules, functions }
    }

    /// State of a module by name (case-insensitive).
    pub fn module_state(&self, name: &str) -> ImplementationState {
        self.modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .map(|m| m.state)
            .unwrap_or(ImplementationState::Unknown)
    }

    /// State of a function by module + name (case-insensitive).
    pub fn function_state(&self, module: &str, name: &str) -> ImplementationState {
        self.functions
            .iter()
            .find(|f| f.module.eq_ignore_ascii_case(module) && f.name.eq_ignore_ascii_case(name))
            .map(|f| f.state)
            .unwrap_or(ImplementationState::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_have_stable_labels() {
        assert_eq!(ImplementationState::Implemented.label(), "Implemented");
        assert_eq!(
            ImplementationState::PartiallyImplemented.label(),
            "Partially Implemented"
        );
        assert_eq!(ImplementationState::Unsupported.label(), "Unsupported");
        assert_eq!(ImplementationState::Unknown.label(), "Unknown");
    }

    #[test]
    fn known_registry_uses_verified_evidence_only() {
        let r = HleRegistry::with_known_entries();
        assert_eq!(
            r.module_state("D3D12_XS.DLL"),
            ImplementationState::Unsupported
        );
        // The XAM entries are not claimed as implemented: their ordinals are
        // Xbox 360 values and must not be used as Series X|S evidence.
        assert_eq!(r.module_state("xam.xex"), ImplementationState::Unknown);
        assert_eq!(
            r.function_state("xam.xex", "XamUserGetSigninState"),
            ImplementationState::Unknown
        );
        assert_eq!(
            r.function_state("xam.xex", "NotARealFunction"),
            ImplementationState::Unknown
        );
        for f in &r.functions {
            assert!(!f.evidence.is_empty(), "{} must cite evidence", f.name);
        }
    }

    #[test]
    fn unknown_modules_do_not_claim_support() {
        let r = HleRegistry::default();
        assert_eq!(r.module_state("unknown.dll"), ImplementationState::Unknown);
        assert_eq!(
            r.function_state("unknown.dll", "Whatever"),
            ImplementationState::Unknown
        );
    }
}
