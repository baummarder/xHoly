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

#[test]
fn stack_initialization_and_bounds_are_enforced_and_diagnosed() {
    let bytes = pe64::synthetic_pe();
    let session =
        GuestSession::load("stack_init.exe", &bytes, HleRuntime::with_foundation()).unwrap();

    let bounds = session
        .cpu()
        .stack_bounds()
        .expect("stack bounds configured");
    assert_eq!(bounds.top, XMEM_HEAP_BASE);
    assert_eq!(
        bounds.bottom,
        XMEM_HEAP_BASE - xholy::core::pipeline::STACK_SIZE
    );
    assert_eq!(session.cpu().regs[reg::RSP], XMEM_HEAP_BASE - 8);
    assert!(session.diagnostics().events().iter().any(|e| matches!(
        e,
        DiagnosticEvent::StackInitialized { bottom, top, initial_rsp, .. }
            if *bottom == bounds.bottom && *top == bounds.top && *initial_rsp == XMEM_HEAP_BASE - 8
    )));

    // Stack underflow: RET at entry pops sentinel, then another RET reads >= stack_top.
    let underflow_bytes = with_entry_code(pe64::synthetic_pe(), &[0x58, 0xC3]); // pop rax; ret
    let mut underflow_session = GuestSession::load(
        "underflow.exe",
        &underflow_bytes,
        HleRuntime::with_foundation(),
    )
    .unwrap();
    let report = underflow_session.run(100);
    assert!(matches!(
        report.stop,
        StopReason::Fault(CpuFault::StackUnderflow { .. })
    ));

    // Stack overflow: Entry RSP = XMEM_HEAP_BASE - 8. Subtracting (STACK_SIZE - 8) puts
    // RSP == stack_bottom, so `push rax` faults with StackOverflow.
    let mut code = Vec::new();
    // 48 81 EC <imm32> : sub rsp, (STACK_SIZE - 8)
    code.extend_from_slice(&[0x48, 0x81, 0xEC]);
    code.extend_from_slice(&((xholy::core::pipeline::STACK_SIZE - 8) as u32).to_le_bytes());
    code.push(0x50); // push rax -> new_rsp = stack_bottom - 8 < bottom
    code.push(0xC3); // ret
    let overflow_bytes = with_entry_code(pe64::synthetic_pe(), &code);
    let mut overflow_session = GuestSession::load(
        "overflow.exe",
        &overflow_bytes,
        HleRuntime::with_foundation(),
    )
    .unwrap();
    let report = overflow_session.run(100);
    assert!(matches!(
        report.stop,
        StopReason::Fault(CpuFault::StackOverflow { .. })
    ));
}

#[test]
fn hle_dispatch_reads_six_arguments_across_registers_and_stack() {
    use xholy::core::hle::runtime::HleValue;

    let mut runtime = HleRuntime::with_foundation();
    runtime
        .register(
            "xmem.dll",
            "XMemAlloc",
            |_ctx, args| {
                // Sum all 6 arguments to verify every register & stack slot.
                let sum: u64 = args
                    .iter()
                    .map(|v| match v {
                        HleValue::Unsigned(u) => *u,
                        _ => 0,
                    })
                    .sum();
                Ok(HleValue::Unsigned(sum))
            },
            "synthetic 6-arg test",
            6,
        )
        .unwrap();

    // Entry RSP = XMEM_HEAP_BASE - 8 (rsp % 16 == 8).
    // Allocate 56 bytes (8 pad + 16 for 2 stack args + 32 shadow space) so pre-call RSP % 16 == 0:
    //   sub rsp, 56
    //   mov qword ptr [rsp+32], 50   ; arg4 (5th arg) -> after CALL push, at [rsp+40]
    //   mov qword ptr [rsp+40], 60   ; arg5 (6th arg) -> after CALL push, at [rsp+48]
    //   mov ecx, 10                  ; arg0
    //   mov edx, 20                  ; arg1
    //   mov r8d, 30                  ; arg2
    //   mov r9d, 40                  ; arg3
    //   call [rip + disp]            ; -> RVA 0x1600
    //   add rsp, 56
    //   ret
    //
    // Let's compute RIP-relative displacement for `call [rip+disp]` to RVA 0x1600:
    // Entry is at RVA 0x1000.
    let mut code: Vec<u8> = vec![
        0x48, 0x83, 0xEC, 56, // sub rsp, 56 (4 bytes, RVA 0x1004)
        0x48, 0xC7, 0x44, 0x24, 32, 50, 0, 0,
        0, // mov qword [rsp+32], 50 (9 bytes, RVA 0x100D)
        0x48, 0xC7, 0x44, 0x24, 40, 60, 0, 0,
        0, // mov qword [rsp+40], 60 (9 bytes, RVA 0x1016)
        0xB9, 10, 0, 0, 0, // mov ecx, 10 (5 bytes, RVA 0x101B)
        0xBA, 20, 0, 0, 0, // mov edx, 20 (5 bytes, RVA 0x1020)
        0x41, 0xB8, 30, 0, 0, 0, // mov r8d, 30 (6 bytes, RVA 0x1026)
        0x41, 0xB9, 40, 0, 0, 0, // mov r9d, 40 (6 bytes, RVA 0x102C)
        0xFF, 0x15, 0, 0, 0, 0, // call [rip+disp] (6 bytes, next_rip RVA = 0x1032)
        0x48, 0x83, 0xC4, 56,   // add rsp, 56
        0xC3, // ret
    ];
    let disp = (0x1600i32 - 0x1032i32).to_le_bytes();
    code[0x2E..0x32].copy_from_slice(&disp);

    let mut bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
    bytes[0x400..0x400 + code.len()].copy_from_slice(&code);

    let mut session = GuestSession::load("six_args.exe", &bytes, runtime).unwrap();
    let report = session.run(1_000);

    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.cpu.register(reg::RAX), 10 + 20 + 30 + 40 + 50 + 60);
    assert_eq!(session.cpu().regs[reg::RSP], XMEM_HEAP_BASE);
    assert!(report.events.iter().any(|e| matches!(
        e,
        DiagnosticEvent::StackArgumentsRead { count: 2, values, .. } if values == &[50, 60]
    )));
}

#[test]
fn strict_abi_and_unsupported_conventions_fail_explicitly() {
    use xholy::core::pipeline::abi::{AbiFault, CallingConvention};

    // 1. Strict Microsoft x64 rejects synthetic_pe_calling because it doesn't align RSP / allocate shadow space.
    let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
    let mut session =
        GuestSession::load("strict.exe", &bytes, HleRuntime::with_foundation()).unwrap();
    session.set_calling_convention(CallingConvention::MicrosoftX64 {
        strict_prologue: true,
    });
    let report = session.run(1_000);
    assert!(matches!(
        report.stop,
        StopReason::AbiFault(AbiFault::MisalignedStack {
            expected_mod16: 8,
            ..
        })
    ));
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, DiagnosticEvent::AbiFault { .. })));

    // 2. Unsupported calling convention fails explicitly.
    let mut unsupported_session =
        GuestSession::load("sysv.exe", &bytes, HleRuntime::with_foundation()).unwrap();
    unsupported_session.set_calling_convention(CallingConvention::Unsupported("System V AMD64"));
    let report = unsupported_session.run(1_000);
    assert!(matches!(
        report.stop,
        StopReason::AbiFault(AbiFault::UnsupportedConvention { ref convention })
            if convention == "System V AMD64"
    ));
}
