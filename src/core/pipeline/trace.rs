//! HLE Trace Viewer data model, filtering, and export helpers — Milestone 31.
//!
//! Pure data and serialization layer built directly on [`ExecutionReport`] and
//! [`DiagnosticEvent`]. Completely independent of any GUI implementation.

use std::path::Path;

use crate::core::hle::runtime::HleRuntime;
use crate::core::hle::ImplementationState;
use crate::core::pipeline::diagnostics::DiagnosticEvent;
use crate::core::pipeline::{ExecutionReport, GuestSession};

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

    fn sample_report() -> ExecutionReport {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let mut session =
            GuestSession::load("sample.exe", &bytes, HleRuntime::with_foundation()).unwrap();
        session.run(1_000)
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
