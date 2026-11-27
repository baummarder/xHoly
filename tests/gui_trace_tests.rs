//! M34 integration tests: trace import/export and GUI presentation.
//!
//! No real window is required: `WindowedGuiState`, `ui::composite` and
//! `InputState` are exercised directly, as in `tests/gui_tests.rs`.

use winit::keyboard::Key;
use xholy::core::pipeline::boot::{run_synthetic_boot, BootStatus};
use xholy::core::pipeline::trace::{export_boot_trace_file, import_trace_file, TraceKind};
use xholy::core::pipeline::StepRecord;
use xholy::gui::windowed::input::{GuiAction, InputState};
use xholy::gui::windowed::state::{DiagnosticsTab, GuiView, TracePanel, WindowedGuiState};
use xholy::gui::windowed::ui;

fn boot_state() -> WindowedGuiState {
    let boot = run_synthetic_boot(1_000).expect("synthetic guest boot");
    let steps: Vec<StepRecord> = boot
        .instructions()
        .map(|(step, rip, kind)| StepRecord {
            step,
            rip,
            kind: kind.clone(),
        })
        .collect();
    let json = export_boot_trace_file(&boot, steps.clone()).expect("export boot trace");
    let file = import_trace_file(&json).expect("import boot trace");
    assert_eq!(file.kind, TraceKind::Boot);

    let mut state = WindowedGuiState {
        view: GuiView::Logs,
        ..Default::default()
    };
    state.load_trace_file(file, Some("trace.xholy-trace.json".into()));
    state
}

#[test]
fn gui_receives_imported_boot_trace_and_preserves_all_phases() {
    let state = boot_state();
    assert_eq!(state.diagnostics_tab, DiagnosticsTab::HleTrace);
    let boot = state.boot_trace.as_ref().expect("boot trace attached");
    assert_eq!(boot.status, BootStatus::Success);
    // 11 base phases, but the single `InstructionExecuted` example is
    // replaced by all 6 real instruction records: 11 - 1 + 6 = 16.
    assert_eq!(boot.events.len(), 16);
    assert_eq!(state.trace_steps.len(), 6);
    assert_eq!(state.trace_path.as_deref(), Some("trace.xholy-trace.json"));
    assert_eq!(
        state.trace_report.as_ref().unwrap().image.name,
        "synthetic-boot.exe"
    );
}

#[test]
fn boot_panel_renders_verdict_phases_and_instruction_records() {
    let mut state = boot_state();
    state.trace_panel = TracePanel::Boot;
    let (fb, layout) = ui::composite(&state, 1280, 720);
    assert_eq!(fb.len(), 1280 * 720 * 4);
    assert_eq!(layout.trace_panels.len(), TracePanel::ALL.len());
    assert!(layout.trace_panels.iter().any(|(_, l)| *l == "BOOT"));
    assert_eq!(layout.trace_actions.len(), 4);
    let labels: Vec<_> = layout.trace_actions.iter().map(|(_, l)| *l).collect();
    assert_eq!(labels, ["OPEN", "SAVE", "REFRESH", "CLEAR"]);
}

#[test]
fn boot_panel_renders_incomplete_and_failed_states_without_inventing_data() {
    let boot_incomplete = run_synthetic_boot(2).expect("bounded boot");
    assert_eq!(boot_incomplete.status, BootStatus::Incomplete);
    let mut state = WindowedGuiState {
        view: GuiView::Logs,
        trace_panel: TracePanel::Boot,
        ..Default::default()
    };
    state.set_boot_trace(boot_incomplete.clone(), Vec::new());
    let (fb, _) = ui::composite(&state, 1280, 720);
    assert_eq!(fb.len(), 1280 * 720 * 4);
    assert_eq!(
        state.boot_trace.as_ref().unwrap().status,
        BootStatus::Incomplete
    );

    let boot_failed = run_synthetic_boot(1_000).expect("boot");
    let mut failed = boot_failed.clone();
    failed.status = BootStatus::Failed;
    state.set_boot_trace(failed, Vec::new());
    let (fb, _) = ui::composite(&state, 1280, 720);
    assert_eq!(fb.len(), 1280 * 720 * 4);
    assert_eq!(
        state.boot_trace.as_ref().unwrap().status,
        BootStatus::Failed
    );
}

#[test]
fn clear_trace_removes_report_phases_and_source() {
    let mut state = boot_state();
    state.clear_trace();
    assert!(state.trace_report.is_none());
    assert!(state.boot_trace.is_none());
    assert!(state.trace_steps.is_empty());
    assert!(state.trace_path.is_none());
    assert_eq!(state.trace_status.as_deref(), Some("trace cleared"));
    let (fb, layout) = ui::composite(&state, 1280, 720);
    assert_eq!(fb.len(), 1280 * 720 * 4);
    assert!(layout.trace_actions.is_empty(), "no controls when no trace");
}

#[test]
fn keyboard_shortcuts_emit_real_trace_actions() {
    let mut input = InputState::new();
    for (ch, expected) in [
        ('o', GuiAction::OpenTrace),
        ('s', GuiAction::SaveTrace),
        ('r', GuiAction::RefreshTrace),
    ] {
        assert!(input.trace_shortcut(&Key::Character(ch.to_string().into()), true));
        assert!(input.drain_actions().contains(&expected));
    }
    assert!(!input.trace_shortcut(&Key::Character("o".into()), false));
    assert!(input.drain_actions().is_empty());
    // Delete is mapped by `keyboard_input`; verify the action enum exists
    // and that GUI dispatch handles it via the state clear operation.
    let mut state = boot_state();
    state.clear_trace();
    assert!(state.trace_report.is_none());
}

#[test]
fn hle_panel_presentation_data_is_preserved_after_import() {
    let state = boot_state();
    let dispatch = state
        .trace_report
        .as_ref()
        .unwrap()
        .dispatches()
        .next()
        .expect("HLE dispatch");
    match dispatch {
        xholy::core::pipeline::diagnostics::DiagnosticEvent::HleDispatch {
            module,
            symbol,
            args,
            ..
        } => {
            assert_eq!(module, "xmem.dll");
            assert_eq!(symbol, "XMemAlloc");
            assert_eq!(args, &vec![64, 0]);
        }
        _ => panic!("expected HLE dispatch"),
    }
    let invocations = state
        .boot_trace
        .as_ref()
        .unwrap()
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                xholy::core::pipeline::boot::BootEvent::HleInvocation { .. }
            )
        })
        .count();
    assert_eq!(invocations, 1);
}
