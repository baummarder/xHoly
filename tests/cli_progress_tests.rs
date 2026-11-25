//! CLI checks for `--progress` and `--analyze` (read-only commands).

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
    assert!(text.contains("Last verification: 2026-11-25"), "{text}");
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
