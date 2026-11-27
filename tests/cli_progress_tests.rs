//! CLI checks for `--progress`, `--analyze`, `--trace` and `--guest-test`.

use std::path::PathBuf;
use std::process::Command;

fn xholy() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xholy"))
}

fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("xholy-cli-{}-{name}", std::process::id()))
}

#[test]
fn progress_command_prints_roadmap_summary_without_compatibility_claims() {
    let out = xholy()
        .arg("--progress")
        .output()
        .expect("run xholy --progress");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Overall roadmap completion"), "{text}");
    assert!(text.contains("Roadmap progress only"), "{text}");
    assert!(text.contains("Last verification: 2026-11-30"), "{text}");
}

#[test]
fn analyze_missing_file_fails_with_a_message() {
    let out = xholy()
        .args(["--analyze", "definitely-missing-file.exe"])
        .output()
        .expect("run xholy --analyze");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}

#[test]
fn analyze_rejects_non_pe_input() {
    let path = temp_file("text.bin");
    std::fs::write(&path, b"not an executable").unwrap();
    let out = xholy().arg("--analyze").arg(&path).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a supported PE32+ image"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn analyze_json_is_machine_readable_and_does_not_modify_the_file() {
    let bytes = xholy::core::loader::pe64::synthetic_pe();
    let path = temp_file("synthetic.exe");
    std::fs::write(&path, &bytes).unwrap();
    let out = xholy()
        .arg("--analyze")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["format"], "PE32+");
    assert_eq!(value["architecture"], "x86-64");
    assert!(value.get("imports").is_some());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "analysis must not modify the file"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn trace_command_exports_json_and_text_without_modifying_guest() {
    let bytes = xholy::core::loader::pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
    let path = temp_file("trace_guest.exe");
    let out_path = temp_file("trace_out.json");
    std::fs::write(&path, &bytes).unwrap();

    let json_out = xholy()
        .arg("--trace")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        json_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json_out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&json_out.stdout).expect("valid trace JSON");
    assert_eq!(parsed["stop"], "Returned");
    assert!(parsed["cpu"]["regs"].is_array());
    assert!(parsed["events"].is_array());

    let text_out = xholy().arg("--trace").arg(&path).output().unwrap();
    assert!(text_out.status.success());
    let text = String::from_utf8_lossy(&text_out.stdout);
    assert!(text.contains("[hle] xmem.dll!XMemAlloc"), "{text}");

    let file_out = xholy()
        .arg("--trace")
        .arg(&path)
        .arg("--json")
        .arg("--trace-out")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(file_out.status.success());
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("\"Returned\""), "{written}");

    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "trace export must not modify the guest binary"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn guest_test_command_runs_the_synthetic_boot_and_reports_success() {
    let out = xholy().arg("--guest-test").output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[boot] load started"), "{text}");
    assert!(text.contains("[boot] stack initialized"), "{text}");
    assert!(
        text.contains("[boot] hle invoked: xmem.dll!XMemAlloc"),
        "{text}"
    );
    assert!(text.contains("[boot] final result: SUCCESS"), "{text}");
    assert!(!text.contains('%'), "no compatibility percentage: {text}");

    let json_out = xholy()
        .args(["--guest-test", "--json", "--max-steps", "1000"])
        .output()
        .unwrap();
    assert!(json_out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&json_out.stdout).expect("valid boot trace JSON");
    assert_eq!(parsed["status"], "Success");
    assert_eq!(parsed["steps"], 6);
    assert_eq!(parsed["rax"], 0x4D33);
    assert_eq!(parsed["marker_verified"], true);
    assert!(parsed["events"].is_array());
    assert!(parsed["report"]["imports"].is_array());
}

#[test]
fn guest_test_json_is_importable_as_a_versioned_trace_file() {
    let out = xholy().args(["--guest-test", "--json"]).output().unwrap();
    assert!(out.status.success());
    // `--guest-test --json` stays the M33 wire format (bare BootTrace).
    // The M34 file format wraps the same data with a schema version.
    let boot: xholy::core::pipeline::boot::BootTrace =
        serde_json::from_slice(&out.stdout).expect("BootTrace deserialises");
    assert_eq!(
        boot.status,
        xholy::core::pipeline::boot::BootStatus::Success
    );
    let steps = boot
        .instructions()
        .map(
            |(step, rip, kind, mnemonic)| xholy::core::pipeline::StepRecord {
                step,
                rip,
                kind: kind.clone(),
                mnemonic: mnemonic.map(str::to_string),
            },
        )
        .collect();
    let file = xholy::core::pipeline::trace::TraceFile::from_boot(&boot, steps);
    let json = file.to_json().expect("versioned trace JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["format"], "xholy-trace");
    assert_eq!(parsed["schema_version"], 2);
    assert_eq!(parsed["kind"], "Boot");
    let imported =
        xholy::core::pipeline::trace::import_trace_file(&json).expect("versioned import");
    assert_eq!(imported, file);
}

#[test]
fn gui_trace_flag_does_not_require_a_graphical_environment() {
    let out = xholy()
        .args(["--gui-headless", "--gui-trace", "missing.xholy-trace.json"])
        .output()
        .unwrap();
    // The GUI reports the import problem in its status line; headless
    // rendering must still succeed without a window.
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn trace_command_missing_file_fails_cleanly() {
    let out = xholy()
        .args(["--trace", "definitely-missing-trace-guest.exe"])
        .output()
        .expect("run xholy --trace");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}
