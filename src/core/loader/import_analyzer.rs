//! Executable import analyzer + structured compatibility report (Phase 27).
//!
//! Reads the PE32+ import table of a parsed image, compares it with the
//! HLE registry and reports each category. Unknown imports are preserved
//! verbatim — never silently dropped. This module never executes guest
//! code and never writes to the analysed file.

use crate::core::hle::{HleRegistry, ImplementationState};
use crate::core::loader::pe64::{Import, Pe64Image};
use serde::{Deserialize, Serialize};

/// One analysed import with its HLE state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzedImport {
    /// Importing module (DLL).
    pub module: String,
    /// Symbol name, or `ordinal 0x....` when imported by ordinal.
    pub symbol: String,
    /// HLE state.
    pub state: ImplementationState,
    /// Evidence from the registry (empty for unknowns).
    pub evidence: String,
}

/// Grouped import analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportAnalysis {
    /// Implemented imports.
    pub implemented: Vec<AnalyzedImport>,
    /// Partially implemented imports.
    pub partial: Vec<AnalyzedImport>,
    /// Recognised but unsupported imports.
    pub unsupported: Vec<AnalyzedImport>,
    /// Unknown imports (kept for diagnosis).
    pub unknown: Vec<AnalyzedImport>,
}

impl ImportAnalysis {
    /// Total number of analysed import slots.
    pub fn total(&self) -> usize {
        self.implemented.len() + self.partial.len() + self.unsupported.len() + self.unknown.len()
    }
}

/// Structured compatibility report for one executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// File name only (no local path is reported).
    pub executable: String,
    /// Image format, e.g. `PE32+`.
    pub format: String,
    /// Guest architecture from the COFF machine field.
    pub architecture: String,
    /// Console generation, only when reliable evidence exists.
    pub console_generation: Option<String>,
    /// Evidence lines for the generation inference.
    pub generation_evidence: Vec<String>,
    /// All imports, categorised.
    pub imports: ImportAnalysis,
    /// Missing or unknown dependencies (module-level).
    pub missing_dependencies: Vec<String>,
    /// Explicit limitations.
    pub limitations: Vec<String>,
}

/// Resolve an import into a stable display string.
///
/// Ordinals are rendered exactly as the compatibility report renders
/// them, so analysis and execution resolve an import to the same
/// verdict. Ordinal *lookup* is not implemented: the registry records
/// symbol names, and no verified Gen9 ordinal table is public.
pub fn import_symbol_name(image: &Pe64Image, import: &Import) -> String {
    match import {
        Import::ByOrdinal(ordinal) => format!("ordinal 0x{ordinal:04X}"),
        Import::ByName(rva) => image
            .import_name(*rva)
            .unwrap_or_else(|| format!("name@{rva:#x} (unresolved)")),
    }
}

/// The single source of truth for one import's verdict.
///
/// An explicit function-level registry entry always wins — including an
/// explicit `Unknown` research verdict. Only symbols with **no** entry
/// fall back to the module state, and only an explicitly unsupported
/// module classifies them as unsupported: a partially implemented module
/// must never credit functions it does not provide.
///
/// Both the read-only compatibility report and the guest execution
/// pipeline call this function, so the two can never disagree.
pub fn classify_import(
    registry: &HleRegistry,
    module: &str,
    symbol: &str,
) -> (ImplementationState, String) {
    let module_meta = registry.module_meta(module);
    let function_meta = registry.function_meta(module, symbol);
    let state = match function_meta {
        Some(meta) => meta.state,
        None if module_meta.map(|m| m.state) == Some(ImplementationState::Unsupported) => {
            ImplementationState::Unsupported
        }
        None => ImplementationState::Unknown,
    };
    let evidence = function_meta
        .map(|f| f.evidence.clone())
        .or_else(|| module_meta.map(|m| m.evidence.clone()))
        .unwrap_or_default();
    (state, evidence)
}

/// Compare every parsed import against the HLE registry.
pub fn analyze_imports(image: &Pe64Image, registry: &HleRegistry) -> ImportAnalysis {
    let mut out = ImportAnalysis::default();
    for descriptor in &image.imports {
        for symbol in &descriptor.symbols {
            let name = import_symbol_name(image, symbol);
            let (state, evidence) = classify_import(registry, &descriptor.dll, &name);
            let entry = AnalyzedImport {
                module: descriptor.dll.clone(),
                symbol: name,
                state,
                evidence,
            };
            match state {
                ImplementationState::Implemented => out.implemented.push(entry),
                ImplementationState::PartiallyImplemented => out.partial.push(entry),
                ImplementationState::Unsupported => out.unsupported.push(entry),
                ImplementationState::Unknown => out.unknown.push(entry),
            }
        }
    }
    out
}

impl CompatibilityReport {
    /// Human-readable summary. Deliberately contains no compatibility
    /// percentage: HLE import coverage is not game compatibility.
    pub fn render_text(&self) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let _ = writeln!(o, "Executable:   {}", self.executable);
        let _ = writeln!(o, "Format:       {}", self.format);
        let _ = writeln!(o, "Architecture: {}", self.architecture);
        match &self.console_generation {
            Some(gen) => {
                let _ = writeln!(o, "Generation:   {gen}");
                for line in &self.generation_evidence {
                    let _ = writeln!(o, "  evidence: {line}");
                }
            }
            None => {
                let _ = writeln!(o, "Generation:   not determined (no reliable evidence)");
            }
        }
        let _ = writeln!(o, "Imports:      {} analysed", self.imports.total());
        let _ = writeln!(
            o,
            "  implemented {}, partially {}, unsupported {}, unknown {}",
            self.imports.implemented.len(),
            self.imports.partial.len(),
            self.imports.unsupported.len(),
            self.imports.unknown.len()
        );
        Self::write_group(&mut o, "Unknown imports (preserved)", &self.imports.unknown);
        Self::write_group(&mut o, "Unsupported imports", &self.imports.unsupported);
        let _ = writeln!(o, "Missing or unresolved dependencies:");
        if self.missing_dependencies.is_empty() {
            let _ = writeln!(o, "  none");
        }
        for dep in &self.missing_dependencies {
            let _ = writeln!(o, "  - {dep}");
        }
        let _ = writeln!(o, "Limitations:");
        for line in &self.limitations {
            let _ = writeln!(o, "  - {line}");
        }
        o
    }

    fn write_group(o: &mut String, title: &str, items: &[AnalyzedImport]) {
        use std::fmt::Write as _;
        let _ = writeln!(o, "{title}:");
        if items.is_empty() {
            let _ = writeln!(o, "  none");
        }
        for item in items {
            let _ = writeln!(o, "  - {}!{}", item.module, item.symbol);
        }
    }
}

/// Analyse an executable file read-only. The file is never executed,
/// modified or passed to the host OS loader.
pub fn analyze_file(
    path: &std::path::Path,
    registry: &HleRegistry,
) -> Result<CompatibilityReport, String> {
    const MAX_BYTES: u64 = 512 * 1024 * 1024;
    let meta =
        std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!(
            "{} exceeds the 512 MiB analysis limit",
            path.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let image = crate::core::loader::pe64::parse(&bytes)
        .map_err(|e| format!("not a supported PE32+ image ({e})"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    Ok(build_report(&name, &image, registry))
}

/// Build a structured report. `executable` should be a file name, not a
/// host path — reports must not leak local directory structures.
pub fn build_report(
    executable: &str,
    image: &Pe64Image,
    registry: &HleRegistry,
) -> CompatibilityReport {
    let imports = analyze_imports(image, registry);
    let mut generation_evidence = Vec::new();
    let mut console_generation = None;
    if image.looks_like_xbox_gdk() {
        generation_evidence.push("PE32+ imports include Xbox GDK runtime DLLs".into());
        console_generation = Some("Xbox Series X|S / Game Core (Gen9)".into());
    }
    let mut missing = Vec::new();
    for import in &imports.unknown {
        let key = format!("{}!{}", import.module, import.symbol);
        if !missing.contains(&key) {
            missing.push(key);
        }
    }
    for import in &imports.unsupported {
        let key = format!("{}!{}", import.module, import.symbol);
        if !missing.contains(&key) && !missing.iter().any(|m| m.starts_with(&import.module)) {
            missing.push(key);
        }
    }
    // Symbols whose name table entry could not be read are reported as
    // unresolved dependencies instead of being silently dropped.
    for import in imports
        .implemented
        .iter()
        .chain(&imports.partial)
        .chain(&imports.unsupported)
        .chain(&imports.unknown)
    {
        if import.symbol.contains("(unresolved)") {
            let key = format!("{}!{}", import.module, import.symbol);
            if !missing.contains(&key) {
                missing.push(key);
            }
        }
    }
    let limitations = vec![
        "No emulator execution was performed for this report".into(),
        "Import coverage is not game compatibility".into(),
        "No retail boot or main-menu result is claimed".into(),
        "Section relocations and imports are parsed, not resolved to host code".into(),
    ];
    CompatibilityReport {
        executable: executable.into(),
        format: "PE32+".into(),
        architecture: if image.machine == crate::core::loader::pe64::IMAGE_FILE_MACHINE_AMD64 {
            "x86-64".into()
        } else {
            format!("machine {:#06x}", image.machine)
        },
        console_generation,
        generation_evidence,
        imports,
        missing_dependencies: missing,
        limitations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loader::pe64;

    #[test]
    fn reports_every_import_without_dropping_unknowns() {
        let image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        let registry = HleRegistry::with_known_entries();
        let analysis = analyze_imports(&image, &registry);
        // Synthetic image imports d3d12_xs.dll by name; its symbol is not
        // in the registry but the module is known → unsupported, not lost.
        assert_eq!(analysis.total(), 1);
        assert_eq!(analysis.unsupported.len(), 1);
        assert_eq!(analysis.unknown.len(), 0);
        assert_eq!(analysis.unsupported[0].module, "d3d12_xs.dll");
    }

    #[test]
    fn unknown_module_imports_are_preserved() {
        let mut image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        image.imports[0].dll = "mystery.dll".into();
        let analysis = analyze_imports(&image, &HleRegistry::default());
        assert_eq!(analysis.unknown.len(), 1);
        assert_eq!(analysis.unknown[0].module, "mystery.dll");
        assert!(!analysis.unknown[0].symbol.is_empty());
    }

    #[test]
    fn report_uses_evidence_based_generation_and_no_percentages() {
        let image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        let report = build_report("game.exe", &image, &HleRegistry::with_known_entries());
        assert_eq!(report.format, "PE32+");
        assert_eq!(report.architecture, "x86-64");
        assert!(report.console_generation.is_some());
        assert!(!report.generation_evidence.is_empty());
        assert!(!report.limitations.is_empty());
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("percent"),
            "compatibility must not report percentages"
        );
    }

    #[test]
    fn non_xbox_image_has_no_generation_claim() {
        let mut image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        image.imports.clear();
        let report = build_report("plain.exe", &image, &HleRegistry::default());
        assert_eq!(report.console_generation, None);
        assert!(report.generation_evidence.is_empty());
    }

    #[test]
    fn xmem_partial_status_flows_into_the_report() {
        // Rewrite the fixture's imported symbol to XMemAlloc (fits the
        // existing hint/name slot) and import it from xmem.dll.
        let mut bytes = pe64::synthetic_pe();
        bytes[0x912..0x91C].copy_from_slice(b"XMemAlloc\0");
        let mut image = pe64::parse(&bytes).unwrap();
        image.imports[0].dll = "xmem.dll".into();
        let analysis = analyze_imports(&image, &HleRegistry::with_known_entries());
        assert_eq!(analysis.partial.len(), 1, "XMemAlloc must be partial");
        assert_eq!(analysis.partial[0].module, "xmem.dll");
        assert_eq!(analysis.partial[0].symbol, "XMemAlloc");
        assert!(
            analysis.partial[0]
                .evidence
                .contains("src/core/hle/xmem.rs"),
            "evidence must point at the implementation: {}",
            analysis.partial[0].evidence
        );
        assert_eq!(
            analysis.implemented.len(),
            0,
            "nothing is fully implemented"
        );
    }

    #[test]
    fn partial_module_does_not_credit_unregistered_symbols() {
        let mut image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        // xmem.dll is partially implemented, but this symbol is not an
        // xmem entry: it must stay Unknown, not inherit module credit.
        image.imports[0].dll = "xmem.dll".into();
        let analysis = analyze_imports(&image, &HleRegistry::with_known_entries());
        assert_eq!(analysis.partial.len(), 0, "no inherited credit");
        assert_eq!(analysis.unknown.len(), 1);
        assert_eq!(analysis.unknown[0].symbol, "D3D12CreateDevice");
    }

    #[test]
    fn unsupported_module_still_classifies_unregistered_symbols() {
        let mut bytes = pe64::synthetic_pe();
        bytes[0x912..0x91A].copy_from_slice(b"XboxFoo\0");
        let image = pe64::parse(&bytes).unwrap();
        let analysis = analyze_imports(&image, &HleRegistry::with_known_entries());
        assert_eq!(analysis.unsupported.len(), 1);
        assert_eq!(analysis.unknown.len(), 0);
        assert_eq!(analysis.unsupported[0].module, "d3d12_xs.dll");
    }

    #[test]
    fn explicit_unknown_research_verdict_survives_an_unsupported_module() {
        // Place a longer hint/name entry at RVA 0x1530 and point the IAT
        // slot at it, so the fixture imports a symbol whose registry entry
        // is explicitly Unknown inside an unsupported module.
        let mut bytes = pe64::synthetic_pe();
        bytes[0x930..0x932].copy_from_slice(&0u16.to_le_bytes());
        bytes[0x932..0x948].copy_from_slice(b"D3D12XboxCreateDevice\0");
        bytes[0xA00..0xA08].copy_from_slice(&0x1530u64.to_le_bytes());
        let image = pe64::parse(&bytes).unwrap();
        let analysis = analyze_imports(&image, &HleRegistry::with_known_entries());
        assert_eq!(analysis.unknown.len(), 1, "research verdict must win");
        assert_eq!(analysis.unsupported.len(), 0);
        assert_eq!(analysis.unknown[0].symbol, "D3D12XboxCreateDevice");
        assert!(!analysis.unknown[0].evidence.is_empty());
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use crate::core::loader::pe64;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("xholy-analyze-{}-{name}", std::process::id()))
    }

    #[test]
    fn unresolved_name_table_entries_are_reported() {
        let mut image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        image.imports[0].symbols[0] = pe64::Import::ByName(0xFFFF_0000);
        let report = build_report("bad.exe", &image, &HleRegistry::with_known_entries());
        assert!(report
            .missing_dependencies
            .iter()
            .any(|d| d.contains("(unresolved)")));
        assert_eq!(report.imports.total(), image.imports[0].symbols.len());
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let err = analyze_file(&temp_path("missing.exe"), &HleRegistry::default()).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn non_pe_input_is_rejected_cleanly() {
        let path = temp_path("text.bin");
        std::fs::write(&path, b"this is not an executable").unwrap();
        let err = analyze_file(&path, &HleRegistry::default()).unwrap_err();
        assert!(err.contains("not a supported PE32+ image"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncated_pe_is_rejected_without_panicking() {
        let bytes = pe64::synthetic_pe();
        let path = temp_path("truncated.exe");
        std::fs::write(&path, &bytes[..bytes.len() / 3]).unwrap();
        assert!(analyze_file(&path, &HleRegistry::default()).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn analysis_is_read_only() {
        let bytes = pe64::synthetic_pe();
        let path = temp_path("ok.exe");
        std::fs::write(&path, &bytes).unwrap();
        let report = analyze_file(&path, &HleRegistry::with_known_entries()).unwrap();
        assert_eq!(
            report.executable,
            format!("xholy-analyze-{}-ok.exe", std::process::id())
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "analysis must not modify the file"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn text_summary_has_no_percentages_and_keeps_unknowns() {
        let mut image = pe64::parse(&pe64::synthetic_pe()).unwrap();
        image.imports[0].dll = "mystery.dll".into();
        let text = build_report("g.exe", &image, &HleRegistry::default()).render_text();
        assert!(text.contains("mystery.dll!"), "{text}");
        assert!(text.contains("not game compatibility"), "{text}");
        assert!(!text.contains('%'), "summary must not contain percentages");
    }
}
