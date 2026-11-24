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