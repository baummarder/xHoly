//! Integration tests for the Phase-29 Guest Execution Pipeline:
//! PE32+ loader -> import resolution -> guest memory -> X64Cpu -> HLE dispatch.
//!
//! All tests use synthetic PE32+ fixtures generated from source.
//! No proprietary SDKs or retail game dumps are used.

use xholy::core::cpu::x64::{reg, CpuFault};
use xholy::core::hle::runtime::{DispatchStatus, HleRuntime};
use xholy::core::hle::xmem::XMEM_HEAP_BASE;
use xholy::core::hle::ImplementationState;
use xholy::core::loader::pe64;
use xholy::core::memory::{regions, GuestMemory};
use xholy::core::pipeline::diagnostics::DiagnosticEvent;
use xholy::core::pipeline::{GuestSession, LoadError, StopReason};

fn with_entry_code(mut bytes: Vec<u8>, code: &[u8]) -> Vec<u8> {
    bytes[0x400..0x400 + code.len()].copy_from_slice(code);
    bytes
}

#[test]
fn loads_synthetic_pe_and_returns_through_sentinel() {
    let bytes = pe64::synthetic_pe();
    let mut session = GuestSession::load("synthetic.exe", &bytes, HleRuntime::with_foundation())
        .expect("synthetic PE should load");

    assert_eq!(session.image().base, regions::TITLE);
    assert_eq!(session.image().entry_va, regions::TITLE + 0x1000);
    assert_eq!(session.image().machine, pe64::IMAGE_FILE_MACHINE_AMD64);
    assert!(session.image().xbox_gdk);

    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.steps, 2);
    assert_eq!(report.cpu.register(reg::RAX), 42);
    assert_eq!(session.cpu().regs[reg::RSP], XMEM_HEAP_BASE);
}

#[test]
fn rejects_malformed_pe_inputs_with_structured_errors() {
    let err = GuestSession::load("bad.exe", b"NOT_PE", HleRuntime::with_foundation())
        .expect_err("non-PE bytes must fail");
    assert!(matches!(err, LoadError::Parse(pe64::Pe64Error::NotPe)));

    let mut bad_machine = pe64::synthetic_pe();
    bad_machine[0x84..0x86].copy_from_slice(&0xAA64u16.to_le_bytes());
    let err = GuestSession::load("arm64.exe", &bad_machine, HleRuntime::with_foundation())
        .expect_err("non-AMD64 machine must fail");
    assert!(matches!(
        err,
        LoadError::Parse(pe64::Pe64Error::Unsupported("machine is not x86-64"))
    ));

    let mut no_entry = pe64::synthetic_pe();
    no_entry[0xA8..0xAC].copy_from_slice(&0u32.to_le_bytes());
    let err = GuestSession::load("no_entry.exe", &no_entry, HleRuntime::with_foundation())
        .expect_err("zero entry RVA must fail");
    assert!(matches!(err, LoadError::NoEntryPoint));
}

#[test]
fn dispatches_xmem_alloc_and_writes_to_guest_heap() {
    let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
    let mut session = GuestSession::load("xmem_call.exe", &bytes, HleRuntime::with_foundation())
        .expect("call fixture should load");

    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.steps, 5);

    let ptr = report.cpu.register(reg::RAX);
    assert!(ptr >= XMEM_HEAP_BASE);
    assert_eq!(ptr % 16, 0);
    assert_eq!(session.space().read_u64(ptr), Some(0x11));

    let dispatches: Vec<_> = report.dispatches().collect();
    assert_eq!(dispatches.len(), 1);
    assert!(matches!(
        dispatches[0],
        DiagnosticEvent::HleDispatch {
            status: DispatchStatus::Completed,
            value: Some(v),
            ..
        } if *v == ptr
    ));
}

#[test]
fn distinguishes_unsupported_unknown_and_failed_imports() {
    // Known unsupported module/symbol.
    let unsupported_bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12CreateDevice");
    let mut session = GuestSession::load(
        "unsupported.exe",
        &unsupported_bytes,
        HleRuntime::with_foundation(),
    )
    .unwrap();
    assert_eq!(session.imports()[0].state, ImplementationState::Unsupported);
    let report = session.run(1_000);
    assert_eq!(
        report.stop,
        StopReason::UnsupportedImport {
            module: "d3d12_xs.dll".into(),
            symbol: "D3D12CreateDevice".into(),
        }
    );
    assert_eq!(report.cpu.register(reg::RAX), 0);

    // Unknown module/symbol.
    let unknown_bytes = pe64::synthetic_pe_calling("unknown_mod.dll", "UnknownFn");
    let mut session =
        GuestSession::load("unknown.exe", &unknown_bytes, HleRuntime::with_foundation()).unwrap();
    assert_eq!(session.imports()[0].state, ImplementationState::Unknown);
    let report = session.run(1_000);
    assert_eq!(
        report.stop,
        StopReason::UnknownImport {
            module: "unknown_mod.dll".into(),
            symbol: "UnknownFn".into(),
        }
    );
    assert_eq!(report.cpu.register(reg::RAX), 0);

    // Runtime failure propagation (XMemFree on unallocated address 64).
    let fail_bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemFree");
    let mut session =
        GuestSession::load("fail.exe", &fail_bytes, HleRuntime::with_foundation()).unwrap();
    let report = session.run(1_000);
    assert!(matches!(
        report.stop,
        StopReason::HleFailure {
            ref module,
            ref symbol,
            ..
        } if module == "xmem.dll" && symbol == "XMemFree"
    ));
}

#[test]
fn reports_cpu_and_memory_faults_without_retiring_faulting_instruction() {
    // Illegal instruction: LEA RAX, [RAX] (0x48 0x8D 0x00)
    let illegal = with_entry_code(pe64::synthetic_pe(), &[0x48, 0x8D, 0x00]);
    let mut session =
        GuestSession::load("illegal.exe", &illegal, HleRuntime::with_foundation()).unwrap();
    let report = session.run(100);
    assert!(matches!(
        report.stop,
        StopReason::Fault(CpuFault::IllegalInstruction { opcode: 0x8D, .. })
    ));
    assert_eq!(report.steps, 0);
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, DiagnosticEvent::CpuFault { .. })));

    // Unmapped memory write: mov qword ptr [rax], 1 with rax = 0
    let mem_fault = with_entry_code(
        pe64::synthetic_pe(),
        &[0x48, 0xC7, 0x00, 0x01, 0x00, 0x00, 0x00],
    );
    let mut session =
        GuestSession::load("memfault.exe", &mem_fault, HleRuntime::with_foundation()).unwrap();
    let report = session.run(100);
    assert_eq!(
        report.stop,
        StopReason::Fault(CpuFault::Memory {
            addr: 0,
            write: true,
            rip: regions::TITLE + 0x1000,
        })
    );
    assert_eq!(report.steps, 0);
}
