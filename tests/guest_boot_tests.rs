//! Milestone 33 — First Guest Boot integration tests.
//!
//! One self-contained synthetic PE32+ guest is loaded through the real
//! pipeline, executes supported x86-64 code, invokes `xmem.dll!XMemAlloc`
//! through its IAT slot, writes a marker into the guest heap, returns a
//! documented status and is recorded in an ordered boot trace.
//!
//! All fixtures are generated from source. No proprietary SDK content,
//! no retail binary, no host pointer.

use std::process::Command;

use xholy::core::cpu::x64::reg;
use xholy::core::hle::runtime::DispatchStatus;
use xholy::core::hle::xmem::{DEFAULT_ALIGNMENT, XMEM_HEAP_BASE};
use xholy::core::loader::pe64::{self, Pe64Error, SYNTHETIC_BOOT_CODE, SYNTHETIC_BOOT_STATUS};
use xholy::core::memory::regions;
use xholy::core::pipeline::boot::{boot_guest, run_synthetic_boot, BootEvent, BootStatus};
use xholy::core::pipeline::diagnostics::DiagnosticEvent;
use xholy::core::pipeline::{GuestSession, LoadError, StepKind, StopReason};

/// Patch entry code at RVA `0x1000` / file offset `0x400`.
fn with_entry_code(bytes: &mut [u8], code: &[u8]) {
    bytes[0x400..0x400 + code.len()].copy_from_slice(code);
}

/// 1. The fixture is a valid, fully synthetic PE32+ image.
#[test]
fn synthetic_boot_guest_is_a_valid_self_contained_pe32plus_image() {
    let bytes = pe64::synthetic_boot_guest();
    let image = pe64::parse(&bytes).expect("parse synthetic guest");
    pe64::validate(&image).expect("validate synthetic guest");
    assert_eq!(image.machine, 0x8664);
    assert_eq!(image.entry_rva, 0x1000);
    assert_eq!(image.size_of_image, 0x3000);
    assert_eq!(
        &bytes[0x400..0x400 + SYNTHETIC_BOOT_CODE.len()],
        SYNTHETIC_BOOT_CODE
    );
    assert_eq!(image.imports.len(), 1);
    assert_eq!(image.imports[0].dll, "xmem.dll");
    assert!(
        image.relocations.is_empty(),
        "call fixture drops the DIR64 site"
    );
    assert!(image.looks_like_xbox_gdk());
}

/// 2. The guest loads successfully and reports honest metadata.
#[test]
fn guest_load_succeeds_and_reports_metadata() {
    let session = GuestSession::load(
        "synthetic-boot.exe",
        &pe64::synthetic_boot_guest(),
        xholy::core::hle::runtime::HleRuntime::with_foundation(),
    )
    .expect("load synthetic guest");
    let image = session.image();
    assert_eq!(image.base, regions::TITLE);
    assert_eq!(image.entry_va, regions::TITLE + 0x1000);
    assert_eq!(image.machine, 0x8664);
    assert!(image.xbox_gdk);
    assert_eq!(session.imports().len(), 1);
}

/// 3. The guest stack is initialized inside configured bounds.
#[test]
fn guest_stack_is_initialized_with_microsoft_x64_alignment() {
    let session = GuestSession::load(
        "synthetic-boot.exe",
        &pe64::synthetic_boot_guest(),
        xholy::core::hle::runtime::HleRuntime::with_foundation(),
    )
    .expect("load synthetic guest");
    let bounds = session.cpu().stack_bounds().expect("stack bounds");
    assert_eq!(bounds.top, XMEM_HEAP_BASE);
    assert_eq!(
        bounds.bottom,
        XMEM_HEAP_BASE - xholy::core::pipeline::STACK_SIZE
    );
    // Sentinel was pushed: callee-entry RSP % 16 == 8.
    assert_eq!(session.cpu().regs[reg::RSP], XMEM_HEAP_BASE - 8);
    assert_eq!(session.cpu().regs[reg::RSP] % 16, 8);
}

/// 4. Supported instructions execute and the guest returns with its status.
#[test]
fn guest_executes_supported_instructions_and_returns_a_documented_status() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    assert_eq!(trace.status, BootStatus::Success);
    assert_eq!(trace.stop, "returned from entry point");
    assert_eq!(trace.report.stop, StopReason::Returned);
    assert_eq!(trace.rax, SYNTHETIC_BOOT_STATUS);
    assert_eq!(trace.steps, 6, "mov, xor, call, mov, mov, ret");
    assert_eq!(trace.report.cpu.register(reg::RSP), XMEM_HEAP_BASE);
    let kinds: Vec<&StepKind> = trace.instructions().map(|(_, _, kind, _)| kind).collect();
    assert_eq!(kinds.len(), 6);
    assert!(kinds.iter().any(|k| matches!(k, StepKind::HleCall { .. })));
    assert!(matches!(kinds.last(), Some(StepKind::Executed)));
}

/// 5. Exactly one controlled HLE call is dispatched.
#[test]
fn boot_performs_one_controlled_hle_call() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    let dispatches: Vec<_> = trace.report.dispatches().collect();
    assert_eq!(dispatches.len(), 1);
    let invocations: Vec<_> = trace
        .events
        .iter()
        .filter(|e| matches!(e, BootEvent::HleInvocation { .. }))
        .collect();
    let returns: Vec<_> = trace
        .events
        .iter()
        .filter(|e| matches!(e, BootEvent::HleReturn { .. }))
        .collect();
    assert_eq!(invocations.len(), 1);
    assert_eq!(returns.len(), 1);
    match &dispatches[0] {
        DiagnosticEvent::HleDispatch {
            module,
            symbol,
            status,
            ..
        } => {
            assert_eq!(module, "xmem.dll");
            assert_eq!(symbol, "XMemAlloc");
            assert_eq!(*status, DispatchStatus::Completed);
        }
        other => panic!("expected HLE dispatch, got {other:?}"),
    }
}

/// 6. HLE arguments come from the documented Microsoft x64 registers.
#[test]
fn hle_arguments_match_the_microsoft_x64_register_convention() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    let invocation = trace
        .events
        .iter()
        .find_map(|e| match e {
            BootEvent::HleInvocation { args, .. } => Some(args.clone()),
            _ => None,
        })
        .expect("HLE invocation phase");
    assert_eq!(invocation, vec![64, 0], "RCX=size, RDX=attributes");
    assert_eq!(trace.report.cpu.register(reg::RCX), 64);
    assert_eq!(trace.report.cpu.register(reg::RDX), 0);
}

/// 7. The HLE return value is a guest address and the guest wrote through it.
#[test]
fn hle_return_value_is_a_guest_pointer_and_the_marker_is_verified() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    assert!(trace.marker_verified, "guest heap marker must be observed");
    let pointer = trace
        .events
        .iter()
        .find_map(|e| match e {
            BootEvent::HleReturn {
                value: Some(value), ..
            } => Some(*value),
            _ => None,
        })
        .expect("HLE return phase with a pointer");
    assert!(
        pointer >= XMEM_HEAP_BASE,
        "guest heap address, never a host pointer"
    );
    assert_eq!(pointer % DEFAULT_ALIGNMENT, 0);
    assert_eq!(
        trace.report.cpu.register(reg::RAX),
        SYNTHETIC_BOOT_STATUS,
        "guest replaced the pointer with its documented status"
    );
}

/// 8. The trace records every required boot phase in order.
#[test]
fn boot_trace_records_all_required_phases() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    let phases: Vec<&str> = trace
        .events
        .iter()
        .map(|e| match e {
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
    let expected = [
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
    ];
    for phase in expected {
        assert!(phases.contains(&phase), "missing boot phase {phase}");
    }
    let first_index = |name: &str| {
        phases
            .iter()
            .position(|p| *p == name)
            .expect("phase present")
    };
    assert!(first_index("LoadStarted") < first_index("LoadCompleted"));
    assert!(first_index("LoadCompleted") < first_index("MemoryInitialized"));
    assert!(first_index("ExecutionStarted") < first_index("InstructionExecuted"));
    assert!(first_index("InstructionExecuted") < first_index("HleInvocation"));
    assert!(first_index("HleInvocation") < first_index("HleReturn"));
    assert!(first_index("HleReturn") < first_index("GuestExit"));
    assert!(first_index("GuestExit") < first_index("FinalResult"));
    assert!(trace.render_text().contains("[boot] final result: SUCCESS"));
}

/// 9. Guest exit and final result are explicit and distinguish outcomes.
#[test]
fn guest_exit_and_final_result_are_explicit() {
    let trace = run_synthetic_boot(1_000).expect("boot synthetic guest");
    assert!(trace.events.iter().any(|e| matches!(
        e,
        BootEvent::GuestExit { reason, steps }
            if reason == "returned from entry point" && *steps == trace.steps
    )));
    assert!(trace.events.iter().any(|e| matches!(
        e,
        BootEvent::FinalResult { status: BootStatus::Success, rax, .. }
            if *rax == SYNTHETIC_BOOT_STATUS
    )));
    assert!(trace.render_text().contains("[boot] guest exit:"));
}

/// 10. Invalid or unsupported guest programs are rejected diagnosably.
#[test]
fn invalid_guest_program_is_rejected_without_execution() {
    let err = boot_guest("bad.exe", b"NOPE", 100).expect_err("not a PE");
    assert!(matches!(err, LoadError::Parse(Pe64Error::NotPe)), "{err}");

    let mut bytes = pe64::synthetic_boot_guest();
    bytes[0xA8..0xAC].copy_from_slice(&0x3000u32.to_le_bytes()); // entry outside image
    let err = boot_guest("bad.exe", &bytes, 100).expect_err("invalid image");
    assert!(matches!(err, LoadError::InvalidImage(_)), "{err}");

    // Unsupported import is diagnosed, never simulated with a fake value.
    let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12CreateDevice");
    let trace = boot_guest("unsupported.exe", &bytes, 100).expect("load unsupported guest");
    assert_eq!(trace.status, BootStatus::Failed);
    assert_eq!(
        trace.report.stop,
        StopReason::UnsupportedImport {
            module: "d3d12_xs.dll".into(),
            symbol: "D3D12CreateDevice".into(),
        }
    );
    assert_eq!(
        trace.report.cpu.register(reg::RAX),
        0,
        "no fake return value"
    );
}

/// 11. The instruction budget bounds execution and marks it incomplete.
#[test]
fn instruction_budget_bounds_execution_and_marks_it_incomplete() {
    let trace = run_synthetic_boot(2).expect("boot synthetic guest");
    assert_eq!(trace.status, BootStatus::Incomplete);
    assert_eq!(trace.report.stop, StopReason::StepsExhausted);
    assert_eq!(trace.steps, 2);
    assert_eq!(trace.instructions().count(), 2);
    assert!(trace.render_text().contains("INCOMPLETE"));
}

/// 12. Faulting memory accesses are diagnosed and fail the boot.
#[test]
fn faulting_memory_access_is_diagnosed_and_fails_the_boot() {
    let mut bytes = pe64::synthetic_boot_guest();
    // Replace `mov [rax],0x11` with `mov qword [0],0x11`: RAX may point
    // anywhere, the write address 0 is unmapped in either guest region.
    with_entry_code(
        &mut bytes,
        &[
            0xB9, 0x40, 0x00, 0x00, 0x00, // mov ecx, 64
            0x31, 0xD2, //                   xor edx, edx
            0xFF, 0x15, 0xF3, 0x05, 0x00, 0x00, // call [rip+0x5f3]
            0x48, 0xC7, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov qword [0],0
            0x11, 0x00, 0x00, 0x00, //       imm32 0x11
            0xC3, //                         ret (never reached)
        ],
    );
    let trace = boot_guest("faulting.exe", &bytes, 100).expect("load faulting guest");
    assert_eq!(trace.status, BootStatus::Failed);
    assert!(matches!(trace.report.stop, StopReason::Fault(_)));
    assert!(trace.events.iter().any(|e| matches!(
        e,
        BootEvent::InstructionExecuted {
            kind: StepKind::Fault { .. },
            ..
        }
    )));
    assert!(!trace.marker_verified);
    assert!(trace.render_text().contains("[boot] final result: FAILED"));
}

/// CLI entry point prints an ordered boot trace without touching any file.
#[test]
fn cli_guest_test_prints_a_trace_and_json() {
    let exe = env!("CARGO_BIN_EXE_xholy");
    let output = Command::new(exe)
        .arg("--guest-test")
        .output()
        .expect("run xholy --guest-test");
    assert!(output.status.success(), "{:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).expect("utf8 trace");
    assert!(stdout.contains("[boot] load started"));
    assert!(stdout.contains("[boot] hle invoked: xmem.dll!XMemAlloc"));
    assert!(stdout.contains("[boot] final result: SUCCESS"));

    let json = Command::new(exe)
        .args(["--guest-test", "--json"])
        .output()
        .expect("run xholy --guest-test --json");
    assert!(json.status.success(), "{:?}", json.stderr);
    let value: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("valid boot trace JSON");
    assert_eq!(value["status"], "Success");
    assert_eq!(value["rax"], SYNTHETIC_BOOT_STATUS);
    assert!(value["events"].is_array());
}
