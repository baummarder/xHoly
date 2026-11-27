# Milestone 36 — HLE Runtime Expansion (M36-MEGA)

**Status:** implemented and verified (synthetic fixtures and xHoly VM services)
**Verification date:** 2026-11-30
**Evidence:** `tests/hle_expansion_tests.rs` (13 tests), unit tests in
`src/core/hle/{resolve,kernel,system,runtime,mod}.rs`, `cargo test --workspace`

## Scope and honesty boundary

xHoly keeps a strict separation between Xbox API claims and xHoly-owned
scaffold services:

| Namespace | Modules | Functions | Purpose |
|---|---:|---:|---|
| Xbox registry (`with_known_entries`) | 7 | 21 | **Unchanged.** Compatibility metadata only. |
| xHoly VM services (`with_xh_vm_entries`) | 4 | 17 | Real host implementations, explicitly **NOT Xbox APIs**. |

No Xbox function was upgraded without documented semantics. No signature,
ordinal, structure layout or return convention was invented. ADR 0001 and
README remain normative: no verified Series X|S XAM ABI exists in the repo.

## M36.1 — Core architecture

- New `src/core/hle/resolve.rs`: pure `ApiVerdict` / `resolve_verdict`
  with documented precedence (explicit function → explicit unsupported
  module → Unknown). Shared by the import analyzer, the guest import
  binder and the dispatcher — one verdict rule, independently testable.
- `HleRuntime` additions (additive, no existing signature changed):
  - typed `DispatchFailure` (`Fault(HleFault)` / `UnbackedClaim`),
  - bounded failure log (`MAX_HLE_FAILURES = 64`) and `dropped_failures()`,
  - bounded `DispatchTrace` (`MAX_HLE_DISPATCH_TRACE = 128`),
  - `call_count(module, symbol)`, `reset_diagnostics()`,
  - `std::error::Error` impls for `HleFault`, `RegisterError`,
    `DispatchFailure`.
- Registry remains data-only; module paths and all existing derives and
  wire formats are preserved.

## M36.2 — Memory HLE

`XMemAlloc` / `XMemFree` remain **PartiallyImplemented** — the attribute
word is recorded verbatim, never decoded. Improvements:

- real guest-address diagnostics (`HleFault::InvalidAddress(actual addr)`),
- completed-call details contain `guest=0x… attrs=0x…` / `freed=0x…`,
- guest heap `query()` metadata is exposed through the xHoly VM service
  `xh_vm.mem.dll!VmQuery`; `VmProtect` and `VmStats` use the existing
  `Protection`/`HeapStats` model. No host pointer is ever returned.

## M36.3 — Kernel HLE (xHoly VM services)

New `src/core/hle/kernel.rs`, **synchronous, no host threads**:

- `GuestThread`, `ThreadState`, `GuestEvent`, `GuestMutex` as plain data;
- `KernelContext` with bounded tables (64 threads/events/mutexes);
- sleep/suspend/resume/yield/exit lifecycle, event signals and mutex
  ownership transfer;
- `WaitResult::{Ready, TimedOut, InvalidHandle, Deadlock}` — timeouts are
  deterministic VM ticks, never wall-clock sleeps; self-deadlock is
  reported rather than waited on.

Services: `xh_vm.krn.dll!{ThreadCreate,ThreadYield,ThreadSleep,
ThreadSuspend,ThreadResume,ThreadExit}` and
`xh_vm.sync.dll!{EventCreate,EventSet,EventWait,MutexCreate,MutexLock,
MutexUnlock}`.

## M36.4 — System HLE (xHoly VM services)

New `src/core/hle/system.rs`:

- `VmInfo` contains only project-owned constants (interface 1.0, table
  limits); no host OS/CPU/clock data is presented as Xbox information;
- `SystemContext` provides a monotonic, deterministic VM tick.

Services: `xh_vm.sys.dll!VmTicks`, `xh_vm.sys.dll!VmInfo`.

## M36.5 — Guest ABI integration

`pe64::synthetic_multi_hle_guest()` is source-generated with **two import
descriptors** (`xmem.dll!XMemAlloc` at IAT `0x1600`, `xh_vm.sys.dll!VmTicks`
at IAT `0x1610`). `GuestSession::load(..., HleRuntime::with_xh_vm())`
resolves both imports, the existing Microsoft x64 ABI reads arguments, the
single dispatcher calls both modules, and the guest returns
`RAX = 64 + 1 = 65`. No second calling-convention implementation exists.

## M36.6 — Diagnostics and tracing

- `trace::hle_call_stats(&ExecutionReport)` derives bounded call counters
  from the validated event list. **No schema change**: M34 schema v1 and
  M35 schema v2 files import unchanged, and no field was added to
  `ExecutionReport`/`StepRecord`.
- CLI text (`ExecutionReport::render_text`) and the GUI `HLE CALLS` panel
  render the same derived counters from the same report.

## M36.7 — Synthetic guest test suite

`tests/hle_expansion_tests.rs` covers the required scenarios: allocation,
valid guest address, guest use, allocation info query, explicit free and
failed call, guest handling of a failure, multiple modules, return values,
stack arguments via the existing ABI, trace export/import with stats,
unknown API preservation, and controlled errors (bad address, bad arity,
bounded logs, reset). It also asserts registry honesty and M33/M34/M35
regressions.

## API status overview (honest)

| API | Status | Notes |
|---|---|---|
| `xmem.dll!XMemAlloc` | PartiallyImplemented | 2-arg shape evidenced; attributes not decoded |
| `xmem.dll!XMemFree` | PartiallyImplemented | 2-arg shape evidenced; attributes ignored |
| `d3d12_xs.dll!D3D12CreateDevice` | Unsupported | No guest device shim |
| `gameinput.dll!GameInputCreate` | Unsupported | No shim |
| All other 17 Xbox entries | Unknown | No verified public signature |
| 17 `xh_vm.*` services | Implemented | xHoly VM scaffold, **not Xbox APIs**, fully tested |

## Limitations

No retail boot, no Xbox kernel ABI, no MMU, no full x86-64 ISA, no GPU
emulation, no GDK dependency. xHoly VM time is a deterministic tick, not a
wall-clock or QPC. Mutex waits are polling with bounded ticks, not host
blocking. No compatibility percentage is derived from VM services.

## M37 suggestion

Add a GUI step inspector that renders imported `StepRecord` mnemonics and
guest RIPs, and consider documented shift instructions (`C1 /4,/5`) for
bit-mask driven synthetic guests.
