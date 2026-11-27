//! M35 — Guest Execution Expansion integration tests.
//!
//! All guests are generated from source. No retail binary, no proprietary
//! SDK content and no host pointer is involved.

use xholy::core::cpu::x64::reg;
use xholy::core::hle::runtime::HleRuntime;
use xholy::core::hle::xmem::{GuestHeap, XMEM_HEAP_BASE};
use xholy::core::loader::pe64::{
    self, Pe64Image, SYNTHETIC_COMPUTE_CODE, SYNTHETIC_COMPUTE_RESULT,
};
use xholy::core::pipeline::boot::{run_synthetic_boot, BootStatus};
use xholy::core::pipeline::trace::{import_trace_file, TraceFile};
use xholy::core::pipeline::{GuestSession, StepKind, StopReason};

fn load(bytes: &[u8]) -> GuestSession {
    GuestSession::load("expansion.exe", bytes, HleRuntime::with_foundation())
        .expect("load synthetic guest")
}

/// The compute fixture is a valid, self-contained PE32+ image.
#[test]
fn compute_guest_fixture_is_valid() {
    let bytes = pe64::synthetic_compute_guest();
    let image = pe64::parse(&bytes).expect("parse");
    pe64::validate(&image).expect("validate");
    assert_eq!(image.entry_rva, 0x1000);
    assert_eq!(
        &bytes[0x400..0x400 + SYNTHETIC_COMPUTE_CODE.len()],
        SYNTHETIC_COMPUTE_CODE
    );
    assert_eq!(image.imports.len(), 1);
    assert_eq!(image.imports[0].dll, "xmem.dll");
}

/// The complex guest really executes all combined operations.
#[test]
fn complex_guest_executes_loop_lea_hle_and_conditional_set() {
    let mut session = load(&pe64::synthetic_compute_guest());
    let report = session.run(1_000);
    assert_eq!(report.stop, StopReason::Returned, "{:?}", report.stop);
    assert_eq!(report.cpu.register(reg::RAX), SYNTHETIC_COMPUTE_RESULT);
    assert_eq!(report.steps, 21, "3 loop iterations + caller + HLE tail");

    let mnemonics: Vec<_> = session
        .step_records()
        .iter()
        .filter_map(|r| r.mnemonic.as_deref())
        .collect();
    assert!(mnemonics.contains(&"dec"), "{mnemonics:?}");
    assert!(mnemonics.contains(&"jcc"), "{mnemonics:?}");
    assert!(mnemonics.contains(&"lea"), "{mnemonics:?}");
    assert!(mnemonics.contains(&"movzx"), "{mnemonics:?}");
    assert!(mnemonics.contains(&"setcc"), "{mnemonics:?}");
    assert!(mnemonics.contains(&"call"), "{mnemonics:?}");

    let hle_calls = session
        .step_records()
        .iter()
        .filter(|r| matches!(r.kind, StepKind::HleCall { .. }))
        .count();
    assert_eq!(hle_calls, 1);
    let dispatch = report.dispatches().next().expect("HLE dispatch");
    match dispatch {
        xholy::core::pipeline::diagnostics::DiagnosticEvent::HleDispatch {
            args, detail, ..
        } => {
            assert_eq!(args, &vec![16, 0], "LEA computed size 16");
            assert!(detail.contains("guest="), "{detail}");
            assert!(detail.contains("attrs="), "{detail}");
        }
        _ => panic!("expected dispatch"),
    }
}

/// M35 trace files carry mnemonics and still import cleanly.
#[test]
fn expansion_trace_round_trips_with_mnemonics() {
    let mut session = load(&pe64::synthetic_compute_guest());
    let report = session.run(1_000);
    let steps = session.step_records().to_vec();
    assert!(steps.iter().all(|s| s.mnemonic.is_some()));

    let json = TraceFile::from_report(&report, steps.clone())
        .to_json()
        .expect("serialize");
    let file = import_trace_file(&json).expect("import");
    assert_eq!(file.schema_version, 2);
    assert_eq!(file.steps, steps);
    assert_eq!(file.report, report);
}

/// HLE diagnostics report the actual guest address on a bad free.
#[test]
fn invalid_hle_pointer_is_diagnosed_with_its_guest_address() {
    let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemFree");
    let mut session = load(&bytes);
    let report = session.run(1_000);
    assert_eq!(
        report.stop,
        StopReason::HleFailure {
            module: "xmem.dll".into(),
            symbol: "XMemFree".into(),
            detail: "runtime failure: invalid guest address 0x40".into(),
        }
    );
}

/// GuestHeap query exposes recorded metadata without host addresses.
#[test]
fn guest_heap_query_reports_guest_metadata_only() {
    let mut heap = GuestHeap::with_defaults();
    let addr = heap.alloc(64, 16, 0xAB).expect("alloc");
    let info = heap.query(addr + 8).expect("query by interior address");
    assert_eq!(info.base, addr);
    assert_eq!(info.size, 64);
    assert_eq!(info.alignment, 16);
    assert_eq!(info.attributes, 0xAB);
    assert_eq!(
        info.protection,
        xholy::core::hle::xmem::Protection::ReadWrite
    );
    assert!(heap.query(XMEM_HEAP_BASE - 1).is_err());
    heap.free(addr).expect("free");
    assert!(
        heap.query(addr).is_err(),
        "freed allocation is not queryable"
    );
}

/// Loader rejects an entry point outside an executable section.
#[test]
fn loader_rejects_a_non_executable_entry_point() {
    let mut bytes = pe64::synthetic_pe();
    // .text section starts at file 0x188; characteristics at +36.
    let sec = 0x188 + 36;
    bytes[sec..sec + 4].copy_from_slice(&0x4000_0010u32.to_le_bytes()); // READ|DATA, no EXECUTE
    let image: Pe64Image = pe64::parse(&bytes).expect("parse");
    let err = pe64::validate(&image).expect_err("entry not executable");
    assert!(err.to_string().contains("executable section"), "{err}");
    assert!(matches!(
        GuestSession::load("bad.exe", &bytes, HleRuntime::with_foundation()),
        Err(xholy::core::pipeline::LoadError::InvalidImage(_))
    ));
}

/// M33/M34 regressions remain intact after expansion.
#[test]
fn m33_boot_and_m34_trace_remain_unchanged() {
    let boot = run_synthetic_boot(1_000).expect("M33 boot");
    assert_eq!(boot.status, BootStatus::Success);
    assert_eq!(boot.rax, 0x4D33);
    assert_eq!(boot.steps, 6);
    let boot_json = boot.export_json().expect("M33 wire format");
    let parsed: serde_json::Value = serde_json::from_str(&boot_json).unwrap();
    assert_eq!(parsed["status"], "Success");

    let file = TraceFile::from_boot(&boot, Vec::new());
    let json = file.to_json().expect("M34 file format");
    let imported = import_trace_file(&json).expect("M34 import");
    assert_eq!(imported.into_boot().unwrap(), boot);
}
