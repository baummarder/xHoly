# Milestone 33 — First Guest Boot

**Status:** implemented and verified (synthetic fixtures only)
**Verification date:** 2026-11-27
**Evidence:** `tests/guest_boot_tests.rs` (13 integration tests),
`src/core/pipeline/boot.rs` unit tests, `cargo test --workspace`

## Goal

Prove that one self-contained, source-generated PE32+ guest can be loaded,
initialized, executed through the existing bounded x86-64 interpreter and
HLE runtime, and traced end-to-end — without retail binaries, proprietary
SDK content or invented Xbox APIs.

## What runs

`xholy --guest-test [--json] [--max-steps N]` executes the fixture from
`pe64::synthetic_boot_guest()`:

```text
Synthetic PE32+ (source-generated)
  → pe64::parse + validate
  → GuestSession::load (map, relocate, resolve imports, stack)
  → X64Cpu::step (bounded x86-64 subset)
  → FF /2 call through IAT slot RVA 0x1600 (HLE transition)
  → HleRuntime::dispatch("xmem.dll", "XMemAlloc", [64, 0])
  → RAX = guest heap pointer; guest writes marker 0x11
  → mov eax, 0x4D33; ret (entry sentinel)
  → BootTrace (11 ordered phases + ExecutionReport)
```

### Synthetic guest entry code

| Bytes | Instruction | Purpose |
|---|---|---|
| `B9 40 00 00 00` | `mov ecx, 64` | `XMemAlloc(size=64, …)` |
| `31 D2` | `xor edx, edx` | `XMemAlloc(…, attributes=0)` |
| `FF 15 F3 05 00 00` | `call [rip+0x5f3]` | IAT slot at RVA `0x1600` |
| `48 C7 00 11 00 00 00` | `mov qword [rax], 0x11` | guest heap marker |
| `B8 33 4D 00 00` | `mov eax, 0x4D33` | documented synthetic status |
| `C3` | `ret` | pops `ENTRY_RETURN_SENTINEL` |

`0x4D33` ("M33") is an xHoly test marker, **not** an Xbox status code.

## Boot status semantics

| Status | Meaning |
|---|---|
| `SUCCESS` | Guest returned, `RAX == 0x4D33`, HLE dispatch completed, guest heap marker verified |
| `INCOMPLETE` | Step budget exhausted or `HLT` reached before the guest finished |
| `FAILED` | CPU fault, ABI fault, or unsupported / unknown / failed import |

No status is derived from import coverage. No compatibility percentage is
claimed anywhere in the trace.

## Trace phases

`BootTrace.events` is ordered:

1. `LoadStarted`
2. `LoadCompleted`
3. `MemoryInitialized`
4. `StackInitialized`
5. `EntryPoint`
6. `ExecutionStarted`
7. `InstructionExecuted` (one per retired / faulting step)
8. `HleInvocation`
9. `HleReturn`
10. `GuestExit`
11. `FinalResult`

`BootTrace.report` is the unchanged `ExecutionReport` consumed by the
existing `--trace` export and GUI HLE Trace Viewer. `BootTrace` is
Serialize-only, like the existing report types: traces are written for
external consumers, never parsed back into typed events.

### Example (`xholy --guest-test`)

```text
[boot] load started: synthetic-boot.exe
[boot] load completed: base=0x20000000000 size=0x3000 entry=0x20000001000 imports=1
[boot] memory initialized: base=0x20000000000 image=0x3000 stack=0x2003fff0000..0x20040000000 heap=0x20040000000
[boot] stack initialized: 0x2003fff0000..0x20040000000 rsp=0x2003ffffff8 sentinel=0xffffffffffffff00
[boot] entry point: 0x20000001000
[boot] cpu started: entry=0x20000001000 max_steps=10000
[boot] step 1: rip=0x20000001000 executed
[boot] step 2: rip=0x20000001005 executed
[boot] step 3: rip=0x20000001007 hle call via 0x20000001600
[boot] step 4: rip=0x2000000100d executed
[boot] step 5: rip=0x20000001014 executed
[boot] step 6: rip=0x20000001019 executed
[boot] hle invoked: xmem.dll!XMemAlloc slot=0x20000001600 args=(0x40, 0x0)
[boot] hle return: xmem.dll!XMemAlloc Completed value=0x20040000000
[boot] guest exit: returned from entry point after 6 step(s)
[boot] final result: SUCCESS steps=6 rax=0x4d33 stop=returned from entry point
```

Values shown are from the synthetic fixture; guest addresses are stable
for this fixture and must not be treated as Xbox memory layout.

## Boundaries and limitations

- Only the existing x86-64 subset is used; no new decoder was added.
- `XMemAlloc`/`XMemFree` remain **Partially Implemented** shims with raw
  16-byte alignment and verbatim attribute recording. M33 does not
  upgrade their registry status.
- The HLE transition intercepts by guest IAT slot address; stale IAT
  contents are never read and no host pointer is written into guest memory.
- The GUI HLE Trace Viewer can already render the embedded
  `ExecutionReport` (`WindowedGuiState::set_trace_report`); M33 does not
  add GUI plumbing. A production path that loads a report into the GUI
  remains future work.
- No retail title boot, no GDK dependency, no ordinal resolution, no
  full x86-64 interpreter, no MMU.

## Tests

- `tests/guest_boot_tests.rs`: fixture validity, load, stack alignment,
  supported instructions, HLE call/arguments/return, marker verification,
  complete trace phases, explicit exit/final result, invalid/unsupported
  guests, instruction budget, faulting memory access, CLI text/JSON.
- Unit tests in `src/core/pipeline/boot.rs`: status, phase order,
  malformed guest rejection.
- Full suite: `cargo test --workspace`.
