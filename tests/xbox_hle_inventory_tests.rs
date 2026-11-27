//! Milestone 37 — honest Xbox HLE inventory tests.
//!
//! The registry is metadata-only. These tests prove that every Xbox entry
//! is dispatched through the single resolver/dispatcher, that Unknown and
//! Unsupported entries never return a fabricated value, and that the two
//! partial xmem shims behave within their evidenced contract.

use xholy::core::hle::resolve::ApiVerdict;
use xholy::core::hle::runtime::{DispatchResult, DispatchStatus, HleRuntime, HleValue};
use xholy::core::hle::{HleRegistry, ImplementationState};
use xholy::core::loader::pe64;
use xholy::core::pipeline::diagnostics::DiagnosticEvent;
use xholy::core::pipeline::{GuestSession, StopReason};

fn xbox_runtime() -> HleRuntime {
    HleRuntime::with_foundation()
}

/// The registry inventory matches the documented audit exactly.
#[test]
fn registry_inventory_matches_the_documented_audit() {
    let registry = HleRegistry::with_known_entries();
    assert_eq!(registry.modules.len(), 7);
    assert_eq!(registry.functions.len(), 21);

    let mut implemented = 0;
    let mut partial = 0;
    let mut unsupported = 0;
    let mut unknown = 0;
    for function in &registry.functions {
        match function.state {
            ImplementationState::Implemented => implemented += 1,
            ImplementationState::PartiallyImplemented => partial += 1,
            ImplementationState::Unsupported => unsupported += 1,
            ImplementationState::Unknown => unknown += 1,
        }
        assert!(
            !function.evidence.is_empty(),
            "{}!{} has no evidence",
            function.module,
            function.name
        );
    }
    assert_eq!(implemented, 0);
    assert_eq!(partial, 2);
    assert_eq!(unsupported, 2);
    assert_eq!(unknown, 17);

    // The xHoly VM namespace is separate and never enters this inventory.
    assert_eq!(
        registry.function_state("xh_vm.sys.dll", "VmTicks"),
        ImplementationState::Unknown
    );
    let vm = HleRegistry::with_xh_vm_entries();
    assert_eq!(vm.modules.len(), 4);
    assert_eq!(vm.functions.len(), 17);
    for module in &vm.modules {
        assert!(module.evidence.contains("NOT an Xbox API"));
    }
}

/// All 21 entries are dispatched centrally; no entry returns a fake value.
#[test]
fn every_xbox_entry_dispatches_through_the_single_runtime() {
    let registry = HleRegistry::with_known_entries();
    let mut runtime = xbox_runtime();
    let functions = registry.functions.clone();
    assert_eq!(functions.len(), 21);

    for function in functions {
        let verdict = ApiVerdict::resolve(&registry, &function.module, &function.name);
        let result: DispatchResult = runtime.dispatch(
            &function.module,
            &function.name,
            &[HleValue::Unsigned(64), HleValue::Unsigned(0)],
        );
        assert_eq!(result.module, function.module);
        assert_eq!(result.symbol, function.name);
        match function.state {
            ImplementationState::Unknown => {
                assert_eq!(result.status, DispatchStatus::Unknown);
                assert_eq!(result.value, None, "{}!{}", function.module, function.name);
                assert_eq!(verdict.state, ImplementationState::Unknown);
            }
            ImplementationState::Unsupported => {
                assert_eq!(result.status, DispatchStatus::Unsupported);
                assert_eq!(result.value, None);
                assert_eq!(verdict.state, ImplementationState::Unsupported);
            }
            ImplementationState::PartiallyImplemented => {
                // Only the two registered xmem shims may complete.
                assert!(matches!(
                    result.status,
                    DispatchStatus::Completed | DispatchStatus::Failed
                ));
            }
            ImplementationState::Implemented => {
                // The registry currently has none; if one is added it must
                // be registered and actually implemented.
                assert_eq!(result.status, DispatchStatus::Completed);
            }
        }
    }

    // Dispatch counts are bounded and derived from the same runtime.
    let stats = runtime.stats();
    assert_eq!(
        stats.completed + stats.unsupported + stats.unknown + stats.failed,
        21
    );
    assert!(runtime.dispatch_trace().len() <= 128);
}

/// Explicit function verdicts beat module fallback for every symbol.
#[test]
fn resolver_precedence_holds_for_the_whole_inventory() {
    let registry = HleRegistry::with_known_entries();
    for function in &registry.functions {
        let verdict = ApiVerdict::resolve(&registry, &function.module, &function.name);
        assert_eq!(verdict.state, function.state, "{}", function.name);
    }
    // Unknown module and symbol cannot inherit support from xmem.dll.
    let verdict = ApiVerdict::resolve(&registry, "xmem.dll", "NotListed");
    assert_eq!(verdict.state, ImplementationState::Unknown);
}

/// XMemAlloc/XMemFree lifecycle with valid and invalid parameters.
#[test]
fn xmem_shims_have_honest_parameter_and_lifecycle_behavior() {
    use xholy::core::hle::xmem::XMEM_HEAP_BASE;
    let mut runtime = xbox_runtime();

    // Invalid size: the heap refuses zero and reports a guest failure.
    let bad = runtime.dispatch(
        "xmem.dll",
        "XMemAlloc",
        &[HleValue::Unsigned(0), HleValue::Unsigned(0)],
    );
    assert_eq!(bad.status, DispatchStatus::Failed);
    assert_eq!(bad.value, None);

    // Wrong argument shape is refused, not guessed.
    let wrong = runtime.dispatch("xmem.dll", "XMemAlloc", &[HleValue::Unsigned(64)]);
    assert_eq!(wrong.status, DispatchStatus::Failed);
    assert!(
        wrong.detail.contains("invalid arguments"),
        "{}",
        wrong.detail
    );

    // Valid allocation/free lifecycle; attributes are recorded, not decoded.
    let addr = match runtime.dispatch(
        "xmem.dll",
        "XMemAlloc",
        &[HleValue::Unsigned(64), HleValue::Unsigned(0xABCD)],
    ) {
        DispatchResult {
            status: DispatchStatus::Completed,
            value: Some(HleValue::Unsigned(addr)),
            ..
        } => addr,
        other => panic!("expected allocation, got {other:?}"),
    };
    assert!(
        addr >= XMEM_HEAP_BASE,
        "guest address, never a host pointer"
    );
    let info = runtime.context_ref().xmem.query(addr).unwrap();
    assert_eq!(
        info.attributes, 0xABCD,
        "raw attribute word recorded verbatim"
    );
    assert_eq!(info.size, 64);
    let ok = runtime.dispatch(
        "xmem.dll",
        "XMemFree",
        &[HleValue::Unsigned(addr), HleValue::Unsigned(0xABCD)],
    );
    assert_eq!(ok.status, DispatchStatus::Completed);
    let double = runtime.dispatch(
        "xmem.dll",
        "XMemFree",
        &[HleValue::Unsigned(addr), HleValue::Unsigned(0)],
    );
    assert_eq!(double.status, DispatchStatus::Failed);
    assert!(double.detail.contains(&format!("{addr:#x}")));
}

/// A guest calling an Unknown/Unsupported import stops with no fake value.
#[test]
fn guest_imports_never_receive_fabricated_success() {
    for (module, symbol, reason) in [
        ("mystery.dll", "DoThing", "UnknownImport"),
        ("d3d12_xs.dll", "D3D12CreateDevice", "UnsupportedImport"),
    ] {
        let bytes = pe64::synthetic_pe_calling(module, symbol);
        let mut session = GuestSession::load("xbox.exe", &bytes, xbox_runtime()).expect("load");
        let report = session.run(1_000);
        match reason {
            "UnknownImport" => assert!(matches!(report.stop, StopReason::UnknownImport { .. })),
            "UnsupportedImport" => {
                assert!(matches!(report.stop, StopReason::UnsupportedImport { .. }))
            }
            _ => unreachable!(),
        }
        assert_eq!(report.dispatches().count(), 1);
        for event in report.dispatches() {
            if let DiagnosticEvent::HleDispatch { value, .. } = event {
                assert_eq!(*value, None, "no value for {reason}");
            }
        }
    }
}

/// xHoly VM services and Xbox APIs stay separate in a single guest session.
#[test]
fn vm_services_are_not_xbox_apis() {
    let mut runtime = HleRuntime::with_xh_vm();
    let vm_result = runtime.dispatch("xh_vm.sys.dll", "VmTicks", &[HleValue::Unsigned(1)]);
    assert_eq!(
        vm_result.status,
        DispatchStatus::Failed,
        "wrong arity fails"
    );

    let xbox_result = runtime.dispatch(
        "xmem.dll",
        "XMemAlloc",
        &[HleValue::Unsigned(32), HleValue::Unsigned(0)],
    );
    assert_eq!(xbox_result.status, DispatchStatus::Completed);
    assert_eq!(
        runtime
            .registry()
            .function_state("xh_vm.sys.dll", "VmTicks"),
        ImplementationState::Implemented
    );
    assert_eq!(
        HleRegistry::with_known_entries().function_state("xh_vm.sys.dll", "VmTicks"),
        ImplementationState::Unknown
    );
}

/// Regression: M33/M34/M35/M36 behaviour is unchanged.
#[test]
fn earlier_milestones_remain_intact() {
    let boot = xholy::core::pipeline::boot::run_synthetic_boot(1_000).unwrap();
    assert_eq!(
        boot.status,
        xholy::core::pipeline::boot::BootStatus::Success
    );
    assert_eq!(boot.rax, 0x4D33);
    assert_eq!(boot.steps, 6);

    let compute = pe64::synthetic_compute_guest();
    let mut session =
        GuestSession::load("compute.exe", &compute, HleRuntime::with_foundation()).unwrap();
    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.cpu.register(xholy::core::cpu::x64::reg::RAX), 16);

    let multi = pe64::synthetic_multi_hle_guest();
    let mut session = GuestSession::load("multi.exe", &multi, HleRuntime::with_xh_vm()).unwrap();
    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned);
    assert_eq!(report.imports.len(), 2);
}
