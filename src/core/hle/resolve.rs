//! Pure HLE API resolution — Milestone 36.1.
//!
//! A **single source of truth** for the question "what does xHoly claim
//! about this module!symbol?". Registry data stays data-only; this module
//! holds no state and performs no dispatch.
//!
//! Verdict precedence (documented, deterministic):
//!
//! 1. an explicit function entry always wins — **including an explicit
//!    `Unknown` or `Unsupported`**;
//! 2. only a symbol without its own entry may inherit an explicitly
//!    `Unsupported` module state;
//! 3. everything else (including `PartiallyImplemented` modules) stays
//!    `Unknown`.
//!
//! This deliberately never upgrades a symbol to `Implemented`, never
//! invents a signature and never produces a value. It is shared by the
//! read-only import analyzer, the guest import binder and the dispatch
//! runtime, so those three can no longer disagree.

use super::{HleRegistry, ImplementationState};

/// Verdict for one `module!symbol` lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiVerdict {
    /// Claimed implementation state.
    pub state: ImplementationState,
    /// Evidence from the explicit function entry, the module, or empty.
    pub evidence: String,
    /// Where the verdict came from (useful in diagnostics).
    pub source: VerdictSource,
}

impl ApiVerdict {
    /// Build the verdict for `module!symbol` against `registry`.
    pub fn resolve(registry: &HleRegistry, module: &str, symbol: &str) -> Self {
        if let Some(function) = registry.function_meta(module, symbol) {
            return Self {
                state: function.state,
                evidence: function.evidence.clone(),
                source: VerdictSource::Function,
            };
        }
        if let Some(module_meta) = registry.module_meta(module) {
            if module_meta.state == ImplementationState::Unsupported {
                return Self {
                    state: ImplementationState::Unsupported,
                    evidence: module_meta.evidence.clone(),
                    source: VerdictSource::Module,
                };
            }
        }
        Self {
            state: ImplementationState::Unknown,
            evidence: String::new(),
            source: VerdictSource::Default,
        }
    }
}

/// Provenance of an [`ApiVerdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictSource {
    /// An explicit `FunctionMeta` entry decided.
    Function,
    /// An explicitly unsupported module decided (no function entry).
    Module,
    /// No evidence: the symbol is preserved as `Unknown`.
    Default,
}

/// Resolve a verdict as `(state, evidence)` — the signature used by the
/// read-only compatibility report and the guest import binder.
pub fn resolve_verdict(
    registry: &HleRegistry,
    module: &str,
    symbol: &str,
) -> (ImplementationState, String) {
    let verdict = ApiVerdict::resolve(registry, module, symbol);
    (verdict.state, verdict.evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_function_entry_always_wins() {
        let registry = HleRegistry::with_known_entries();
        let verdict = ApiVerdict::resolve(&registry, "xmem.dll", "XMemAlloc");
        assert_eq!(verdict.state, ImplementationState::PartiallyImplemented);
        assert_eq!(verdict.source, VerdictSource::Function);
        assert!(!verdict.evidence.is_empty());

        // An explicit Unknown wins even inside a partial module.
        let verdict = ApiVerdict::resolve(&registry, "xmem.dll", "XMemVirtualAlloc");
        assert_eq!(verdict.state, ImplementationState::Unknown);
        assert_eq!(verdict.source, VerdictSource::Function);
    }

    #[test]
    fn only_explicitly_unsupported_modules_inherit_credit() {
        let registry = HleRegistry::with_known_entries();
        let verdict = ApiVerdict::resolve(&registry, "d3d12_xs.dll", "NotListed");
        assert_eq!(verdict.state, ImplementationState::Unsupported);
        assert_eq!(verdict.source, VerdictSource::Module);

        // A partially implemented module never inherits credit for an
        // unlisted function.
        let verdict = ApiVerdict::resolve(&registry, "xmem.dll", "NotListed");
        assert_eq!(verdict.state, ImplementationState::Unknown);
        assert_eq!(verdict.source, VerdictSource::Default);
        assert!(verdict.evidence.is_empty());
    }

    #[test]
    fn unknown_modules_and_symbols_stay_unknown() {
        let registry = HleRegistry::with_known_entries();
        let (state, evidence) = resolve_verdict(&registry, "mystery.dll", "DoThing");
        assert_eq!(state, ImplementationState::Unknown);
        assert!(evidence.is_empty());
    }

    #[test]
    fn explicit_unsupported_symbol_beats_a_partial_module() {
        let registry = HleRegistry::with_known_entries();
        let verdict = ApiVerdict::resolve(&registry, "d3d12_xs.dll", "D3D12CreateDevice");
        assert_eq!(verdict.state, ImplementationState::Unsupported);
        assert_eq!(verdict.source, VerdictSource::Function);
    }
}
