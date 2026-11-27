//! Import resolution for guest execution — Phase 29.
//!
//! Binds every parsed IAT slot to an HLE verdict. The verdict comes
//! from [`classify_import`] — the *same* function the read-only
//! compatibility report uses — so analysis and execution can never
//! disagree about an import.

use crate::core::hle::{HleRegistry, ImplementationState};
use crate::core::loader::import_analyzer::{classify_import, import_symbol_name};
use crate::core::loader::pe64::Pe64Image;
use serde::{Deserialize, Serialize};

/// One import bound to its slot in the guest IAT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedImport {
    /// Importing module (DLL), as written in the import descriptor.
    pub module: String,
    /// Symbol name, or `ordinal 0x....` when imported by ordinal.
    pub symbol: String,
    /// Guest VA of the IAT slot the CPU calls through. `None` when the
    /// descriptor has no first-thunk table — there is nothing to call.
    pub slot_va: Option<u64>,
    /// Function-level verdict (explicit entries win over module state).
    pub state: ImplementationState,
    /// Evidence from the registry (empty when nothing is documented).
    pub evidence: String,
}

impl ResolvedImport {
    /// Stable `module!symbol` display key.
    pub fn key(&self) -> String {
        format!("{}!{}", self.module, self.symbol)
    }
}

/// Resolve every import of `image` against the registry.
///
/// * symbol names come from [`import_symbol_name`] (ordinals included),
/// * verdicts come from [`classify_import`] (an explicit function
///   entry — including an explicit `Unknown` — always wins),
/// * the IAT slot is `image_base + first_thunk + 8 * index`, or `None`
///   when the descriptor declares no first-thunk table.
pub fn resolve_imports(
    image: &Pe64Image,
    registry: &HleRegistry,
    image_base: u64,
) -> Vec<ResolvedImport> {
    let mut out = Vec::new();
    for descriptor in &image.imports {
        for (index, symbol) in descriptor.symbols.iter().enumerate() {
            let name = import_symbol_name(image, symbol);
            let (state, evidence) = classify_import(registry, &descriptor.dll, &name);
            let slot_va = if descriptor.first_thunk != 0 {
                Some(image_base + u64::from(descriptor.first_thunk) + 8 * index as u64)
            } else {
                None
            };
            out.push(ResolvedImport {
                module: descriptor.dll.clone(),
                symbol: name,
                slot_va,
                state,
                evidence,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loader::pe64;

    const BASE: u64 = 0x0200_0000_0000;

    fn resolve(image: &Pe64Image) -> Vec<ResolvedImport> {
        resolve_imports(image, &HleRegistry::with_known_entries(), BASE)
    }

    #[test]
    fn known_partial_import_resolves_with_its_slot_and_evidence() {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let image = pe64::parse(&bytes).unwrap();
        let resolved = resolve(&image);
        assert_eq!(resolved.len(), 1);
        let import = &resolved[0];
        assert_eq!(import.module, "xmem.dll");
        assert_eq!(import.symbol, "XMemAlloc");
        assert_eq!(import.slot_va, Some(BASE + 0x1600), "first_thunk RVA");
        assert_eq!(import.state, ImplementationState::PartiallyImplemented);
        assert!(
            import.evidence.contains("src/core/hle/xmem.rs"),
            "evidence must point at the implementation: {}",
            import.evidence
        );
        assert_eq!(import.key(), "xmem.dll!XMemAlloc");
    }

    #[test]
    fn symbol_without_a_function_entry_stays_unknown_in_a_partial_module() {
        // xmem.dll is partially implemented, but this symbol is not an
        // xmem entry: no module credit may be inherited.
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "D3D12CreateDevice");
        let image = pe64::parse(&bytes).unwrap();
        let resolved = resolve(&image);
        assert_eq!(resolved[0].state, ImplementationState::Unknown);
        assert_eq!(resolved[0].symbol, "D3D12CreateDevice");
    }

    #[test]
    fn explicit_unknown_wins_over_an_unsupported_module() {
        let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12XboxCreateDevice");
        let image = pe64::parse(&bytes).unwrap();
        let resolved = resolve(&image);
        assert_eq!(
            resolved[0].state,
            ImplementationState::Unknown,
            "explicit research verdict must win over the module state"
        );
        assert!(!resolved[0].evidence.is_empty());
    }

    #[test]
    fn unsupported_module_classifies_symbols_without_their_own_entry() {
        let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12CreateDevice");
        let image = pe64::parse(&bytes).unwrap();
        let resolved = resolve(&image);
        assert_eq!(resolved[0].state, ImplementationState::Unsupported);
    }

    #[test]
    fn ordinal_imports_keep_their_documented_display_form() {
        // Import by ordinal: set the high bit in the IAT thunk. No
        // verified Gen9 ordinal table exists, so the verdict is Unknown
        // — the name is preserved as the compatibility report shows it.
        let mut bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        bytes[0xA00..0xA08].copy_from_slice(&(1u64 << 63 | 0x0005).to_le_bytes());
        let image = pe64::parse(&bytes).unwrap();
        let resolved = resolve(&image);
        assert_eq!(resolved[0].symbol, "ordinal 0x0005");
        assert_eq!(resolved[0].state, ImplementationState::Unknown);
        assert_eq!(resolved[0].slot_va, Some(BASE + 0x1600));
    }

    #[test]
    fn descriptor_without_first_thunk_has_no_slot() {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let mut image = pe64::parse(&bytes).unwrap();
        image.imports[0].first_thunk = 0;
        let resolved = resolve(&image);
        assert_eq!(resolved[0].slot_va, None);
        assert_eq!(resolved.len(), 1, "the import itself is still recorded");
    }
}
