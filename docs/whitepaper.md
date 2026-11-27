# xHoly Technical Whitepaper

*A research HLE architecture for Xbox Series X|S on Windows and Linux (x86-64)*

Version 0.2.0 (Phase 24B). Replaces the 0.1.0 text, which described Xbox One and ARM64 targets that are no longer part of the project. The binding decisions are in `docs/adr/0001-target-architecture.md`.

---

## 1. Scope

xHoly targets Xbox Series X|S (Gen9 / Scarlett) only. Hosts are Windows x86-64 and Linux x86-64, with equal standing. Xbox 360, Xbox One, ARM64 hosts and macOS are not targets.

The approach is high-level emulation (HLE): platform APIs are reproduced on the host, and guest code is not run through a full hardware model. ScarlettX is the compatibility reference; its source is not copied.

**Current state:** Research scaffold. No retail title runs. The figures in section 8 are goals, not measurements.

## 2. System overview

```
Game package ──► Package loader (PE32+, planned 24D)
                     │
                     ▼
              Guest x86-64 code ──► ABI bridges (Win64 ↔ host ABI, 24E)
                     │                       │
                     ▼                       ▼
            Kernel / API tables        GPU translation (24F)
            (verified ABI only)              │
                     │                       ▼
                     ▼             Backend: D3D12 (Windows)
            Memory model                     Vulkan (Linux)
                     │
                     ▼
                I/O: input, audio, storage
```

The Scala runtime is outside the target scope. Keeping it is an open decision (ADR 0001, question 3).

## 3. CPU execution

Xbox Series X|S executes x86-64 (Zen 2) PE32+ code. The target is native execution of that code on an x86-64 host, with Xbox ABI bridges where guest and host calling conventions differ. This avoids a full CPU emulation.

On Windows, guest and host share the Win64 calling convention. On Linux, PE code uses Win64 while the host uses SysV, so thunks are required. Their correctness is tested in phase 24E.

The existing x86-64 decoder and the x86-64→WASM JIT are research subsets. Whether they become the CPU path or are replaced by native mapping is decided in phase 24E.

The AArch64 interpreter remains only as a test harness. It does not model any Xbox target.

### Guest execution pipeline (M29–M36)

For bounded research tests, `src/core/pipeline/` drives a synthetic
PE32+ guest end-to-end:

```text
PE32+ loader → import resolution → X64Cpu → HLE dispatch → ExecutionReport → BootTrace
```

`GuestSession` owns the flat guest window and the xmem guest heap behind a
single `GuestSpace` façade. Guest addresses are explicit `u64` VAs; IAT
calls are intercepted by guest slot address and never contain host
pointers. The Microsoft x64 argument registers (`RCX/RDX/R8/R9`, return
in `RAX`) are documented in `src/core/pipeline/abi.rs`; strict stack
layout checks are available for arities above four arguments.

Milestone 33 adds `src/core/pipeline/boot.rs`: a self-contained synthetic
guest makes one `XMemAlloc` call through its IAT slot and returns a
documented status. Its ordered boot trace (load, memory, stack, entry
point, CPU steps, HLE invocation/return, exit, final result) is exported
by `xholy --guest-test [--json]` and can be fed to the existing HLE Trace
Viewer. This is a research fixture, not retail boot evidence.
See [milestone-33.md](milestone-33.md).

Milestone 34 completes the observation path: reports are deserialisable
and travel in a versioned `.xholy-trace.json` container. The GUI trace
viewer consumes boot phases, instruction records and the
SUCCESS/INCOMPLETE/FAILED verdict; Open/Save/Refresh/Clear perform real
file actions. Import is validated and never executes guest content. See
[milestone-34.md](milestone-34.md).

Milestone 35 expands the bounded subset with LEA, INC/DEC, MOVZX/MOVSX and
SETcc, adds guest-heap metadata queries and stricter executable-entry
validation, and executes a self-authored compute guest (`RAX = 16`) with
per-step mnemonics in the trace. The registry remains the single source of
truth; no partially implemented function is presented as complete. See
[milestone-35.md](milestone-35.md).

Milestone 36 turns the dispatcher into a modular HLE runtime: one pure
resolver shared by analysis and execution, bounded failure/dispatch logs,
typed failures, and deterministic xHoly VM services (memory, guest threads,
events, mutexes, system info) in a namespace explicitly separate from Xbox
APIs. A synthetic guest calls two modules in sequence and returns a
verifiable result. See [milestone-36.md](milestone-36.md).

Milestone 37 completes the evidence audit of all 21 Xbox API entries and
documents which ones can be implemented without inventing signatures,
flags, layouts or return conventions. None can: the registry therefore
keeps 0 Implemented, 2 PartiallyImplemented, 2 Unsupported and 17 Unknown
entries. All entries are dispatched through the single resolver/dispatcher
with no fabricated return value. xHoly VM services remain outside the Xbox
compatibility rating. See [xbox-hle-inventory.md](xbox-hle-inventory.md)
and [milestone-37.md](milestone-37.md).

## 4. GPU translation

Xbox Series X|S renders through Direct3D 12 on RDNA2. The translation layer maps guest D3D12 calls to a host backend: D3D12 on Windows, Vulkan on Linux.

Shader input is DXIL. Precompiled RDNA2 `XS2` microcode is not supported and is rejected with an explicit error. The current DXIL parser is not connected to a draw path.

The software rasterizer exists for headless tests only and is not a supported compatibility path.

## 5. Memory

The guest memory model follows the Series X|S unified-memory layout. Exact regions and sizes are not yet verified against a usable source. The 16 GiB figure from version 0.1.0 is not backed by a verified source and is removed. The current implementation is a flat 1 GiB arena with region classification only, without paging or fault handling. Sparse, host-backed paging is planned for phase 24F.

## 6. Scala runtime

The Scala runtime (JVM and Scala.js) and its legacy shims are outside the target scope. The legacy shims were written for Xbox 360-era code and are not part of the Series X|S path. Keeping or removing the runtime is an open decision for the project owner.

## 7. Configuration

Configuration uses Rust `serde` with TOML, lives at `~/.config/xholy/config.toml`, supports named per-game profiles, and is hot-reloaded via `notify`.

## 8. Performance goals (not measured)

| Target | Approach |
|---|---|
| Frame rate at 1440p and 4K | Native-speed guest execution plus D3D12 or Vulkan backend |
| Input latency below 15 ms | Low-latency present path, input polling independent of the frame loop |

These are goals. Measured values are added only when a benchmark produces them. `benchmarks/REPORT.md` covers the scaffold baseline only.

## 9. Non-goals

- Xbox 360 (PowerPC, XEX2) and Xbox One.
- ARM64 and armv7 hosts, including Raspberry Pi.
- macOS as a host.
- Compatibility claims without test evidence.

## 10. Repository and co-authorship

Apache-2.0 licensed. AI-agent co-authorship is recorded in commit history; see `CONTRIBUTING.md`, `Agents.md` and `.github/CITATION.cff`.

*Redeem Your Games.*