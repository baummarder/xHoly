# xHoly — Status

Honest feature checklist. xHoly is a **research scaffold**, not a
working Xbox emulator — see `docs/whitepaper.md` for the roadmap.

## Works

### Core (Rust)
- [x] Cargo project, lib + binary, CI workflow, benchmarks (criterion)
- [x] Minimal self-contained ARM64 interpreter (`src/core/cpu/arm64.rs`)
  with decoder, execution loop, embedded Hello-World program, tests
- [x] **Real AArch64 interpreter** (`src/core/cpu/interpreter.rs`) —
  actual ARM64 encodings: MOVZ/MOVN/MOVK, ADD/SUB (imm+shifted reg,
  NZCV), AND/ORR/EOR/ANDS, LDR/STR family, B/BL/B.cond/CBZ/RET/BRK,
  SVC→kernel. Subset only — no NEON/SIMD/sysregs.
- [x] **Xbox kernel stub** (`src/core/kernel/`) — VA memory layout,
  syscall table (Write/CreateThread/Sleep/WriteFile/ReadFile/Yield/
  Exit/Ticks), round-robin thread scheduler with context snapshots
- [x] **Game loader** (`src/core/loader.rs` + `src/core/loader/`)
  — flat/XEX-lite images, `xboxconfig.xml` parsing, `GameImage::
  load_into`; Phase-10 submodules: ISO game source (XBOX/GAME scan,
  executables/shaders/cover-art), XEX2 header parser, `ParsedGameImage`
  + `AssetCatalog`, `game_runner` (ISO→shader-compile→kernel exec)
- [x] **Shader translation pipeline** (`src/core/shader/`, Phase 10) —
  DXBC container parser (ISGN/OSGN sigs, bitcode extract), HLSL IR,
  hand-rolled SPIR-V 1.0 serializer, WGSL emitter, FNV-1a-keyed
  `.spv`/`.wgsl` cache, resource-binding model. Retail DXIL → blit
  fallback (no LLVM decode yet)
- [x] **Modular kernel** (`src/core/kernel/`, Phase 10) — memory
  layout, syscall table svc#0–16 (+Ticks=17), round-robin scheduler,
  GIC stub, MMIO regions, kernel console, heap/event/file/device
  object tables
- [x] **Execution glue** (`src/core/execution.rs`) — `--game PATH`
  runs a guest image on interpreter+kernel headless; `hello_world_program`
  assembler proves end-to-end ("Hello, xHoly!" via real SVC)
- [x] CPU translator stub (`src/core/cpu/translator.rs`)
- [x] Memory manager, config system (TOML, hot-reload, profiles)
- [x] Optical-drive subsystem: drive detection, ISO mounting,
  multi-disc grouping, game metadata scanning (`src/optical/`)
- [x] Audio driver abstraction, I/O/HID stubs
- [x] **Infrastructure stack** (Phase 11.5, 15 modules) — config
  loader/merge/hot-reload + infra sections, leveled logging with
  sinks/filters/rotation/spans, input state+mapping+deadzone+press
  machine, versioned save slots 1–8 + quick/auto + RLE + cloud-sync
  stub, mod manifests/deps/lifecycle, cheat values/triggers/manager/
  compiler (declarative, no Lua), benchmarks+profiler+reports,
  debug menu/overlay/panels/tools, cloud client/auth/cache/sync over
  injectable transport, FNV-1a shader cache + LRU manager, network
  transport/cache/proxy, anonymized batched telemetry, multiplayer
  lobby/state-sync/prediction stubs, storage devices/mounts/SSD
  cache, utils (hash/string/thread/semaphore). Cross-module coverage
  in `tests/infra_tests.rs`. Limits: no real network transport, no
  Lua, no wire netcode, RLE not gzip — see README §Infrastructure.

### GPU / rendering
- [x] wgpu 24 software preview pipeline (`src/core/gpu/`)
  — `bytes_to_bitmap` → RGBA software rasterizer → wgpu offscreen
- [x] DXIL→shader translation stubs (`src/gpu/translation`) +
  Phase-10 `core::shader` pipeline (container→IR→SPIR-V→WGSL→cache);
  retail DXIL still unsupported → blit fallback

### GUI
- [x] Ratatui/crossterm TUI (`--gui-tui`): game library from optical
  drives + `games/`, FPS/CPU/GPU readout, hotkeys
- [x] Windowed GUI (`--gui`): winit + wgpu, software-composited UI
  (sidebar, toolbar, preview, info + settings panels), mouse
  hit-testing, all hotkeys (ESC/F1/F5/F9/E/C/Q/F11/O)
- [x] Headless render mode (`--gui-headless`) for CI/tests
- [x] GPU API auto-detection: Vulkan → OpenGL → Software, persisted to
  `~/.config/xholy/gpu_config.toml`; live-detected Vulkan/RTX on dev
  machine (`docs/gpu-api-support.md`)
- [x] Frame pacing (`FramePacer`): VSync-aware present-mode selection,
  FPS cap, min/max/avg stats (`docs/framerate-fix.md`)
- [x] Config: `--fps-limit`, `--resolution`, `--fullscreen`,
  `--theme`, `--gpu-api`, `--frame-pacing`, `--no-vsync`

### Native / other
- [x] Scala/JVM + Scala.js runtime bridge skeleton (`scala/`, `build.sbt`)
- [x] C++17 Win32 window manager (`cpp/window_manager/`) — separate
  native module, pure `windows.h`, own CMake build + tests + demo
- [x] **C++ WM ↔ Rust FFI** — `extern "C"` API + `build.rs`/`cc` compile,
  `src/gui/native_wm.rs` safe wrappers, `--native-wm` demo flag
  (Windows-only; additive — winit GUI still default)
- [x] **GPU post-processing** (Phase 9) — `shaders/postfx.wgsl`:
  bright-pass + separable bloom blur (half-res) + composite with
  tone mapping (Reinhard/Filmic/ACES) + FXAA-lite; wired through
  `WgpuRenderer::set_postfx`, CLI `--bloom/--tone-mapping/--aa`
- [x] **Widget system** (`src/gui/widgets.rs`) — retained-mode tree,
  z-order, hover/press/drag, Clicked/Dragged events, framebuffer draw
- [x] **Texture atlas + ISO-offset loader + wgpu texture cache**
  (`src/gui/textures.rs`)
- [x] **Phase 13 JIT** (`src/jit/`) — x86-64 → WASM pipeline, pure
  Rust, no external deps: subset disassembler (REX.W, ModR/M reg-reg
  **and memory operands** mod 0/1/2 + SIB + disp8/disp32, mov/add/
  sub/xor/and/or/cmp/test/inc/dec/neg/not/push/pop/jmp/jcc/call/
  ret/syscall/nop/movzx/movsx, SSE2 integer subset: movd/movq/
  movdqa/movdqu/paddq/psubq/pand/por/pxor via xmm lane globals),
  dependency scheduler (mem ops + push/pop are barriers), linear-scan
  allocator, real WASM 1.0 binary writer, own WASM interpreter
  (i32/i64 locals+globals, memory, imports, select). Each x86 basic
  block → WASM func returning next guest offset; GPRs→i64 globals,
  flags→6×i32 globals (zf/sf/cf/of/pf/af — PF exact parity, AF aux
  carry), XMM→32×i64 lane globals, `syscall`→`xholy.syscall` host
  import.
- [x] **Phase 14 JIT integration** (`src/jit/emulator_loop.rs`,
  `src/jit/flags.rs`) — `JitEmulator` trait + `WasmJitEmulator`
  (compile→execute→exception-unwind), `JitConfig`, `JitFlags` EFLAGS
  model, `set_syscall_handler` host hook, and `X86Interpreter`: a
  scalar fallback executing the *same decoded subset* (mem operands,
  SIMD lanes, all flags) when the JIT fails or is disabled.
  `EmulationCore` gained `jit: Option<WasmJitEmulator>` +
  `compile_and_run(program)`; `--enable-jit` / `--jit-no-fallback`
  CLI flags; `Emulator::enable_jit()`.
  **Honest limits:** RIP-relative mod=0 rm=101 decodes as absolute
  disp32; SIMD is integer-lane only (no packed FP, no byte/word
  lanes); `wasmtime` not integrated — the interpreter executes the
  emitted subset.
- [x] **Phase 17 minimal guest game + real load path** — bundled
  `games/minimal/` fixture (`default.xex` XEX2 header + AArch64
  payload + `xboxconfig.xml`, regenerable via
  `cargo run --example gen_minimal_game`). `--game games/minimal`
  loads the XEX through the real path (loader → payload after
  `header_size` → `USER_BASE` → entry RVA → ARM64 interpreter →
  kernel SVC) and prints `Hello, xHoly!`; the TUI library lists it
  and Enter launches it into the on-screen console log.
  `xex_parser` gained section layout (`XexSection`) and an import
  resolver stub (`resolve_imports` → deterministic thunk VAs in the
  kernel region); `kernel::xam` is the matching XAM export stub
  table. The kernel gained a **pre-emptive scheduler tick**: the CPU
  calls back every `preempt_budget` instructions and the kernel
  rotates thread contexts mid-stream (in addition to cooperative
  SYS_YIELD). `GUEST_MEM_SIZE` now covers the heap arena so
  HeapAlloc writes land in mapped memory.
  **Honest limits:** the XEX reader handles only our minimal
  contiguous-payload layout — no retail section table, compression,
  encryption, or real import directory; XAM stubs return plausible
  constants, not console services; pre-emption is a budget tick on a
  single CPU, not SMP.
- [x] **Phase 18 guest framebuffer + windowed launch** — guest
  framebuffer aperture at `kernel::GUEST_FB_BASE` (160×120 RGBA8 +
  magic/w/h/seq header inside the user VA window). `execution::
  read_guest_frame` extracts it bounds-checked against both the flat
  buffer and the kernel memory-layer region table; `RunReport.frame`
  carries it to the GUI. The bundled `games/minimal` now writes a
  two-tone pattern ("x"/"y" halves) plus `Hello, xHoly!` in ~96k
  instructions. The windowed GUI composites the guest frame into the
  preview instead of the synthetic card, **holds the last frame** when
  the FB aperture is empty, shows guest output in a CONSOLE pane, and
  has a real LAUNCH toolbar button (+ Enter / preview click) running
  `game_runner::run_path` — same path as the TUI.
  **Honest limits:** one-shot run per launch (no persistent guest
  timeslice across GUI frames yet); 160×120 fixed guest FB; the
  windowed GUI still has no guest→GPU draw submission — the guest
  frame is CPU-composited like the rest of the UI.
- [x] **Phase 12 rendering additions** — `src/gui/windowed/
  frame_buffer.rs` (fused color+depth `FrameBuffer`, Load/Clear pass
  API, dark-Xbox clear color) and DXBC-style texture containers in
  `src/gui/textures.rs`: `DXBC` header parse, uncompressed RGBA8888
  passthrough, real BC1/DXT1 decode, BC7 → visible magenta dummy
  (not implemented). `tests/perf_tests.rs` bounds CPU-side costs
  (atlas pack <20 ms, 1080p tonemap, uniform math).
- [x] **Shader cache** (`src/gui/shader_cache.rs`) — validated WGSL
  copies + FNV manifest under the platform cache dir
- [x] Perf frame-drop logging (sustained <50% FPS limit → warning)
- [x] Branding: logo (gold X + crucifix), app/window icons, PNG
  exports, favicon
- [x] Docs: README, INSTALL, CONTRIBUTING, CITATION.cff, whitepaper,
  GPU-API + framerate docs, MISSING.md audit

## Does NOT work yet

- [ ] **No retail game runs** — the ARM64 interpreter covers a useful
  subset but real titles need NEON/SIMD, bitmask immediates,
  load/store pairs, sysregs, and a real XEX section/import parser.
  `--game` runs *small* ARM64 programs (hello-world works end-to-end).
- [ ] Kernel syscalls are xHoly-assigned numbers, not the real Xbox
  XAM ABI — `kernel::xam` stubs resolved imports but nothing services
  real XAM calls. Scheduler is pre-emptive by instruction budget but
  still single-CPU cooperative underneath.
- [ ] No *native* JIT — the x86-64→WASM JIT exists (`src/jit/`) but
  executes in a Rust WASM interpreter, not machine code; a
  dynasm-style JIT remains future work. The guest CPU loop still runs
  the ARM64-style interpreter; `compile_and_run` is the x86-64 entry.
- [ ] Guest frames reach the windowed preview via CPU compositing, but
  there is still no guest→GPU draw path (shaders, swapchain present of
  guest-rendered content); launches are one-shot runs, not sustained
  emulation sessions
- [ ] GUI still on winit; `--native-wm` proves the C++ WM works but
  the main windowed GUI hasn't migrated to it.
- [ ] GPU translation covers a documented subset only — no LLVM/DXIL
  bitcode decode; the "game preview" renders a synthetic bitmap, not
  game frames
- [ ] Save states / load states are UI stubs (slot list only)
- [ ] Cheats are a toggle flag, no cheat engine
- [ ] TUI + windowed Launch run the real game path and show console
  output + guest framebuffer frames; sustained multi-frame guest
  execution inside the GUI loop is still future work
- [ ] Audio output is a driver abstraction with no real stream
- [ ] Controller/HID input not wired to the GUI
- [ ] The Rust GUI and the C++ window manager are **not integrated** —
  the C++ module is standalone infrastructure
- [ ] Multi-window managed-app features (real secondary windows in the
  emulator UI) — the WM supports them, the GUI doesn't use them yet

## Tests

`cargo test --features cpu-stub` → all green (unit + GUI + GPU-API +
framerate integration tests). C++ side:
`cpp/window_manager` builds with MSVC + CMake/Ninja; its test binary
creates real HWNDs and passes on a desktop session.
