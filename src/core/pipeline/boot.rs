//! First Guest Boot — Milestone 33.
//!
//! Thin orchestration layer around the existing
//! [`GuestSession`](crate::core::pipeline::GuestSession): it loads one
//! self-contained synthetic PE32+ program, runs it with the bounded x86-64
//! interpreter, and records every phase of the boot in an explicit,
//! ordered [`BootTrace`].
//!
//! No new execution engine, no new memory model, no new HLE registry:
//! [`GuestSession::load`] / [`GuestSession::run`] remain the single source
//! of truth. This module only *observes* their results and emits the
//! phases the trace viewer / CLI needs:
//!
//! ```text
//! LoadStarted → LoadCompleted → MemoryInitialized → StackInitialized
//! → EntryPoint → ExecutionStarted → InstructionExecuted* → HleInvocation
//! → HleReturn → GuestExit → FinalResult
//! ```
//!
//! The synthetic guest is generated from source
//! ([`crate::core::loader::pe64::synthetic_boot_guest`]); no proprietary
//! SDK content, no retail binary and no host pointer is involved.

use serde::{Deserialize, Serialize};

use crate::core::cpu::x64::reg;
use crate::core::hle::runtime::{DispatchStatus, HleRuntime};
use crate::core::loader::pe64::{self, SYNTHETIC_BOOT_MARKER, SYNTHETIC_BOOT_STATUS};
use crate::core::memory::GuestMemory;
use crate::core::pipeline::diagnostics::DiagnosticEvent;
use crate::core::pipeline::{GuestSession, LoadError, StepKind, StopReason};

/// Whether a guest boot finished, was cut short, or failed.
///
/// Distinguishes successful, incomplete and failed executions — never a
/// compatibility claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootStatus {
    /// Guest returned with the documented synthetic status and marker.
    Success,
    /// Execution ended before the guest finished (step budget / HLT).
    Incomplete,
    /// Guest faulted, or an import was unsupported / unknown / failed.
    Failed,
}

impl BootStatus {
    /// Stable short label for reports and the GUI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "SUCCESS",
            Self::Incomplete => "INCOMPLETE",
            Self::Failed => "FAILED",
        }
    }
}

/// One phase of a synthetic guest boot (M33 trace events).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootEvent {
    /// Boot started; loading is about to begin.
    LoadStarted {
        /// Diagnostic label of the guest (fixture name).
        name: String,
    },
    /// PE32+ parse, validation, mapping and import resolution succeeded.
    LoadCompleted {
        /// Base guest VA of the mapped image.
        base: u64,
        /// `SizeOfImage` in bytes.
        size: u64,
        /// Absolute guest VA of the entry point.
        entry_va: u64,
        /// Number of resolved imports.
        imports: usize,
    },
    /// Guest address space and heap were prepared.
    MemoryInitialized {
        /// Flat window base (`regions::TITLE`).
        base: u64,
        /// Bytes mapped from the image.
        image_size: u64,
        /// Lowest stack address allowed by the bounds.
        stack_bottom: u64,
        /// Highest stack address allowed by the bounds.
        stack_top: u64,
        /// Base guest VA of the xmem guest heap.
        heap_base: u64,
    },
    /// Guest stack and return sentinel were installed.
    StackInitialized {
        /// Lowest stack address.
        bottom: u64,
        /// Highest stack address.
        top: u64,
        /// `RSP` at entry (callee-entry alignment per Microsoft x64).
        initial_rsp: u64,
        /// Return address pushed for the entry point.
        sentinel: u64,
    },
    /// Entry point was resolved to a guest VA.
    EntryPoint {
        /// Absolute guest VA.
        va: u64,
    },
    /// CPU execution started with a bounded instruction budget.
    ExecutionStarted {
        /// Guest VA of the first instruction.
        entry_va: u64,
        /// Maximum number of retired instructions.
        max_steps: u64,
    },
    /// One guest instruction retired (or faulted).
    InstructionExecuted {
        /// Retired-instruction counter (`0` for a fault).
        step: u64,
        /// Guest VA of the instruction.
        rip: u64,
        /// What the instruction did.
        kind: StepKind,
    },
    /// An HLE import was invoked through its IAT slot.
    HleInvocation {
        /// Importing module.
        module: String,
        /// Imported symbol.
        symbol: String,
        /// Guest VA of the IAT slot.
        slot_va: u64,
        /// Arguments read from the Microsoft x64 argument registers.
        args: Vec<u64>,
    },
    /// The HLE dispatch returned (only for a completed implementation).
    HleReturn {
        /// Importing module.
        module: String,
        /// Imported symbol.
        symbol: String,
        /// Dispatch outcome.
        status: DispatchStatus,
        /// Return value written to `RAX`, if any.
        value: Option<u64>,
    },
    /// Guest execution ended with a structured reason.
    GuestExit {
        /// Rendered [`StopReason`].
        reason: String,
        /// Retired instructions.
        steps: u64,
    },
    /// Final, machine-readable result of the boot.
    FinalResult {
        /// Success / incomplete / failed verdict.
        status: BootStatus,
        /// Retired instructions.
        steps: u64,
        /// Guest `RAX` at exit.
        rax: u64,
        /// Structured stop reason.
        stop: String,
    },
}

impl BootEvent {
    /// Stable single-line rendering for logs and the CLI.
    pub fn render_line(&self) -> String {
        match self {
            Self::LoadStarted { name } => format!("[boot] load started: {name}"),
            Self::LoadCompleted {
                base,
                size,
                entry_va,
                imports,
            } => format!(
                "[boot] load completed: base={base:#x} size={size:#x} entry={entry_va:#x} imports={imports}"
            ),
            Self::MemoryInitialized {
                base,
                image_size,
                stack_bottom,
                stack_top,
                heap_base,
            } => format!(
                "[boot] memory initialized: base={base:#x} image={image_size:#x} stack={stack_bottom:#x}..{stack_top:#x} heap={heap_base:#x}"
            ),
            Self::StackInitialized {
                bottom,
                top,
                initial_rsp,
                sentinel,
            } => format!(
                "[boot] stack initialized: {bottom:#x}..{top:#x} rsp={initial_rsp:#x} sentinel={sentinel:#x}"
            ),
            Self::EntryPoint { va } => format!("[boot] entry point: {va:#x}"),
            Self::ExecutionStarted { entry_va, max_steps } => {
                format!("[boot] cpu started: entry={entry_va:#x} max_steps={max_steps}")
            }
            Self::InstructionExecuted { step, rip, kind } => {
                let what = match kind {
                    StepKind::Executed => "executed".into(),
                    StepKind::Halted => "halted".into(),
                    StepKind::HleCall { slot_addr } => format!("hle call via {slot_addr:#x}"),
                    StepKind::Fault { description } => format!("fault: {description}"),
                };
                format!("[boot] step {step}: rip={rip:#x} {what}")
            }
            Self::HleInvocation {
                module,
                symbol,
                slot_va,
                args,
            } => {
                let args = args
                    .iter()
                    .map(|a| format!("{a:#x}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[boot] hle invoked: {module}!{symbol} slot={slot_va:#x} args=({args})")
            }
            Self::HleReturn {
                module,
                symbol,
                status,
                value,
            } => {
                let value = value.map_or_else(
                    || "none".into(),
                    |v| format!("{v:#x}"),
                );
                format!("[boot] hle return: {module}!{symbol} {status:?} value={value}")
            }
            Self::GuestExit { reason, steps } => {
                format!("[boot] guest exit: {reason} after {steps} step(s)")
            }
            Self::FinalResult {
                status,
                steps,
                rax,
                stop,
            } => format!(
                "[boot] final result: {} steps={steps} rax={rax:#x} stop={stop}",
                status.label()
            ),
        }
    }
}

/// Full ordered trace of one synthetic guest boot (Serialize only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootTrace {
    /// Diagnostic guest name.
    pub guest: String,
    /// Success / incomplete / failed verdict.
    pub status: BootStatus,
    /// Retired guest instructions.
    pub steps: u64,
    /// Guest `RAX` at exit.
    pub rax: u64,
    /// Whether the guest's heap marker write was observed in guest memory.
    pub marker_verified: bool,
    /// Human-readable stop reason.
    pub stop: String,
    /// Every boot phase, in order.
    pub events: Vec<BootEvent>,
    /// Underlying guest execution report (feeds the existing trace viewer).
    pub report: crate::core::pipeline::ExecutionReport,
}

impl BootTrace {
    /// Render every phase as one line.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for event in &self.events {
            out.push_str(&event.render_line());
            out.push('\n');
        }
        out
    }

    /// Serialize to pretty JSON (CLI `--guest-test --json`).
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// All instruction phases in order (shortcut for tests and tooling).
    pub fn instructions(&self) -> impl Iterator<Item = (u64, u64, &StepKind)> {
        self.events.iter().filter_map(|event| match event {
            BootEvent::InstructionExecuted { step, rip, kind } => Some((*step, *rip, kind)),
            _ => None,
        })
    }
}

/// Run a synthetic guest boot and produce its trace.
///
/// `bytes` must be a valid PE32+ image; validation failures are returned as
/// [`LoadError`] without executing anything. The guest always uses the
/// foundation HLE runtime (`XMemAlloc` / `XMemFree` shims only).
pub fn boot_guest(name: &str, bytes: &[u8], max_steps: u64) -> Result<BootTrace, LoadError> {
    let mut events = Vec::new();
    events.push(BootEvent::LoadStarted {
        name: name.to_string(),
    });

    let mut session = GuestSession::load(name, bytes, HleRuntime::with_foundation())?;
    let image = session.image().clone();
    events.push(BootEvent::LoadCompleted {
        base: image.base,
        size: image.size,
        entry_va: image.entry_va,
        imports: session.imports().len(),
    });

    let (stack_bottom, stack_top, initial_rsp) = session
        .cpu()
        .stack_bounds()
        .map(|bounds| (bounds.bottom, bounds.top, session.cpu().regs[reg::RSP]))
        .unwrap_or((0, 0, session.cpu().regs[reg::RSP]));
    events.push(BootEvent::MemoryInitialized {
        base: image.base,
        image_size: image.size,
        stack_bottom,
        stack_top,
        heap_base: crate::core::hle::xmem::XMEM_HEAP_BASE,
    });
    events.push(BootEvent::StackInitialized {
        bottom: stack_bottom,
        top: stack_top,
        initial_rsp,
        sentinel: crate::core::pipeline::ENTRY_RETURN_SENTINEL,
    });
    events.push(BootEvent::EntryPoint { va: image.entry_va });
    events.push(BootEvent::ExecutionStarted {
        entry_va: image.entry_va,
        max_steps,
    });

    let report = session.run(max_steps);

    for record in session.step_records() {
        events.push(BootEvent::InstructionExecuted {
            step: record.step,
            rip: record.rip,
            kind: record.kind.clone(),
        });
    }
    for event in &report.events {
        if let DiagnosticEvent::HleDispatch {
            module,
            symbol,
            args,
            status,
            value,
            ..
        } = event
        {
            let slot_va = session
                .imports()
                .iter()
                .find(|import| import.module == *module && import.symbol == *symbol)
                .and_then(|import| import.slot_va)
                .unwrap_or(0);
            events.push(BootEvent::HleInvocation {
                module: module.clone(),
                symbol: symbol.clone(),
                slot_va,
                args: args.clone(),
            });
            events.push(BootEvent::HleReturn {
                module: module.clone(),
                symbol: symbol.clone(),
                status: *status,
                value: *value,
            });
        }
    }

    events.push(BootEvent::GuestExit {
        reason: report.stop.to_string(),
        steps: report.steps,
    });

    let rax = report.cpu.register(reg::RAX);
    let completed_pointer = report.dispatches().find_map(|event| match event {
        DiagnosticEvent::HleDispatch {
            status: DispatchStatus::Completed,
            value,
            ..
        } => *value,
        _ => None,
    });
    let marker_verified = completed_pointer.and_then(|addr| session.space().read_u64(addr))
        == Some(SYNTHETIC_BOOT_MARKER);
    let status = if report.stop == StopReason::Returned
        && rax == SYNTHETIC_BOOT_STATUS
        && completed_pointer.is_some()
        && marker_verified
    {
        BootStatus::Success
    } else if matches!(report.stop, StopReason::StepsExhausted | StopReason::Halted) {
        BootStatus::Incomplete
    } else {
        BootStatus::Failed
    };
    events.push(BootEvent::FinalResult {
        status,
        steps: report.steps,
        rax,
        stop: report.stop.to_string(),
    });

    Ok(BootTrace {
        guest: name.to_string(),
        status,
        steps: report.steps,
        rax,
        marker_verified,
        stop: report.stop.to_string(),
        events,
        report,
    })
}

/// Run the built-in synthetic M33 guest (no file system access).
pub fn run_synthetic_boot(max_steps: u64) -> Result<BootTrace, LoadError> {
    boot_guest(
        "synthetic-boot.exe",
        &pe64::synthetic_boot_guest(),
        max_steps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_boot_ends_with_the_documented_status() {
        let trace = run_synthetic_boot(1_000).expect("synthetic guest must boot");
        assert_eq!(trace.status, BootStatus::Success);
        assert_eq!(trace.rax, SYNTHETIC_BOOT_STATUS);
        assert_eq!(trace.report.stop, StopReason::Returned);
    }

    #[test]
    fn trace_contains_every_required_boot_phase_in_order() {
        let trace = run_synthetic_boot(1_000).expect("synthetic guest must boot");
        let names: Vec<&str> = trace
            .events
            .iter()
            .map(|event| match event {
                BootEvent::LoadStarted { .. } => "LoadStarted",
                BootEvent::LoadCompleted { .. } => "LoadCompleted",
                BootEvent::MemoryInitialized { .. } => "MemoryInitialized",
                BootEvent::StackInitialized { .. } => "StackInitialized",
                BootEvent::EntryPoint { .. } => "EntryPoint",
                BootEvent::ExecutionStarted { .. } => "ExecutionStarted",
                BootEvent::InstructionExecuted { .. } => "InstructionExecuted",
                BootEvent::HleInvocation { .. } => "HleInvocation",
                BootEvent::HleReturn { .. } => "HleReturn",
                BootEvent::GuestExit { .. } => "GuestExit",
                BootEvent::FinalResult { .. } => "FinalResult",
            })
            .collect();
        assert_eq!(names.first(), Some(&"LoadStarted"));
        assert_eq!(names.last(), Some(&"FinalResult"));
        for phase in [
            "LoadStarted",
            "LoadCompleted",
            "MemoryInitialized",
            "StackInitialized",
            "EntryPoint",
            "ExecutionStarted",
            "InstructionExecuted",
            "HleInvocation",
            "HleReturn",
            "GuestExit",
            "FinalResult",
        ] {
            assert!(names.contains(&phase), "missing phase {phase}");
        }
    }

    #[test]
    fn malformed_guest_is_rejected_without_a_boot_trace() {
        assert!(boot_guest("bad.exe", b"NOPE", 100).is_err());
    }
}
