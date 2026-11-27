# Milestone 34 — GUI Trace Integration

**Status:** implemented and verified (synthetic fixtures only)
**Verification date:** 2026-11-28
**Evidence:** `tests/gui_trace_tests.rs` (6 tests), `src/core/pipeline/trace.rs`
unit tests, `tests/cli_progress_tests.rs`, `cargo test --workspace`

## Goal

Make the M33 guest execution and boot trace observable in the graphical
interface and support importing/exporting execution reports — without
rewriting the loader, CPU, GuestMemory model or HLE registry.

## What changed

### 1. Deserializable report graph

`ExecutionReport` and everything reachable from it can now be
deserialised: `ImageMeta`, `StopReason`, `StepRecord`, `StepKind`,
`ResolvedImport`, `DiagnosticEvent`, `AbiFault`, `CpuFault`, `CpuSnapshot`
and the M34 `BootTrace` types. `CpuFault::IllegalInstruction::reason`
changed from `&'static str` to an owned `String`; the decoder's static
messages are copied into it. No host pointer or borrowed data is part of
a report.

`--trace --json` and `--guest-test --json` keep their existing wire
format, so M33 behaviour and CLI tests are unchanged.

### 2. Versioned trace file format

Extension: `.xholy-trace.json` (pretty JSON).

```json
{
  "format": "xholy-trace",
  "schema_version": 1,
  "kind": "Boot",
  "report": { "...": "ExecutionReport" },
  "boot": { "guest": "...", "status": "Success", "events": ["..."] },
  "steps": [ { "step": 1, "rip": "...", "kind": "Executed" } ]
}
```

- `format` must equal `xholy-trace`, `schema_version` must equal `1`.
- `kind` is `Execution` (`--trace`) or `Boot` (`--guest-test`).
- `report` is always present; `boot` is present only for `kind: Boot`.
- `steps` preserves per-instruction records.
- Imports are bounded to 64 MiB and validated: step numbers must start
  at 1 and be consecutive, step count must not exceed retired
  instructions, boot result must agree with the report, and a boot trace
  must contain a `FinalResult` phase.
- Failures are structured (`TraceImportError`): invalid JSON, too large,
  not a trace, unsupported version, malformed/inconsistent data.

### 3. GUI integration

- `GuiConfig::trace_file` (CLI `--gui-trace PATH`) is loaded at startup
  for windowed, headless and TUI runs.
- `WindowedGuiState` holds `boot_trace`, `trace_steps`, `trace_path` and
  `trace_status`; `set_trace_report`, `set_boot_trace`, `load_trace_file`
  and `clear_trace` attach or remove real data.
- The HLE Trace Viewer gained a sixth panel, `BOOT`, which renders the
  ordered boot phases, instruction records and the verdict line. The
  summary bar shows SUCCESS / INCOMPLETE / FAILED in accent / yellow /
  red plus source path and last file-action result.
- Four controls in the viewer perform real actions:
  **OPEN**, **SAVE**, **REFRESH**, **CLEAR**. Keyboard shortcuts:
  Ctrl+O, Ctrl+S, Ctrl+R, Delete. No native file dialog dependency is
  added; the path comes from `--gui-trace` or the viewer's source field.

### 4. CLI integration

Existing commands are untouched. `--gui-trace PATH` is consumed only in
the GUI configuration block and is a documented no-op on CLI-only runs,
so headless execution never requires a graphical environment.

## Workflow

```powershell
xholy --guest-test --json > boot.json          # M33 wire format
# Export a versioned file from a report/boot trace via the library or GUI Save.
xholy --gui --gui-trace captured.xholy-trace.json
xholy --gui-headless --gui-trace captured.xholy-trace.json
```

## Tests

- `src/core/pipeline/trace.rs`: export/import round trip for report and
  boot, invalid JSON, unknown file, unsupported version, missing fields,
  inconsistent steps/boot payload, empty trace, 2,000 bounded steps.
- `tests/gui_trace_tests.rs`: GUI state receives an imported boot trace,
  boot panel renders verdict/phases/instructions, incomplete and failed
  states, clear removes all trace data, shortcuts emit real actions, HLE
  presentation data survives import.
- `tests/cli_progress_tests.rs`: `--guest-test --json` remains
  deserialisable as `BootTrace`; versioned wrapper validates;
  `--gui-headless --gui-trace` works without a display.

## Limitations

- No native file picker; paths are supplied on the command line or by the
  viewer source field. Typing arbitrary new paths inside the GUI is not
  implemented.
- The GUI cannot execute a PE itself; `--gui-trace` only parses data.
- Imported traces are data, not executable content; no guest file is ever
  run by the import path.
- `XMem` shims remain partially implemented; no retail boot, GDK or MMU.
- No compatibility percentage is derived from trace contents.

## M35 suggestion

Add a small path-entry field to the trace viewer so Open/Save can target
a new file without the CLI, and render the imported instruction records
with their register snapshots as a step-by-step inspector.
