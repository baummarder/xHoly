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
  XAM/loader ABI; scheduler is cooperative snapshots, not preemptive.
- [ ] No JIT — interpreter only (a dynasm-style JIT is future work).
- [ ] Guest can't yet drive the wgpu GUI — no framebuffer→renderer path.
- [ ] GUI still on winit; `--native-wm` proves the C++ WM works but
  the main windowed GUI hasn't migrated to it.
- [ ] GPU translation covers a documented subset only — no LLVM/DXIL
  bitcode decode; the "game preview" renders a synthetic bitmap, not
  game frames
- [ ] Save states / load states are UI stubs (slot list only)
- [ ] Cheats are a toggle flag, no cheat engine
- [ ] Launch/play buttons log to stderr; nothing boots
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
