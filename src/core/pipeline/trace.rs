//! HLE Trace Viewer data model, filtering, and import/export — M31 + M34.
//!
//! Pure data and serialization layer built directly on [`ExecutionReport`] and
//! [`DiagnosticEvent`]. Completely independent of any GUI implementation.
//!
//! M34 adds a documented, versioned file format (`.xholy-trace.json`) so a
//! report can be exported, re-imported and inspected in the GUI. Imported
//! data is treated as untrusted: format name and schema version are checked
//! before any value is used.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::hle::runtime::HleRuntime;
use crate::core::hle::ImplementationState;
use crate::core::pipeline::boot::{BootStatus, BootTrace};
use crate::core::pipeline::diagnostics::DiagnosticEvent;
use crate::core::pipeline::{ExecutionReport, GuestSession, StepRecord};

/// Magic format tag of an xHoly trace file.
pub const TRACE_FORMAT: &str = "xholy-trace";
/// Current schema version of the trace file format.
///
/// M35 adds the optional `mnemonic` field to step records. Files written
/// with schema 1 remain importable; new exports use schema 2.
pub const TRACE_SCHEMA_VERSION: u32 = 2;
/// Oldest schema version this build can still import (M34 files).
pub const TRACE_MIN_SCHEMA_VERSION: u32 = 1;
/// Maximum accepted size of an imported trace file (64 MiB).
///
/// Bounds parsing work for untrusted input. A normal synthetic guest trace
/// is a few kilobytes; 64 MiB leaves ample room for long bounded runs.
pub const MAX_TRACE_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Kind of execution stored in a trace file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    /// Plain execution report (`--trace`).
    Execution,
    /// Synthetic guest boot with phases and final verdict (`--guest-test`).
    Boot,
}

/// Boot-specific payload of a versioned trace file.
///
/// The `ExecutionReport` itself is stored once in [`TraceFile::report`]; this
/// struct contains only the boot-specific fields of [`BootTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBootData {
    /// Diagnostic guest name.
    pub guest: String,
    /// SUCCESS / INCOMPLETE / FAILED verdict.
    pub status: BootStatus,
    /// Retired guest instructions.
    pub steps: u64,
    /// Guest `RAX` at exit.
    pub rax: u64,
    /// Whether the guest's heap marker was observed.
    pub marker_verified: bool,
    /// Human-readable stop reason.
    pub stop: String,
    /// Ordered boot phases.
    pub events: Vec<crate::core::pipeline::boot::BootEvent>,
}

/// Versioned, self-contained trace file (`.xholy-trace.json`).
///
/// `format` and `schema_version` are validated on import, so unrelated JSON
/// files can never be mistaken for a trace. No host pointers, secrets or
/// machine-specific data are stored — all addresses are guest VAs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceFile {
    /// Must equal [`TRACE_FORMAT`].
    pub format: String,
    /// Must equal [`TRACE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// What kind of execution the file contains.
    pub kind: TraceKind,
    /// The execution report (always present).
    pub report: ExecutionReport,
    /// Boot phases and verdict, present for [`TraceKind::Boot`].
    pub boot: Option<TraceBootData>,
    /// Per-instruction records of the most recent run.
    pub steps: Vec<StepRecord>,
}

/// Why a trace file could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceImportError {
    /// Input is not valid UTF-8 JSON.
    InvalidJson(String),
    /// File is larger than [`MAX_TRACE_FILE_BYTES`].
    TooLarge(usize),
    /// File does not carry the xHoly trace format tag.
    NotATrace,
    /// File uses a schema version this build cannot read.
    UnsupportedVersion(u32),
    /// A required field is missing, malformed or inconsistent.
    Malformed(String),
}

impl std::fmt::Display for TraceImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(e) => write!(f, "invalid trace JSON: {e}"),
            Self::TooLarge(size) => write!(f, "trace file of {size} bytes exceeds the limit"),
            Self::NotATrace => f.write_str("not an xHoly trace file"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported trace schema version {v}"),
            Self::Malformed(msg) => write!(f, "malformed trace file: {msg}"),
        }
    }
}

impl std::error::Error for TraceImportError {}

impl TraceFile {
    /// Wrap an execution report in a versioned trace file.
    pub fn from_report(report: &ExecutionReport, steps: Vec<StepRecord>) -> Self {
        Self {
            format: TRACE_FORMAT.into(),
            schema_version: TRACE_SCHEMA_VERSION,
            kind: TraceKind::Execution,
            report: report.clone(),
            boot: None,
            steps,
        }
    }

    /// Wrap a boot trace (including its report) in a versioned trace file.
    pub fn from_boot(boot: &BootTrace, steps: Vec<StepRecord>) -> Self {
        Self {
            format: TRACE_FORMAT.into(),
            schema_version: TRACE_SCHEMA_VERSION,
            kind: TraceKind::Boot,
            report: boot.report.clone(),
            boot: Some(TraceBootData {
                guest: boot.guest.clone(),
                status: boot.status,
                steps: boot.steps,
                rax: boot.rax,
                marker_verified: boot.marker_verified,
                stop: boot.stop.clone(),
                events: boot.events.clone(),
            }),
            steps,
        }
    }

    /// Serialise to pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Validate and deserialise a trace file.
    pub fn from_json(text: &str) -> Result<Self, TraceImportError> {
        if text.len() > MAX_TRACE_FILE_BYTES {
            return Err(TraceImportError::TooLarge(text.len()));
        }
        let file: Self =
            serde_json::from_str(text).map_err(|e| TraceImportError::InvalidJson(e.to_string()))?;
        file.validate()?;
        Ok(file)
    }

    /// Validate imported data before any of it reaches the GUI.
    pub fn validate(&self) -> Result<(), TraceImportError> {
        if self.format != TRACE_FORMAT {
            return Err(TraceImportError::NotATrace);
        }
        if !(TRACE_MIN_SCHEMA_VERSION..=TRACE_SCHEMA_VERSION).contains(&self.schema_version) {
            return Err(TraceImportError::UnsupportedVersion(self.schema_version));
        }
        // `steps` must be a stable prefix: step numbers start at 1 and
        // increase by one, no matter where the trace came from.
        for (index, record) in self.steps.iter().enumerate() {
            if record.step != index as u64 + 1 {
                return Err(TraceImportError::Malformed(format!(
                    "step record {} has number {}",
                    index, record.step
                )));
            }
        }
        if self.steps.len() as u64 > self.report.steps {
            return Err(TraceImportError::Malformed(
                "more step records than retired instructions".into(),
            ));
        }
        if let Some(boot) = &self.boot {
            if self.kind != TraceKind::Boot {
                return Err(TraceImportError::Malformed(
                    "boot payload in a non-boot trace".into(),
                ));
            }
            if boot.steps != self.report.steps || boot.stop != self.report.stop.to_string() {
                return Err(TraceImportError::Malformed(
                    "boot result disagrees with the execution report".into(),
                ));
            }
            if boot.rax != self.report.cpu.register(crate::core::cpu::x64::reg::RAX) {
                return Err(TraceImportError::Malformed(
                    "boot RAX disagrees with the CPU snapshot".into(),
                ));
            }
            if !boot.events.iter().any(|event| {
                matches!(
                    event,
                    crate::core::pipeline::boot::BootEvent::FinalResult { .. }
                )
            }) {
                return Err(TraceImportError::Malformed(
                    "boot trace has no final result".into(),
                ));
            }
        } else if self.kind == TraceKind::Boot {
            return Err(TraceImportError::Malformed(
                "boot trace has no boot payload".into(),
            ));
        }
        Ok(())
    }

    /// Reconstruct a [`BootTrace`] if this file contains one.
    pub fn into_boot(self) -> Option<BootTrace> {
        let boot = self.boot?;
        Some(BootTrace {
            guest: boot.guest,
            status: boot.status,
            steps: boot.steps,
            rax: boot.rax,
            marker_verified: boot.marker_verified,
            stop: boot.stop,
            events: boot.events,
            report: self.report,
        })
    }
}

/// Export an execution report to a versioned `.xholy-trace.json` document.
pub fn export_trace_file(
    report: &ExecutionReport,
    steps: Vec<StepRecord>,
) -> Result<String, String> {
    TraceFile::from_report(report, steps)
        .to_json()
        .map_err(|e| format!("cannot serialise trace: {e}"))
}

/// Import a versioned `.xholy-trace.json` document.
pub fn import_trace_file(text: &str) -> Result<TraceFile, TraceImportError> {
    TraceFile::from_json(text)
}

/// Export a synthetic boot trace to a versioned `.xholy-trace.json` document.
pub fn export_boot_trace_file(boot: &BootTrace, steps: Vec<StepRecord>) -> Result<String, String> {
    TraceFile::from_boot(boot, steps)
        .to_json()
        .map_err(|e| format!("cannot serialise boot trace: {e}"))
}

/// Event-type filter for the HLE Trace Viewer timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceEventFilter {
    /// Show all diagnostic events.
    #[default]
    All,
    /// Image load and stack initialization events.
    Load,
    /// Import resolution events.
    Imports,
    /// HLE dispatch and stack-argument events.
    Hle,
    /// CPU and ABI fault events.
    Faults,
    /// Execution stop event.
    Stopped,
}

impl TraceEventFilter {
    /// All filter variants in UI order.
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Load,
        Self::Imports,
        Self::Hle,
        Self::Faults,
        Self::Stopped,
    ];

    /// Short uppercase label for filter chips.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Load => "LOAD",
            Self::Imports => "IMPORTS",
            Self::Hle => "HLE",
            Self::Faults => "FAULTS",
            Self::Stopped => "STOP",
        }
    }

    /// Parse a filter chip label (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "ALL" => Some(Self::All),
            "LOAD" => Some(Self::Load),
            "IMPORTS" => Some(Self::Imports),
            "HLE" => Some(Self::Hle),
            "FAULTS" => Some(Self::Faults),
            "STOP" | "STOPPED" => Some(Self::Stopped),
            _ => None,
        }
    }

    /// Whether `event` matches this filter.
    pub fn matches(self, event: &DiagnosticEvent) -> bool {
        match self {
            Self::All => true,
            Self::Load => matches!(
                event,
                DiagnosticEvent::Loaded { .. } | DiagnosticEvent::StackInitialized { .. }
            ),
            Self::Imports => matches!(event, DiagnosticEvent::ImportResolved { .. }),
            Self::Hle => matches!(
                event,
                DiagnosticEvent::HleDispatch { .. } | DiagnosticEvent::StackArgumentsRead { .. }
            ),
            Self::Faults => matches!(
                event,
                DiagnosticEvent::CpuFault { .. } | DiagnosticEvent::AbiFault { .. }
            ),
            Self::Stopped => matches!(event, DiagnosticEvent::Stopped { .. }),
        }
    }
}

/// Summary counts of resolved imports by [`ImplementationState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportSummary {
    /// Total resolved imports.
    pub total: usize,
    /// Fully implemented imports.
    pub implemented: usize,
    /// Partially implemented imports.
    pub partially_implemented: usize,
    /// Explicitly unsupported imports.
    pub unsupported: usize,
    /// Unknown / unverified imports.
    pub unknown: usize,
}

/// Compute import verdict counts from an [`ExecutionReport`].
pub fn import_summary(report: &ExecutionReport) -> ImportSummary {
    let mut summary = ImportSummary {
        total: report.imports.len(),
        ..Default::default()
    };
    for import in &report.imports {
        match import.state {
            ImplementationState::Implemented => summary.implemented += 1,
            ImplementationState::PartiallyImplemented => summary.partially_implemented += 1,
            ImplementationState::Unsupported => summary.unsupported += 1,
            ImplementationState::Unknown => summary.unknown += 1,
        }
    }
    summary
}

/// Extract `(bottom, top, initial_rsp)` from the trace's `StackInitialized`
/// event, if one was recorded. Returns `None` when unavailable rather than
/// fabricating zero values.
pub fn stack_bounds_from_report(report: &ExecutionReport) -> Option<(u64, u64, u64)> {
    report.events.iter().find_map(|event| match event {
        DiagnosticEvent::StackInitialized {
            bottom,
            top,
            initial_rsp,
            ..
        } => Some((*bottom, *top, *initial_rsp)),
        _ => None,
    })
}

/// Return `(original_index, &DiagnosticEvent)` pairs matching `filter` and an
/// optional case-insensitive substring `query`.
pub fn filter_events<'a>(
    report: &'a ExecutionReport,
    filter: TraceEventFilter,
    query: &str,
) -> Vec<(usize, &'a DiagnosticEvent)> {
    let needle = query.trim().to_ascii_lowercase();
    report
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            if !filter.matches(event) {
                return false;
            }
            if needle.is_empty() {
                return true;
            }
            event.render_line().to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

/// Format multi-line detail lines for a single [`DiagnosticEvent`].
pub fn event_detail_lines(event: &DiagnosticEvent) -> Vec<String> {
    match event {
        DiagnosticEvent::Loaded {
            base,
            size,
            entry_va,
            imports,
            xbox_gdk,
        } => vec![
            "Event: Image Loaded".into(),
            format!("Base VA:    {base:#x}"),
            format!("Image Size: {size:#x} ({size} bytes)"),
            format!("Entry VA:   {entry_va:#x}"),
            format!("Imports:    {imports}"),
            format!("Xbox GDK:   {xbox_gdk}"),
        ],
        DiagnosticEvent::StackInitialized {
            bottom,
            top,
            initial_rsp,
            sentinel,
        } => vec![
            "Event: Stack Initialized".into(),
            format!("Bounds:      {bottom:#x} .. {top:#x}"),
            format!("Initial RSP: {initial_rsp:#x}"),
            format!("Sentinel:    {sentinel:#x}"),
        ],
        DiagnosticEvent::ImportResolved {
            module,
            symbol,
            slot_va,
            state,
        } => vec![
            "Event: Import Resolved".into(),
            format!("Module: {module}"),
            format!("Symbol: {symbol}"),
            format!(
                "IAT Slot: {}",
                slot_va.map_or_else(|| "unavailable (no IAT slot)".into(), |v| format!("{v:#x}"))
            ),
            format!("State:  {}", state.label()),
        ],
        DiagnosticEvent::StackArgumentsRead {
            module,
            symbol,
            rsp,
            count,
            values,
        } => {
            let mut lines = vec![
                "Event: Stack Arguments Read".into(),
                format!("Target: {module}!{symbol}"),
                format!("RSP:    {rsp:#x}"),
                format!("Count:  {count}"),
            ];
            for (idx, val) in values.iter().enumerate() {
                lines.push(format!("Arg #{}: {val:#x} ({val})", idx + 4));
            }
            lines
        }
        DiagnosticEvent::HleDispatch {
            module,
            symbol,
            args,
            status,
            value,
            detail,
        } => {
            let mut lines = vec![
                "Event: HLE Dispatch".into(),
                format!("Target: {module}!{symbol}"),
                format!("Status: {status:?}"),
                format!(
                    "Return: {}",
                    value.map_or_else(
                        || "none (no value returned)".into(),
                        |v| format!("{v:#x} ({v})")
                    )
                ),
            ];
            if args.is_empty() {
                lines.push("Args:   none".into());
            } else {
                for (i, a) in args.iter().enumerate() {
                    lines.push(format!("Arg[{i}]: {a:#x} ({a})"));
                }
            }
            lines.push(format!("Detail: {detail}"));
            lines
        }
        DiagnosticEvent::CpuFault { rip, description } => vec![
            "Event: CPU Fault".into(),
            format!("RIP:         {rip:#x}"),
            format!("Description: {description}"),
        ],
        DiagnosticEvent::AbiFault { rip, description } => vec![
            "Event: ABI Fault".into(),
            format!("RIP:         {rip:#x}"),
            format!("Description: {description}"),
        ],
        DiagnosticEvent::Stopped { reason, steps } => vec![
            "Event: Execution Stopped".into(),
            format!("Reason: {reason}"),
            format!("Steps:  {steps}"),
        ],
    }
}

/// Serialize an [`ExecutionReport`] to pretty JSON.
pub fn export_json(report: &ExecutionReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// Render an [`ExecutionReport`] as human-readable text.
pub fn export_text(report: &ExecutionReport) -> String {
    report.render_text()
}

/// Read a PE32+ executable read-only (never modifying it), run it in a
/// [`GuestSession`] with [`HleRuntime::with_foundation`], and return its
/// [`ExecutionReport`].
pub fn run_pe_file_trace(path: &Path, max_steps: u64) -> Result<ExecutionReport, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("guest.exe");
    let mut session = GuestSession::load(name, &bytes, HleRuntime::with_foundation())
        .map_err(|e| format!("cannot load {}: {e}", path.display()))?;
    Ok(session.run(max_steps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loader::pe64;
    use crate::core::pipeline::{StepKind, StepRecord};

    fn sample_report() -> ExecutionReport {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let mut session =
            GuestSession::load("sample.exe", &bytes, HleRuntime::with_foundation()).unwrap();
        session.run(1_000)
    }

    fn sample_boot() -> BootTrace {
        crate::core::pipeline::boot::run_synthetic_boot(1_000).unwrap()
    }

    #[test]
    fn execution_report_json_round_trips_without_invented_data() {
        let report = sample_report();
        let json = export_trace_file(&report, Vec::new()).expect("export");
        let file = import_trace_file(&json).expect("import");
        assert_eq!(file.format, TRACE_FORMAT);
        assert_eq!(file.schema_version, TRACE_SCHEMA_VERSION);
        assert_eq!(file.kind, TraceKind::Execution);
        assert_eq!(file.report, report);
        assert!(file.boot.is_none());
        assert!(file.steps.is_empty());
    }

    #[test]
    fn boot_trace_round_trip_preserves_phases_and_steps() {
        let boot = sample_boot();
        let steps = boot
            .instructions()
            .map(|(step, rip, kind, mnemonic)| StepRecord {
                step,
                rip,
                kind: kind.clone(),
                mnemonic: mnemonic.map(str::to_string),
            })
            .collect();
        let json = export_boot_trace_file(&boot, steps).expect("export");
        let file = import_trace_file(&json).expect("import");
        assert_eq!(file.kind, TraceKind::Boot);
        let imported = file.clone().into_boot().expect("boot payload");
        assert_eq!(imported, boot);
        assert_eq!(file.steps.len(), 6);
        assert_eq!(file.steps[0].step, 1);
        assert_eq!(file.steps[5].step, 6);
    }

    #[test]
    fn import_rejects_invalid_json_and_unknown_files() {
        assert!(matches!(
            import_trace_file("{not json"),
            Err(TraceImportError::InvalidJson(_))
        ));
        assert!(matches!(
            import_trace_file("{\"hello\":\"world\"}"),
            Err(TraceImportError::InvalidJson(_))
        ));
        let mut value: serde_json::Value =
            serde_json::from_str(&export_trace_file(&sample_report(), Vec::new()).unwrap())
                .unwrap();
        value["format"] = serde_json::json!("something-else");
        assert_eq!(
            import_trace_file(&value.to_string()),
            Err(TraceImportError::NotATrace)
        );
        assert!(matches!(
            import_trace_file(&"x".repeat(MAX_TRACE_FILE_BYTES + 1)),
            Err(TraceImportError::TooLarge(_))
        ));
    }

    #[test]
    fn m34_schema_one_files_still_import() {
        let report = sample_report();
        let mut value: serde_json::Value =
            serde_json::from_str(&export_trace_file(&report, Vec::new()).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(1);
        let file = import_trace_file(&value.to_string()).expect("M34 file imports");
        assert_eq!(file.schema_version, 1);
        assert_eq!(file.report, report);
    }

    #[test]
    fn import_rejects_unsupported_version_and_missing_fields() {
        let report = sample_report();
        let mut value: serde_json::Value =
            serde_json::from_str(&export_trace_file(&report, Vec::new()).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(99);
        let text = value.to_string();
        assert_eq!(
            import_trace_file(&text),
            Err(TraceImportError::UnsupportedVersion(99))
        );

        let mut value: serde_json::Value =
            serde_json::from_str(&export_trace_file(&report, Vec::new()).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("report");
        assert!(matches!(
            import_trace_file(&value.to_string()),
            Err(TraceImportError::InvalidJson(_))
        ));
    }

    #[test]
    fn import_rejects_inconsistent_step_records_and_boot_payload() {
        let report = sample_report();
        let steps = vec![StepRecord {
            step: 7,
            rip: 0,
            kind: StepKind::Executed,
            mnemonic: None,
        }];
        let text = export_trace_file(&report, steps).unwrap();
        assert!(matches!(
            import_trace_file(&text),
            Err(TraceImportError::Malformed(_))
        ));

        let boot = sample_boot();
        let mut file = TraceFile::from_boot(&boot, Vec::new());
        file.boot.as_mut().unwrap().stop = "made up".into();
        assert!(matches!(
            file.validate(),
            Err(TraceImportError::Malformed(_))
        ));

        let mut file = TraceFile::from_report(&report, Vec::new());
        file.kind = TraceKind::Boot;
        assert!(matches!(
            file.validate(),
            Err(TraceImportError::Malformed(_))
        ));
    }

    #[test]
    fn empty_trace_file_round_trips() {
        let mut report = sample_report();
        report.stop = crate::core::pipeline::StopReason::StepsExhausted;
        report.steps = 0;
        let rax = report.cpu.register(crate::core::cpu::x64::reg::RAX);
        let boot = BootTrace {
            guest: "empty.exe".into(),
            status: BootStatus::Failed,
            steps: 0,
            rax,
            marker_verified: false,
            stop: "step budget exhausted".into(),
            events: vec![crate::core::pipeline::boot::BootEvent::FinalResult {
                status: BootStatus::Failed,
                steps: 0,
                rax,
                stop: "step budget exhausted".into(),
            }],
            report,
        };
        let json = export_boot_trace_file(&boot, Vec::new()).unwrap();
        let imported = import_trace_file(&json)
            .unwrap()
            .into_boot()
            .expect("boot payload");
        assert_eq!(imported.events.len(), 1);
        assert_eq!(imported.status, BootStatus::Failed);
    }

    #[test]
    fn bounded_large_trace_file_round_trips() {
        // A bounded synthetic trace with many plausible instruction steps.
        let mut report = sample_report();
        report.steps = 2_000;
        let steps: Vec<_> = (0..2_000)
            .map(|i| StepRecord {
                step: i + 1,
                rip: 0x2000_0000_0000 + i,
                kind: StepKind::Executed,
                mnemonic: Some("nop".into()),
            })
            .collect();
        let json = export_trace_file(&report, steps.clone()).unwrap();
        let file = import_trace_file(&json).expect("import bounded trace");
        assert_eq!(file.steps.len(), 2_000);
        assert_eq!(file.steps[1_999].step, 2_000);
    }

    #[test]
    fn filters_events_by_category_and_query() {
        let report = sample_report();
        assert_eq!(
            filter_events(&report, TraceEventFilter::All, "").len(),
            report.events.len()
        );
        let hle = filter_events(&report, TraceEventFilter::Hle, "");
        assert_eq!(hle.len(), 1);
        assert!(matches!(hle[0].1, DiagnosticEvent::HleDispatch { .. }));

        let by_query = filter_events(&report, TraceEventFilter::All, "XMemAlloc");
        assert!(by_query.len() >= 2);
        assert!(filter_events(&report, TraceEventFilter::Faults, "").is_empty());
    }

    #[test]
    fn extracts_stack_bounds_and_import_summary() {
        let report = sample_report();
        let (bottom, top, initial_rsp) =
            stack_bounds_from_report(&report).expect("stack bounds must be present");
        assert!(bottom < top);
        assert_eq!(initial_rsp, top - 8);

        let summary = import_summary(&report);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.partially_implemented, 1);
        assert_eq!(summary.unsupported, 0);
        assert_eq!(summary.unknown, 0);
    }

    #[test]
    fn json_and_text_export_preserve_report_fields() {
        let report = sample_report();
        let json = export_json(&report).expect("serialize report");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["image"]["name"], "sample.exe");
        assert_eq!(parsed["stop"], "Returned");
        assert!(parsed["events"].is_array());
        assert!(parsed["cpu"]["regs"].is_array());

        let text = export_text(&report);
        assert!(text.contains("Guest execution: sample.exe"));
        assert!(text.contains("[hle] xmem.dll!XMemAlloc"));
    }

    #[test]
    fn handles_empty_execution_report_without_inventing_data() {
        use crate::core::cpu::x64::X64Cpu;
        use crate::core::pipeline::{ImageMeta, StopReason};

        let empty = ExecutionReport {
            image: ImageMeta {
                name: "empty.exe".into(),
                base: 0x2000_0000_0000,
                size: 0x1000,
                entry_va: 0x2000_0000_1000,
                machine: 0x8664,
                xbox_gdk: false,
            },
            imports: Vec::new(),
            stop: StopReason::StepsExhausted,
            steps: 0,
            cpu: X64Cpu::new().snapshot(),
            events: Vec::new(),
        };
        assert_eq!(stack_bounds_from_report(&empty), None);
        assert!(filter_events(&empty, TraceEventFilter::All, "").is_empty());
        assert_eq!(import_summary(&empty).total, 0);
        let json = export_json(&empty).expect("empty report serializes");
        assert!(json.contains("StepsExhausted"));
        let text = export_text(&empty);
        assert!(text.contains("no diagnostic events"));
    }

    #[test]
    fn preserves_event_ordering_and_fault_details() {
        let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12CreateDevice");
        let mut session =
            GuestSession::load("unsp.exe", &bytes, HleRuntime::with_foundation()).unwrap();
        let report = session.run(1_000);
        let summary = import_summary(&report);
        assert_eq!(summary.unsupported, 1);
        assert_eq!(summary.unknown, 0);
        assert!(matches!(
            report.events.first(),
            Some(DiagnosticEvent::Loaded { .. })
        ));
        assert!(matches!(
            report.events.last(),
            Some(DiagnosticEvent::Stopped { .. })
        ));
        let dispatch = report.dispatches().next().expect("one dispatch");
        let lines = event_detail_lines(dispatch);
        assert!(lines.iter().any(|l| l.contains("none (no value returned)")));
    }
}
