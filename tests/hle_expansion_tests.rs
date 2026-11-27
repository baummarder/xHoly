//! M36-MEGA — HLE Runtime Expansion integration tests.
//!
//! All guests are generated from source; no retail binary and no
//! proprietary SDK content is used. xHoly VM services are explicitly
//! separate from the Xbox compatibility registry.

use xholy::core::hle::kernel::{KernelContext, WaitResult};
use xholy::core::hle::runtime::{DispatchStatus, HleRuntime};
use xholy::core::hle::system::{SystemContext, VM_VERSION_MAJOR};
use xholy::core::hle::{HleRegistry, ImplementationState};
use xholy::core::loader::pe64;
use xholy::core::memory::GuestMemory;
use xholy::core::pipeline::boot::{run_synthetic_boot, BootStatus};
use xholy::core::pipeline::diagnostics::DiagnosticEvent;
use xholy::core::pipeline::trace::hle_call_stats;
use xholy::core::pipeline::{GuestSession, StopReason};

fn load_multi() -> GuestSession {
    let bytes = pe64::synthetic_multi_hle_guest();
    GuestSession::load("multi.exe", &bytes, HleRuntime::with_xh_vm()).expect("load multi-HLE guest")
}

/// 1–3. Allocation, valid guest address, and guest use of that address.
#[test]
fn scenario_allocation_address_and_guest_use() {
    let mut session = load_multi();
    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned, "{:?}", report.stop);
    assert_eq!(report.steps, 9);
    assert_eq!(report.imports.len(), 2, "two import descriptors");
    assert_eq!(report.imports[0].module, "xmem.dll");
    assert_eq!(report.imports[1].module, "xh_vm.sys.dll");

    let allocations = report.dispatches().next().expect("XMemAlloc dispatch");
    let pointer = match allocations {
        DiagnosticEvent::HleDispatch { value, .. } => value.expect("guest pointer"),
        _ => panic!("expected dispatch"),
    };
    assert!(pointer >= xholy::core::hle::xmem::XMEM_HEAP_BASE);
    assert_eq!(
        session.space().read_u64(pointer),
        Some(pe64::SYNTHETIC_MULTI_MARKER),
        "guest wrote through the returned guest address"
    );
}

/// 4. Allocation information query through a guest-visible VM service.
#[test]
fn scenario_allocation_information_query() {
    let mut runtime = HleRuntime::with_xh_vm();
    let addr = runtime.context().xmem.alloc(64, 16, 0xAB).unwrap();
    let info = runtime.context().xmem.query(addr).unwrap();
    assert_eq!(info.base, addr);
    assert_eq!(info.size, 64);
    assert_eq!(info.attributes, 0xAB);
    let result = runtime.dispatch(
        "xh_vm.mem.dll",
        "VmQuery",
        &[xholy::core::hle::runtime::HleValue::Unsigned(addr + 8)],
    );
    assert_eq!(result.status, DispatchStatus::Completed);
    assert_eq!(
        result.value,
        Some(xholy::core::hle::runtime::HleValue::Unsigned(64))
    );
}

/// 5. Explicit free and double-free diagnostics.
#[test]
fn scenario_free_and_failed_api_call() {
    let mut runtime = HleRuntime::with_xh_vm();
    use xholy::core::hle::runtime::HleValue::Unsigned;
    let addr = runtime.context().xmem.alloc(32, 16, 0).unwrap();
    let ok = runtime.dispatch("xmem.dll", "XMemFree", &[Unsigned(addr), Unsigned(0)]);
    assert_eq!(ok.status, DispatchStatus::Completed);
    let failed = runtime.dispatch("xmem.dll", "XMemFree", &[Unsigned(addr), Unsigned(0)]);
    assert_eq!(failed.status, DispatchStatus::Failed);
    assert!(
        failed.detail.contains(&format!("{addr:#x}")),
        "{}",
        failed.detail
    );
    assert!(failed.failure.is_some(), "typed failure preserved");
}

/// 6. A failed API call stops the guest with a structured reason.
#[test]
fn scenario_guest_handles_a_failed_call() {
    let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemFree");
    let mut session =
        GuestSession::load("fail.exe", &bytes, HleRuntime::with_xh_vm()).expect("load");
    let report = session.run(1_000);
    assert!(matches!(report.stop, StopReason::HleFailure { .. }));
    assert_eq!(hle_call_stats(&report).failed, 1);
}

/// 7–8. Multiple modules in sequence and correct return values.
#[test]
fn scenario_multiple_modules_and_return_values() {
    let mut session = load_multi();
    let report = session.run(1_000);
    assert_eq!(report.steps, 9);
    // 64 (saved alloc size) + 1 (first VmTicks tick) = 65.
    assert_eq!(
        report.cpu.register(xholy::core::cpu::x64::reg::RAX),
        pe64::SYNTHETIC_MULTI_RESULT
    );
    let modules: Vec<_> = report
        .dispatches()
        .map(|e| match e {
            DiagnosticEvent::HleDispatch { module, symbol, .. } => (module.clone(), symbol.clone()),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        modules,
        vec![
            ("xmem.dll".into(), "XMemAlloc".into()),
            ("xh_vm.sys.dll".into(), "VmTicks".into()),
        ]
    );
}

/// 9. Stack arguments are read via the existing Microsoft x64 ABI.
#[test]
fn scenario_stack_arguments() {
    use xholy::core::cpu::x64::{reg, X64Cpu};
    use xholy::core::memory::{regions, Memory};
    use xholy::core::pipeline::abi::{read_hle_arguments, CallingConvention};

    // Callee-entry RSP must be 8 mod 16; stack arg #4 lives at RSP+40.
    let rsp = regions::TITLE + 0x1000 - 8;
    let mut cpu = X64Cpu::new();
    cpu.rip = regions::TITLE + 0x1000;
    cpu.regs[reg::RSP] = rsp;
    cpu.regs[reg::RCX] = 10;
    cpu.regs[reg::RDX] = 20;
    cpu.regs[reg::R8] = 30;
    cpu.regs[reg::R9] = 40;
    let mut memory = Memory::new();
    assert!(memory.write_u64(rsp + 40, 50));
    assert!(memory.write_u64(rsp + 48, 60));
    let args = read_hle_arguments(&cpu, &memory, CallingConvention::default(), 6)
        .expect("six arguments are readable");
    assert_eq!(args, vec![10, 20, 30, 40, 50, 60]);
}

/// 10. A traceable HLE trace survives export/import with derived stats.
#[test]
fn scenario_trace_export_import_and_stats() {
    let mut session = load_multi();
    let report = session.run(1_000);
    let stats = hle_call_stats(&report);
    assert_eq!(stats.total, 2);
    assert_eq!(stats.completed, 2);
    let file = xholy::core::pipeline::trace::TraceFile::from_report(
        &report,
        session.step_records().to_vec(),
    );
    let json = file.to_json().unwrap();
    let imported = xholy::core::pipeline::trace::import_trace_file(&json).unwrap();
    assert_eq!(imported.report, report);
    assert_eq!(hle_call_stats(&imported.report), stats);
    assert!(report.render_text().contains("hle calls: 2 total"));
}

/// 11. Unknown APIs are preserved, never silently successful.
#[test]
fn scenario_unknown_api_is_preserved() {
    let bytes = pe64::synthetic_pe_calling("mystery.dll", "DoThing");
    let mut session =
        GuestSession::load("unknown.exe", &bytes, HleRuntime::with_xh_vm()).expect("load");
    let report = session.run(1_000);
    assert_eq!(
        report.stop,
        StopReason::UnknownImport {
            module: "mystery.dll".into(),
            symbol: "DoThing".into()
        }
    );
    assert_eq!(report.cpu.register(xholy::core::cpu::x64::reg::RAX), 0);
    assert_eq!(hle_call_stats(&report).unknown, 1);
}

/// 12. Controlled error handling: invalid arguments, bounds, and limits.
#[test]
fn scenario_controlled_errors() {
    use xholy::core::hle::runtime::HleValue;
    let mut runtime = HleRuntime::with_xh_vm();
    let bad = runtime.dispatch("xh_vm.mem.dll", "VmQuery", &[HleValue::Unsigned(0)]);
    assert_eq!(bad.status, DispatchStatus::Failed);
    assert!(
        bad.detail.contains("invalid guest address"),
        "{}",
        bad.detail
    );

    let bad_args = runtime.dispatch("xh_vm.sys.dll", "VmTicks", &[HleValue::Unsigned(1)]);
    assert_eq!(bad_args.status, DispatchStatus::Failed);
    assert!(bad_args.detail.contains("invalid arguments"));

    let stats = runtime.stats();
    assert_eq!(stats.failed, 2);
    assert_eq!(runtime.dispatch_trace().len(), 2);
    runtime.reset_diagnostics();
    assert_eq!(runtime.stats().failed, 0);
    assert!(runtime.dispatch_trace().is_empty());
}

/// Module-level dispatcher tests: kernel timeouts and host-thread freedom.
#[test]
fn kernel_and_system_hle_foundations_are_deterministic() {
    let mut kernel = KernelContext::new();
    let thread = kernel.create_thread().unwrap();
    assert!(kernel.sleep(thread, 2));
    assert_eq!(kernel.wait_event(999, 0), WaitResult::InvalidHandle);
    let event = kernel.create_event().unwrap();
    assert_eq!(kernel.wait_event(event, 1), WaitResult::TimedOut);
    assert!(kernel.set_event(event));
    assert_eq!(kernel.wait_event(event, 0), WaitResult::Ready);

    let mut system = SystemContext::new();
    system.advance();
    assert_eq!(system.ticks(), 1);
    assert_eq!(system.info().major, VM_VERSION_MAJOR);
}

/// Registry honesty: Xbox entries unchanged, VM entries clearly separated.
#[test]
fn registry_status_is_honest() {
    let xbox = HleRegistry::with_known_entries();
    assert_eq!(xbox.modules.len(), 7);
    assert_eq!(xbox.functions.len(), 21);
    assert_eq!(
        xbox.function_state("xmem.dll", "XMemAlloc"),
        ImplementationState::PartiallyImplemented
    );
    assert_eq!(
        xbox.function_state("xmem.dll", "XMemQuery"),
        ImplementationState::Unknown
    );
    assert_eq!(
        xbox.function_state("xh_vm.sys.dll", "VmTicks"),
        ImplementationState::Unknown,
        "xHoly VM services must not appear in the Xbox registry"
    );

    let vm = HleRegistry::with_xh_vm_entries();
    for module in &vm.modules {
        assert!(
            module.evidence.contains("NOT an Xbox API"),
            "{}: {}",
            module.name,
            module.evidence
        );
    }
    let all = HleRegistry::with_all_entries();
    assert_eq!(all.functions.len(), 21 + vm.functions.len());
}

/// Bounded failure log: guest input cannot grow host memory without limit.
#[test]
fn failure_log_is_bounded() {
    use xholy::core::hle::runtime::HleValue;
    let mut runtime = HleRuntime::with_xh_vm();
    for _ in 0..200 {
        let _ = runtime.dispatch("xh_vm.mem.dll", "VmQuery", &[HleValue::Unsigned(0)]);
    }
    assert_eq!(
        runtime.failures().len(),
        xholy::core::hle::runtime::MAX_HLE_FAILURES
    );
    assert!(runtime.dropped_failures() > 0);
    assert!(runtime.dispatch_trace().len() <= xholy::core::hle::runtime::MAX_HLE_DISPATCH_TRACE);
}

/// M33/M34/M35 regressions.
#[test]
fn earlier_milestones_remain_intact() {
    let boot = run_synthetic_boot(1_000).expect("M33 boot");
    assert_eq!(boot.status, BootStatus::Success);
    assert_eq!(boot.rax, 0x4D33);
    assert_eq!(boot.steps, 6);
    let json = boot.export_json().unwrap();
    assert!(json.contains("\"status\": \"Success\""));

    let compute = pe64::synthetic_compute_guest();
    let mut session = GuestSession::load(
        "compute.exe",
        &compute,
        xholy::core::hle::runtime::HleRuntime::with_foundation(),
    )
    .unwrap();
    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.cpu.register(xholy::core::cpu::x64::reg::RAX), 16);
}
