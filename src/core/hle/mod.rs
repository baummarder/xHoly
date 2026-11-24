//! HLE API registry (metadata only) — Phase 27.
//!
//! Tracks which Xbox modules/functions xHoly claims to support and how
//! that claim is evidenced. **No function signatures, ordinals, calling
//! conventions or return values are invented here.** The only ordinals
//! we reference are the already-verified XAM ordinals in
//! [`crate::core::kernel::xam_abi`], and those stay in that module.

use serde::{Deserialize, Serialize};

pub mod graphics;
pub mod runtime;
pub mod xmem;

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
                evidence: "role documented in the public GDK graphics docs; xHoly has no guest device shim or draw translation".into(),
            },
            ModuleMeta {
                name: "xmem.dll".into(),
                role: "Xbox memory services".into(),
                state: ImplementationState::PartiallyImplemented,
                evidence: "public GDK docs show XMemAlloc/XMemFree call shape; implemented in src/core/hle/xmem.rs (attributes not decoded)".into(),
            },
            ModuleMeta {
                name: "xg_xs.dll".into(),
                role: "Xbox texture layout and swizzle library (Series X|S)".into(),
                state: ImplementationState::Unsupported,
                evidence: "role documented in microsoft/DirectXTex and Xbox-ATG-Samples (MIT), which reference XG_XS.DLL; xHoly implements none of it".into(),
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
        // Per-symbol research (Phase 28). Every entry states whether a
        // reliable public source exists; symbols without one stay
        // `Unknown`. Sources: public GDK conceptual pages on
        // docs.xbox.com (/build/...), microsoft/DirectXTex and
        // microsoft/Xbox-ATG-Samples (MIT). The GDK reference pages
        // (/reference/...) are access-restricted and are not used as
        // documentation.
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
            // --- xmem.dll -------------------------------------------------
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemAlloc".into(),
                state: ImplementationState::PartiallyImplemented,
                evidence: "two-argument call shape (size, ULONGLONG attributes) shown in the public GDK XDSP overview on docs.xbox.com and in microsoft/DirectXTex (MIT); src/core/hle/xmem.rs allocates guest memory but does not decode attributes".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemFree".into(),
                state: ImplementationState::PartiallyImplemented,
                evidence: "called as XMemFree(pointer, attributes) in microsoft/DirectXTex (MIT); src/core/hle/xmem.rs frees guest allocations but does not decode attributes".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemAllocDefault".into(),
                state: ImplementationState::Unknown,
                evidence: "no public interface documentation found; GDK reference pages are access-restricted".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemFreeDefault".into(),
                state: ImplementationState::Unknown,
                evidence: "no public interface documentation found; GDK reference pages are access-restricted".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemVirtualAlloc".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page describes the contract (VirtualAlloc-like plus an XMemFlags field) but publishes no parameter list or flag values".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemAllocatePhysicalPages".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page describes 64 KiB physical-page semantics but publishes no signature".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemFreePhysicalPages".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page describes the role but publishes no signature".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemMapPhysicalPages".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page describes mapping semantics but publishes no signature".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemMapPhysicalPagesScatter".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page lists the function but publishes no signature".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemTransferMemory".into(),
                state: ImplementationState::Unknown,
                evidence: "no public documentation found for the Gen9 interface".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemGetWorkingSetStatistics".into(),
                state: ImplementationState::Unknown,
                evidence: "public GDK memory page describes the role (in-use footprint query) but publishes no signature".into(),
            },
            FunctionMeta {
                module: "xmem.dll".into(),
                name: "XMemIsToolsMemAvailable".into(),
                state: ImplementationState::Unknown,
                evidence: "no public documentation found for the Gen9 interface".into(),
            },
            // --- xg_xs.dll ------------------------------------------------
            FunctionMeta {
                module: "xg_xs.dll".into(),
                name: "XGCreateTextureComputer".into(),
                state: ImplementationState::Unknown,
                evidence: "no public documentation for this exact symbol (Microsoft samples use XGCreateTexture2DComputer); third-party reverse-engineering sources disagree on the interface".into(),
            },
            FunctionMeta {
                module: "xg_xs.dll".into(),
                name: "XGSuggestSwizzleMode".into(),
                state: ImplementationState::Unknown,
                evidence: "no public documentation for this exact symbol (Microsoft samples use XGComputeOptimalSwizzleMode); third-party reverse-engineering sources disagree on the interface".into(),
            },
            // --- d3d12_xs.dll ---------------------------------------------
            FunctionMeta {
                module: "d3d12_xs.dll".into(),
                name: "D3D12XboxCreateDevice".into(),
                state: ImplementationState::Unknown,
                evidence: "signature exists only in the access-restricted GDK header d3d12_xs.h; no public documentation found".into(),
            },
            FunctionMeta {
                module: "d3d12_xs.dll".into(),
                name: "D3D12ConfigureVirtualMemoryX".into(),
                state: ImplementationState::Unknown,
                evidence: "no public documentation found for the Gen9 interface".into(),
            },
            FunctionMeta {
                module: "d3d12_xs.dll".into(),
                name: "D3D12SerializeVersionedRootSignature".into(),
                state: ImplementationState::Unknown,
                evidence: "a same-named PC D3D12 API is publicly documented, but equivalence of the d3d12_xs.dll export is unverified".into(),
            },
            FunctionMeta {
                module: "d3d12_xs.dll".into(),
                name: "D3D12CreateDevice".into(),
                state: ImplementationState::Unsupported,
                evidence: "public PC D3D12 API name; xHoly has no guest device shim (see src/core/hle/graphics.rs)".into(),
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
        self.module_meta(name)
            .map(|m| m.state)
            .unwrap_or(ImplementationState::Unknown)
    }

    /// Full metadata for a module, if it is registered.
    pub fn module_meta(&self, name: &str) -> Option<&ModuleMeta> {
        self.modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// State of a function by module + name (case-insensitive).
    pub fn function_state(&self, module: &str, name: &str) -> ImplementationState {
        self.function_meta(module, name)
            .map(|f| f.state)
            .unwrap_or(ImplementationState::Unknown)
    }

    /// Full metadata for a function, if it is registered.
    pub fn function_meta(&self, module: &str, name: &str) -> Option<&FunctionMeta> {
        self.functions
            .iter()
            .find(|f| f.module.eq_ignore_ascii_case(module) && f.name.eq_ignore_ascii_case(name))
    }

    /// Every registered function of a module (case-insensitive).
    pub fn functions_for(&self, module: &str) -> Vec<&FunctionMeta> {
        self.functions
            .iter()
            .filter(|f| f.module.eq_ignore_ascii_case(module))
            .collect()
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

    #[test]
    fn gen9_symbol_states_match_the_documented_research() {
        let r = HleRegistry::with_known_entries();
        // Only the two xmem shims are (partially) implemented.
        assert_eq!(
            r.function_state("xmem.dll", "XMemAlloc"),
            ImplementationState::PartiallyImplemented
        );
        assert_eq!(
            r.function_state("xmem.dll", "XMemFree"),
            ImplementationState::PartiallyImplemented
        );
        // Everything else in the three investigated modules is Unknown:
        // no reliable public interface documentation was found.
        for symbol in [
            "XMemAllocDefault",
            "XMemFreeDefault",
            "XMemVirtualAlloc",
            "XMemAllocatePhysicalPages",
            "XMemFreePhysicalPages",
            "XMemMapPhysicalPages",
            "XMemMapPhysicalPagesScatter",
            "XMemTransferMemory",
            "XMemGetWorkingSetStatistics",
            "XMemIsToolsMemAvailable",
        ] {
            assert_eq!(
                r.function_state("xmem.dll", symbol),
                ImplementationState::Unknown,
                "{symbol}"
            );
        }
        for symbol in ["XGCreateTextureComputer", "XGSuggestSwizzleMode"] {
            assert_eq!(
                r.function_state("xg_xs.dll", symbol),
                ImplementationState::Unknown,
                "{symbol}"
            );
        }
        for symbol in [
            "D3D12XboxCreateDevice",
            "D3D12ConfigureVirtualMemoryX",
            "D3D12SerializeVersionedRootSignature",
        ] {
            assert_eq!(
                r.function_state("d3d12_xs.dll", symbol),
                ImplementationState::Unknown,
                "{symbol}"
            );
        }
        assert_eq!(
            r.module_state("xg_xs.dll"),
            ImplementationState::Unsupported
        );
    }

    #[test]
    fn every_gen9_entry_carries_evidence_and_no_ordinals_are_invented() {
        let r = HleRegistry::with_known_entries();
        let gen9_modules = ["xmem.dll", "xg_xs.dll", "d3d12_xs.dll"];
        let mut checked = 0;
        for module in gen9_modules {
            let entries = r.functions_for(module);
            assert!(!entries.is_empty(), "{module} must be researched");
            for entry in entries {
                assert!(
                    !entry.evidence.is_empty(),
                    "{}!{}",
                    entry.module,
                    entry.name
                );
                // Symbols are imported by name in the analysed executable;
                // no numeric ordinal is ever recorded here.
                assert!(
                    entry
                        .name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_?@".contains(c)),
                    "unexpected symbol name {}",
                    entry.name
                );
                checked += 1;
            }
        }
        assert!(checked >= 17, "all 17 imported symbols must be covered");
    }
}
