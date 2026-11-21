# xHoly Technical Whitepaper

*A hybrid Rust/Scala emulation architecture for Xbox Series S|X and Xbox One*

Version 0.1.0 — research scaffold

---

## 1. Motivation

Console emulation has converged on a few proven patterns: dynamic binary
translation (x86↔ARM), API translation layers (D3D→Vulkan/Metal), and
high-performance memory management. No open project today targets the full
Xbox One → Xbox Series S|X generation with a single coherent architecture.

xHoly explores a **hybrid-language** design: a Rust core for the
performance-critical emulation path, and a Scala/JVM runtime for game
content that benefits from managed execution and the JVM ecosystem, with a
Scala.js bundle for web deployment.

> **Honesty note:** this document describes the *target* architecture. The
> current codebase is a scaffold implementing subsystem boundaries and
> build plumbing; translator passes are stubs.

## 2. System overview

```
Game image ──► Loader ──► CPU translator ──► Memory manager
                              │                  │
                              ▼                  ▼
                        GPU translator      I/O subsystem
                        (DXIL→SPIRV/        (input, Kinect,
                         GLSL/WGSL)          storage, audio)
                              │
                              ▼
                     Backend: wgpu / software
                              │
                              ▼
                        Display + audio out

Scala runtime ◄──stdio JSON──► Rust host   (JVM/Scala.js game modules)
```

## 3. CPU translation

Xbox One (Jaguar, x86-64) and Series X|S (Zen 2, x86-64) share the same
ISA family, simplifying translation: the translator lowers guest x86-64
to host x86-64/aarch64 via a threaded-code or JIT pipeline.

Planned stages:

1. **Decoder** — x86-64 instruction decode (length, prefixes, ModRM/SIB).
2. **IR** — lift to a small SSA IR with explicit flags.
3. **Lowering** — emit host machine code (Cranelift-style) or threaded
   interpreter for cold paths.
4. **Block linking** — direct-block chaining for hot loops.

The scaffold ships an interpreter skeleton over a minimal opcode subset
so the memory/GPU/timing pipeline can be exercised deterministically.

## 4. GPU translation

Xbox titles render through Direct3D 12 on RDNA2 (Series X|S) or AMD GCN
(Xbox One). The translation layer:

- **Shader bytecode**: DXIL/DXBC → SPIR-V (Vulkan), GLSL (OpenGL), or WGSL
  (WebGPU/Metal). Ray-tracing, mesh/amplification shaders, and variable
  rate shading are first-class `ShaderStage`/`ShaderFeatures` types.
- **Command stream**: D3D12 command lists → backend render passes.
- **Fallback**: a software rasterizer (barycentric triangle path exists)
  for headless/CI and Pi-class hosts.

## 5. Memory

Guest memory mimics the Xbox kernel layout: a flat addressable window
with reserved regions for hypervisor, GPU carve-out, title memory, and an
I/O aperture (`src/core/memory.rs`). The scaffold uses a 1 GiB resident
arena; the production design calls for sparse/paged allocation over the
full 16 GiB address space.

## 6. Scala runtime & compatibility

`scala/` is an sbt `crossProject` (JVM + Scala.js):

- **JVM runtime** (`runtime.Main`): hosts JVM-based game modules,
  exchanges newline-delimited JSON with the Rust host over stdio
  (`BridgeMessage` protocol in `src/runtime/scala_bridge.rs`).
- **Scala.js bundle**: browser deployment shell.
- **Legacy shims**: `GameLoader.loadLegacy` stubs the compatibility path
  for older titles (Xbox 360-era managed code, arcade titles).

## 7. Configuration

Hybrid schema: Rust `serde` + TOML on the host side; Scala case classes
mirror the same fields. Config lives at `~/.config/xholy/config.toml`,
supports named per-game profiles, and is hot-reloaded via `notify`.

## 8. Performance plan

| Target | Approach |
|---|---|
| 60 FPS @ 4K | wgpu backend + async compute; VRS and mesh shaders where available |
| <15 ms latency | Frame pipelining, low-latency present, 1000 Hz input polling |
| Pi 4 support | Software rasterizer + threaded interpreter fallback tier |

## 9. Repository & co-authorship

Apache-2.0 licensed. AI-agent co-authorship is recorded in commit history; see
`CONTRIBUTING.md` and `.github/CITATION.cff`.

*Redeem Your Games.*
