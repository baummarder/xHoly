//! Execution diagnostics — Phase 29.
//!
//! An ordered event trace for one guest session: what was loaded, how
//! each import was classified, every HLE dispatch with its arguments
//! and outcome, CPU faults, and the final stop reason. This is the data
//! model a future HLE Trace Viewer will render — this milestone ships
//! the text rendering only, no GUI.

use crate::core::cpu::x64::CpuFault;
use crate::core::hle::runtime::DispatchStatus;
use crate::core::hle::ImplementationState;
use serde::Serialize;

/// One entry in the execution trace.
///
/// `Serialize` only: the trace is written for external consumers (the
/// future trace viewer); xHoly never parses traces back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DiagnosticEvent {
    /// The image was parsed, validated, relocated and mapped.
    Loaded {
        /// Guest VA of the image.
        base: u64,
        /// `SizeOfImage` in bytes.
        size: u64,
        /// Absolute guest VA of the entry point.
        entry_va: u64,
        /// Number of resolved imports.
        imports: usize,
        /// Evidence-based Xbox GDK hint.
        xbox_gdk: bool,
    },
    /// One import was bound to an IAT slot and classified.
    ImportResolved {
        /// Importing module.
        module: String,
        /// Symbol name (or `ordinal 0x....`).
        symbol: String,
        /// Guest VA of its IAT slot, when it has one.
        slot_va: Option<u64>,
        /// Function-level verdict.
        state: ImplementationState,
    },
    /// One HLE dispatch ran (or was refused) with these arguments.
    HleDispatch {
        /// Requested module.
        module: String,
        /// Requested symbol.
        symbol: String,
        /// Arguments read from the x64 argument registers (empty when
        /// the call has no registered implementation).
        args: Vec<u64>,
        /// Outcome class.
        status: DispatchStatus,
        /// Return value, only for `Completed`.
        value: Option<u64>,
        /// Evidence / failure reason from the runtime.
        detail: String,
    },
    /// The CPU raised a structured fault.
    CpuFault {
        /// Instruction pointer of the faulting instruction.
        rip: u64,
        /// Human-readable fault description.
        description: String,
    },
    /// Execution stopped.
    Stopped {
        /// Rendered [`crate::core::pipeline::StopReason`].
        reason: String,
        /// Instructions retired in this run.
        steps: u64,
    },
}

impl DiagnosticEvent {
    /// Single-line rendering for logs and the future trace viewer.
    pub fn render_line(&self) -> String {
        match self {
            Self::Loaded {
                base,
                size,
                entry_va,
                imports,
                xbox_gdk,
            } => format!(
                "[load] base={base:#x} size={size:#x} entry={entry_va:#x} \
                 imports={imports} xbox_gdk={xbox_gdk}"
            ),
            Self::ImportResolved {
                module,
                symbol,
                slot_va,
                state,
            } => {
                let slot = match slot_va {
                    Some(va) => format!("{va:#x}"),
                    None => "none".into(),
                };
                format!(
                    "[import] {module}!{symbol} slot={slot} state={}",
                    state.label()
                )
            }
            Self::HleDispatch {
                module,
                symbol,
                args,
                status,
                value,
                detail,
            } => {
                let args = args
                    .iter()
                    .map(|a| format!("{a:#x}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let value = match value {
                    Some(v) => format!(" value={v:#x}"),
                    None => String::new(),
                };
                format!("[hle] {module}!{symbol}({args}) -> {status:?}{value} ({detail})")
            }
            Self::CpuFault { rip, description } => format!("[cpu-fault] {rip:#x}: {description}"),
            Self::Stopped { reason, steps } => format!("[stop] {reason} after {steps} step(s)"),
        }
    }
}

/// Ordered diagnostics accumulated by one guest session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    events: Vec<DiagnosticEvent>,
}

impl Diagnostics {
    /// An empty trace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one event.
    pub fn push(&mut self, event: DiagnosticEvent) {
        self.events.push(event);
    }

    /// All events in execution order.
    pub fn events(&self) -> &[DiagnosticEvent] {
        &self.events
    }

    /// Render the whole trace, one event per line.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        if self.events.is_empty() {
            out.push_str("no diagnostic events\n");
            return out;
        }
        for event in &self.events {
            out.push_str(&event.render_line());
            out.push('\n');
        }
        out
    }

    /// Every recorded HLE dispatch, in order.
    pub fn dispatches(&self) -> impl Iterator<Item = &DiagnosticEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e, DiagnosticEvent::HleDispatch { .. }))
    }
}

/// Render a CPU fault for the trace (single source: its `Display`).
pub fn describe_fault(fault: &CpuFault) -> String {
    fault.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_render_as_stable_single_lines() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(DiagnosticEvent::Loaded {
            base: 0x2000_0000_0000,
            size: 0x3000,
            entry_va: 0x2000_0000_1000,
            imports: 1,
            xbox_gdk: true,
        });
        diagnostics.push(DiagnosticEvent::ImportResolved {
            module: "xmem.dll".into(),
            symbol: "XMemAlloc".into(),
            slot_va: Some(0x2000_0000_1600),
            state: ImplementationState::PartiallyImplemented,
        });
        diagnostics.push(DiagnosticEvent::HleDispatch {
            module: "xmem.dll".into(),
            symbol: "XMemAlloc".into(),
            args: vec![0x40, 0],
            status: DispatchStatus::Completed,
            value: Some(0x2000_4000_0000),
            detail: "implemented: test".into(),
        });
        diagnostics.push(DiagnosticEvent::Stopped {
            reason: "returned from entry point".into(),
            steps: 5,
        });

        let text = diagnostics.render_text();
        assert!(text.contains("[load] base=0x200000000000"), "{text}");
        assert!(
            text.contains(
                "[import] xmem.dll!XMemAlloc slot=0x200000001600 state=Partially Implemented"
            ),
            "{text}"
        );
        assert!(
            text.contains("[hle] xmem.dll!XMemAlloc(0x40, 0x0) -> Completed value=0x200040000000"),
            "{text}"
        );
        assert!(
            text.contains("[stop] returned from entry point after 5 step(s)"),
            "{text}"
        );
        assert_eq!(diagnostics.events().len(), 4);
        assert_eq!(diagnostics.dispatches().count(), 1);
    }

    #[test]
    fn empty_diagnostics_say_so() {
        assert_eq!(Diagnostics::new().render_text(), "no diagnostic events\n");
    }

    #[test]
    fn cpu_fault_description_matches_the_fault_display() {
        let fault = crate::core::cpu::x64::CpuFault::Memory {
            addr: 0x1000,
            write: true,
            rip: 0x2000,
        };
        let description = describe_fault(&fault);
        assert!(
            description.contains("write failed at 0x1000"),
            "{description}"
        );
        assert!(description.contains("0x2000"), "{description}");
    }
}
