# xHoly — Status

Honest feature checklist. xHoly is a **research scaffold**, not a
working Xbox emulator — see `docs/whitepaper.md` for the roadmap.

## Works

### Core (Rust)
- [x] Cargo project, lib + binary, CI workflow, benchmarks (criterion)
- [x] Minimal self-contained ARM64 interpreter (`src/core/cpu/arm64.rs`)
  with decoder, execution loop, embedded Hello-World program, tests
- [x] CPU translator stub (`src/core/cpu/translator.rs`)
- [x] Memory manager, config system (TOML, hot-reload, profiles)
- [x] Optical-drive subsystem: drive detection, ISO mounting,
  multi-disc grouping, game metadata scanning (`src/optical/`)
- [x] Audio driver abstraction, I/O/HID stubs

### GPU / rendering
- [x] wgpu 24 software preview pipeline (`src/core/gpu/`)
  — `bytes_to_bitmap` → RGBA software rasterizer → wgpu offscreen
- [x] DXIL→shader translation stubs (`src/gpu/translation`)

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
- [x] Branding: logo (gold X + crucifix), app/window icons, PNG
  exports, favicon
- [x] Docs: README, INSTALL, CONTRIBUTING, CITATION.cff, whitepaper,
  GPU-API + framerate docs, MISSING.md audit

## Does NOT work yet

- [ ] **No actual Xbox game execution** — no x86→host CPU translation,
  no Xbox kernel/HAL, no game loading beyond optical scanning
- [ ] GPU translation is a stub — no real DXIL→SPIR-V pipeline; the
  "game preview" renders a synthetic bitmap, not game frames
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
