//! XAM import stub — Phase 17.
//!
//! Retail Xbox games don't call raw `SVC`s; they import functions from
//! system XEX libraries (`xam.xex`, `xaminput.xex`, `d3d11.xex`, ...)
//! through a kernel thunk table resolved at load time. This module is
//! the honest stub for that ABI:
//!
//! - [`import_stub_va`] hands out deterministic guest VAs inside the
//!   kernel region for each resolved import (used by
//!   [`crate::core::loader::xex_parser::XexHeader::resolve_imports`]).
//! - [`XamTable`] records which XAM exports a game asked for and
//!   answers the common ones (sign-in state, title storage, message
//!   boxes) with plausible success values.
//!
//! Nothing here implements real XAM semantics — it exists so the
//! loader→kernel path has a defined place for imports and so guest
//! code probing import slots sees stable addresses.

/// Guest VA range for import thunks: just below KERNEL_BASE's used
/// area, inside the mapped kernel window.
pub const IMPORT_STUB_BASE: u64 = 0x8000_0000_F000;
/// Bytes reserved per thunk stub.
pub const THUNK_STRIDE: u64 = 0x10;
/// Maximum number of stubbed imports.
pub const MAX_IMPORTS: u32 = 256;

/// Deterministic thunk VA for import index `i`.
pub fn import_stub_va(index: u32) -> u64 {
    IMPORT_STUB_BASE + (index.min(MAX_IMPORTS - 1) as u64) * THUNK_STRIDE
}

/// A single XAM import the game requested.
#[derive(Debug, Clone)]
pub struct XamImport {
    /// Library the import came from (`xam.xex`, ...).
    pub library: String,
    /// Export name.
    pub name: String,
    /// Thunk VA assigned by the resolver.
    pub thunk_va: u64,
}

/// Known XAM exports and their stubbed return values.
///
/// Values are chosen to look like success to typical caller checks:
/// handles are small nonzero ids, states are "signed in", flags are 1.
#[derive(Debug, Default)]
pub struct XamTable {
    /// Imports registered at load time.
    pub imports: Vec<XamImport>,
    /// Lookup: normalized `LIBRARY!NAME` → stubbed return value.
    /// `None` means "unknown export" → caller returns an error code.
    known: Vec<(String, i64)>,
}

impl XamTable {
    /// New table seeded with the common XAM exports.
    pub fn new() -> Self {
        let known = [
            ("XAM.xex!XamUserGetSigninState", 1i64), // ERROR_SUCCESS-ish / signed-in
            ("XAM.xex!XamUserGetName", 0),
            ("XAM.xex!XamShowMessageBoxUI", 0),
            ("XAM.xex!XamLoaderLaunchTitle", 0),
            ("XAM.xex!XamLoaderTerminateTitle", 0),
            ("XAM.xex!XShowWriteProfile", 0),
            ("XAM.xex!XamLoaderCreateNamedSection", 1),
            ("XAM.xex!XamLoaderDeleteNamedSection", 0),
            ("XAMINPUT.xex!XamInputGetState", 0),
            ("XAMINPUT.xex!XamInputSetState", 0),
            ("D3D11.xex!D3D11CreateDevice", 0),
            ("D3D11.xex!D3D11CreateDeviceAndSwapChain", 0),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
        Self {
            imports: Vec::new(),
            known,
        }
    }

    /// Register an import resolved by the XEX parser.
    pub fn register(&mut self, library: &str, name: &str, thunk_va: u64) {
        self.imports.push(XamImport {
            library: library.to_string(),
            name: name.to_string(),
            thunk_va,
        });
    }

    /// Stubbed return value for an export, or `None` when unknown.
    pub fn call_stub(&self, library: &str, name: &str) -> Option<i64> {
        let key = format!("{}!{}", library.to_ascii_uppercase(), name);
        self.known
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
            .map(|(_, v)| *v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thunk_vas_are_stable_and_distinct() {
        assert_eq!(import_stub_va(0), IMPORT_STUB_BASE);
        assert_eq!(import_stub_va(1), IMPORT_STUB_BASE + THUNK_STRIDE);
        assert_eq!(import_stub_va(300), import_stub_va(MAX_IMPORTS - 1));
    }

    #[test]
    fn xam_table_answers_known_exports() {
        let mut t = XamTable::new();
        assert_eq!(t.call_stub("xam.xex", "XamUserGetSigninState"), Some(1));
        assert_eq!(t.call_stub("XAMINPUT.xex", "XamInputGetState"), Some(0));
        assert_eq!(t.call_stub("xam.xex", "NoSuchExport"), None);
        t.register("xam.xex", "XamUserGetSigninState", import_stub_va(0));
        assert_eq!(t.imports.len(), 1);
        assert_eq!(t.imports[0].thunk_va, IMPORT_STUB_BASE);
    }
}
