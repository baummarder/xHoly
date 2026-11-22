# Phase 10 — Game-Shader-Translation (DXIL → SPIR-V → WGSL) + Kernel-Integration

Phase 10 adds the two "middle layers" the earlier phases stubbed out:
a **shader translation pipeline** (DXBC/DXIL container → IR → SPIR-V →
WGSL, cached) and a **modular kernel** (memory layout, syscall table,
scheduler, interrupt stub, MMIO, console) plus a **game runner** that
ties ISO → image → shader compile → kernel execution together.

Everything here is a *research subset*: the container/signature parsing,
IR, SPIR-V serialization, WGSL emission, caching, binding model, and all
kernel machinery are real code with tests — but retail DXIL bitcode is
**not** decoded (LLVM bitcode decoding is out of scope) and the syscalls
are xHoly-assigned numbers, not the Xbox XAM ABI.

## Shader pipeline (`src/core/shader/`)

```text
DXBC container ──► dxil_parser ──► DxilAst (hash, ISGN/OSGN sigs, bitcode)
      │                                        │
      │                                   hlsl_compiler::lower
      │                                        ▼
      │                                   HlslIr (Node tree)
      │                                        │
      │              ┌─────────────────────────┼──────────────────┐
      │              ▼                         ▼                  ▼
      │        spirv_generator            wgsl_emitter      shader_cache
      │        (SPIR-V 1.0 words)         (WGSL text)       (FNV-1a keyed
      │              │                         │             .spv/.wgsl)
      │              └────────► ShaderArtifact ◄┘
      │
      └── shader_binder: ResourceTable/ResourceBinding for
          uniforms, textures, samplers, storage buffers
```

- **`dxil_parser.rs`** — parses the DXBC container: checksum, part
  table, `HASH`/`ISGN`/`OSGN`/`SFI0`/`DXIL`/`RDAT`/`STAT` fourcc parts.
  ISGN/OSGN signature blobs are decoded into `SignatureParam`s
  (name, semantic, component type). The DXIL part's LLVM bitcode is
  extracted as raw bytes for the next stage.
- **`hlsl_compiler.rs`** — lowers bitcode to `HlslIr`, a small AST
  (`Const`, `Add`, `Mul`, `SampleTexture`, `VertexId`, `InputParam`).
  Supported input is a documented **host-generated mini-format**
  (`XHLS` magic + stage byte + encoded nodes) used by tests/tools.
  Retail LLVM bitcode → `Err(Unsupported("llvm-bitcode"))`, which the
  cache layer turns into a WGSL blit fallback with a warning — never a
  silent wrong shader.
- **`spirv_generator.rs`** — hand-rolled SPIR-V 1.0 serializer for the
  IR subset: capability/memory-model/entry-point, type decls
  (void/f32/vec2-4/pointer/function), `OpConstant`/`OpFAdd`/`OpFMul`,
  function skeleton. Produces a *structurally valid* module — it does
  not yet emit interface blocks (`gl_PerVertex`, output variables), so
  treat it as serialization + cache-key material; the WGSL emitter is
  what wgpu actually consumes.
- **`wgsl_emitter.rs`** — IR → WGSL source. Emits `vs_main`/`fs_main`
  functions with translated bodies, plus `blit_wgsl()` fallback.
- **`shader_cache.rs`** — `GameShaderCache` keyed by
  `key = hash(dxil_bitcode ‖ bindings_sig)`. Artifacts stored as
  `<key>.wgsl` + `<key>.spv` under the platform cache dir
  (`%LOCALAPPDATA%\xHoly\cache\shaders\game\` on Windows,
  `$XDG_CACHE_HOME/xholy/shaders/game/` elsewhere). `compile_shader()`
  is the one-call pipeline: DXBC unwrap → lower → emit → SPIR-V →
  cache put/get.
- **`shader_binder.rs`** — `ResourceTable`/`ResourceBinding` model:
  uniforms, textures, samplers, storage buffers with bind group / set /
  binding indices and WGSL type names. Pure data model (no wgpu
  objects) so it stays testable headless; wgpu bind-group creation is
  a follow-up.

## Kernel (`src/core/kernel/`)

The Phase-8 monolith is split into focused modules; `mod.rs` keeps the
public API (`Kernel`, `SvcCall`, `SYS_*` constants, `Thread`) stable.

| Module | Contents |
|--------|----------|
| `memory.rs` | `KernelMemoryLayout`: kernel/user/device/paged-pool regions, `virt_to_phys` sanity mapping, region classification. Simulated — no real MMU. |
| `syscall.rs` | `KernelObjects`: heaps (bump allocator over `HEAP_BASE` arena), events, files, devices, suspended-thread sets; handle allocation. |
| `scheduler.rs` | `Thread` context (regs/sp/pc/flags/state/sleep_ticks), `age_sleepers`, `next_ready`, `round_robin`, `state_counts`. |
| `interrupt.rs` | `Gic` stub: 64 IRQ lines, enable/raise/lower, periodic timer IRQ, `pending()`/`ack()`. |
| `mmio.rs` | `XboxIoSpace`: GPU framebuffer, texture, audio, storage, display regions with read/write dispatch. |
| `console.rs` | `KernelConsole`: debug log, `klog`, panic-style messages. |

### Syscall table (svc numbers, x0 = number, x1..x7 = args)

| svc | Name | svc | Name |
|-----|------|-----|------|
| 0 | Write | 9 | Resume |
| 1 | CreateThread | 10 | CreateHeap |
| 2 | CreateProcess | 11 | HeapAlloc |
| 3 | Sleep | 12 | HeapFree |
| 4 | WriteFile | 13 | CreateEvent |
| 5 | ReadFile | 14 | OpenFile |
| 6 | Yield | 15 | CreateDevice |
| 7 | Exit | 16 | SetContext |
| 8 | Suspend | 17 | Ticks (xHoly extension) |

Notes:
- Phase-8 had `SYS_TICKS = 8`; the Phase-10 table assigns 8 to Suspend,
  so Ticks moved to **17**. `SYS_EXIT`/`SYS_SLEEP`/etc. keep their
  Phase-8 values — existing tests updated accordingly.
- Sleep timers age once per syscall tick (`age_sleepers` in
  `handle_syscall`), not per context switch, so `Sleep(n)` sleeps n
  ticks.
- Heaps are bump allocators — `HeapFree` drops bookkeeping but does
  not reclaim arena space (documented limitation).

## Loader (`src/core/loader/`)

`loader.rs` stays the flat/XEX-lite entry point and now declares
submodules:

- **`iso_parser.rs`** — `IsoGameSource` wraps the Phase-4
  `Iso9660Image`: `find_executables()` (XBOX/*.xex, XBOX/GAME, GAME),
  `find_shaders()` (.cso/.dxbc/.dxil/.shad), `find_cover_art()`,
  `disc_hints()` for multi-disc names, `inspect_iso()` summary.
- **`xex_parser.rs`** — `parse_xex()` → `XexHeader`: module flags,
  header size, timestamp, media id, title id (0x184), entry point
  (0x104), image base (0x108), image size. Retail optional-header
  directory / PE sections are *not* reconstructed.
- **`game_image.rs`** — `ParsedGameImage` (title, `GameRegion`,
  `Vec<DiscInfo>`, `AssetCatalog`, entry/image base, title id,
  release date) per the Phase-10 spec structure.
- **`game_runner.rs`** — `run_path()` auto-detects ISO (>34 sectors)
  vs. flat/dir; `run_iso()` extracts the executable + asset catalog
  from the disc, compiles discovered shader blobs through
  `compile_shader` (first 8, warnings collected), then runs
  `execution::execute_game`. `run_flat()` wraps the Phase-8 path.

## Shaders (`shaders/`)

- `postfx.wgsl` — Phase-9 post-processing (unchanged).
- `render.wgsl` — default fullscreen bit-bitmap → gold-on-dark path
  used by `core::gpu::wgpu_backend` (restored gold-mix after a
  placeholder rewrite broke the R8 contract).
- `ui.wgsl` — UI blit placeholder for the windowed GUI.

## Tests

- `dxil_parser`: DXBC container round-trip (ISGN + DXIL parts),
  non-DXBC rejection.
- `hlsl_compiler`: mini-format encode/lower for vertex+fragment,
  unsupported LLVM bitcode error.
- `spirv_generator`: header/bound sanity, OpFAdd/OpFMul presence,
  unsupported-node errors.
- `wgsl_emitter`: stage emission + blit fallback.
- `shader_cache`: hit/miss keying, unsupported→blit fallback artifact.
- `shader_binder`: resource table/binding model.
- `kernel::memory`: region layout/classification.
- `kernel`: write+sleep syscall, round-robin, exit-halts,
  suspend/resume, heap alloc/free, event/file/device handles.
- `scheduler`: rotation, sleep aging/wake, suspended exclusion.
- `interrupt`: timer periodicity, enable/pending/ack.
- `mmio`: region dispatch.
- `loader`: XEX2 header parse/reject, ISO source reject,
  `run_path` flat execution, `game_image` region/catalog units.
- `game_execution_tests.rs`: updated syscall-number assertions
  (Suspend=8, SetContext=16, Ticks=17).

## Honest limitations

- **No real DXIL decoding** — LLVM bitcode → `Unsupported` → blit
  fallback. Full DXC/LLVM integration is future work.
- **SPIR-V is a subset serializer** — no interface blocks, no real
  engine yet; WGSL is the consumed format.
- **No retail XEX** — header fields only; no sections/imports/TLS.
- **No real Xbox ABI** — syscall numbers/args are xHoly's, not XAM.
- **No MMU/page faults** — memory layout is classification only.
- **No real GIC** — IRQ lines are a stub; guest can't take exceptions.
- **No NEON/SIMD, no JIT** — interpreter subset only (Phase 8).
- **Bump heaps** — no reclamation.
- **ISO path executes the raw XEX bytes flat** — section relocation
  not performed; only tiny hand-built images actually run.
