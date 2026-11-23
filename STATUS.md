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
- [x] GPU API auto-detection (Phase 21): platform order — Windows →
  D3D12, Linux → Vulkan, macOS → Metal — then wgpu's other backends and
  Software as fallbacks; persisted to
  `~/.config/xholy/gpu_config.toml`; live-detected Vulkan/RTX on dev
  machine (`docs/gpu-api-support.md`)
- [x] Frame pacing (`FramePacer`): VSync-aware present-mode selection,
  FPS cap, min/max/avg stats (`docs/framerate-fix.md`)
- [x] **Phase 21 platform GPU decision** (`src/gui/gpu_api_detector.rs`,
  `src/gui/native_gpu_backend.rs`) — `GpuPlatform` (Windows/Linux/macOS)
  → primary `GpuApi` (D3D12/Vulkan/Metal) with a deterministic fallback
  chain ending in Software; `PlatformBackend` / `NativeGpuBackend` map
  the native API to wgpu's matching backend bits (DX12/VULKAN/METAL)
  and can create a backend-restricted `wgpu::Instance`. CLI/config
  accept `d3d12`/`dx12`, `vulkan`, `metal`, `opengl`, `software`. The
  decision is tested for all three platforms.
  **Honest limits:** no native D3D12/Metal device or command-queue
  layer is linked — the "native" backends are wgpu backend bits, and
  the windowed renderer still composites guest frames on the CPU.
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
- [x] **Phase 19 real XEX section layout** (`src/core/loader/xex_parser.rs`) —
  real XEX2 files are no longer ignored: `parse_xex` decodes the
  **big-endian** 0x18-byte fixed header (`security_offset`,
  `header_count`) and the optional-header `(key, value)` directory with
  Xenia-compatible key IDs. Entry point / image base come from the
  directory (`XEX_HEADER_ENTRY_POINT`, `..._IMAGE_BASE_ADDRESS`), title
  id and media id from execution info, image size from the security
  block. `XexHeader::sections()` now reconstructs `.text` / `.data` /
  `.rdata` from the security page descriptors (RVA = page index × page
  size, 4 KiB or 64 KiB per `image_flags`), with a contiguous `.text`
  fallback for xHoly's legacy little-endian fixture. `is_retail_layout()`
  additionally recognizes a parsed directory. `loader::image_from_bytes`
  consequently strips the correct (big-endian) header size and resolves
  the entry RVA from real headers instead of rejecting them.
  **Honest limits:** no AES decryption, no basic/normal LZX
  decompression, no embedded PowerPC PE section table, no real import
  directory; `raw_offset` is an uncompressed-layout estimate, and the
  guest CPU still cannot execute PowerPC/FX code. Retail titles do not
  boot — this closes only the loader/header gap.
- [x] **Phase 22 bootable minigame + persistent guest loop** —
  `games/minigame/default.xex` (regenerable via
  `cargo run --example gen_minigame_game`) is a **real big-endian XEX2**
  whose AArch64 payload runs a game loop: timer (`SYS_TICKS`), host input
  from the new guest input aperture (`GUEST_INPUT_BASE`), framebuffer
  sequence bump, a moving coloured row, then `SYS_FRAME_END` to yield.
  `GuestSession` (`core::execution`) owns CPU + kernel across frames;
  `Arm64Cpu::run_slice` resumes at the saved PC, keeps registers/memory/
  ticks, writes host input before each slice and holds the last frame.
  GUI Launch now creates a session and `tick_guest()` advances it once
  per GUI frame; held keyboard keys map to guest buttons. Also fixed a
  real interpreter bug: unconditional branches sign-extended imm26 as
  28 bits, turning backward jumps into far-forward jumps.
  **Honest limits:** 160×120 FB, one painted row per frame, left/right/
  up/down/A only, no sustained audio/HID or retail PowerPC titles.
- [x] **Phase 23 direct guest→GPU path** — `WgpuRenderer::render_guest_frame`
  uploads guest RGBA straight into a wgpu texture and blits it with the
  GUI shader; while a guest runs, `WindowApp::render_frame` uses this
  path instead of CPU-compositing the guest frame into the UI. The last
  frame is held by `GuestSession` when the guest writes nothing, and
  the framebuffer/input apertures are bounds-checked against the kernel
  memory-layer region table plus the flat guest buffer.
  **Honest limits:** the guest FB is scaled by the blit shader (no
  guest draw-call translation), and the UI overlay is suppressed while
  a guest is running.
- [x] **Phase 20 XEX2 image decode** (`src/core/loader/image_decode.rs`) —
  the retail payload is now decoded instead of mapped verbatim:
  AES-128-CBC with zero IV (Xenia `aes_decrypt_buffer`, not CTR) using
  the session key from security info +0x150 (retail/devkit two-key
  probe), BASIC data/zero-run expansion, LZX-normal de-blocking
  (u32 block size + 20-byte digest + u16 chunk framing) and a pure-Rust
  **regular LZX** decompressor (32 KiB frames, verbatim/aligned/
  uncompressed blocks, canonical Huffman trees, R0/R1/R2 offsets,
  Intel E8 fixups). `image_from_bytes` routes real XEX2 through this
  path; legacy scaffold images stay raw. Verified with NIST FIPS-197 /
  SP 800-38A AES vectors, the known libmspack uncompressed-block
  stream, a self-built verbatim block, and synthetic XEX fixtures.
  **Honest limits:** LZX block SHA1 digests are parsed but not verified;
  no LZX DELTA patch application; 4 KiB-page LZX is clamped to a 32 KiB
  window; the embedded PowerPC PE, imports and system flags are still
  not interpreted, so retail titles still do not boot.
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
  load/store pairs, sysregs and a real import parser. Phases 19/20
  added real XEX2 header/section parsing plus AES/LZX image decoding;
  the embedded PowerPC PE and imports remain stubbed. `--game` runs
  *small* ARM64 programs (hello-world works end-to-end).
- [ ] Kernel syscalls are xHoly-assigned numbers, not the real Xbox
  XAM ABI — `kernel::xam` stubs resolved imports but nothing services
  real XAM calls. Scheduler is pre-emptive by instruction budget but
  still single-CPU cooperative underneath.
- [ ] No *native* JIT — the x86-64→WASM JIT exists (`src/jit/`) but
  executes in a Rust WASM interpreter, not machine code; a
  dynasm-style JIT remains future work. The guest CPU loop still runs
  the ARM64-style interpreter; `compile_and_run` is the x86-64 entry.
- [ ] Guest frames now reach the GPU directly (Phase 23), but there is
  still no guest→GPU **draw-call** translation (shaders/swapchain
  submitted by the guest); the guest paints into a fixed 160×120 FB that
  the host scales. Sessions are persistent across frames (Phase 22),
  not one-shot, but the GUI overlay is hidden while a game runs
- [ ] GUI still on winit; `--native-wm` proves the C++ WM works but
  the main windowed GUI hasn't migrated to it.
- [ ] GPU translation covers a documented subset only — no LLVM/DXIL
  bitcode decode; the "game preview" renders a synthetic bitmap, not
  game frames
- [ ] Save states / load states are UI stubs (slot list only)
- [ ] Cheats are a toggle flag, no cheat engine
- [ ] TUI + windowed Launch run the real game path and show console
  output + guest framebuffer frames; the windowed GUI now sustains
  multi-frame guest execution (Phase 22), the TUI Launch is still a
  one-shot `game_runner::run_path`
- [ ] Audio output is a driver abstraction with no real stream
- [ ] Controller/HID input not wired to the GUI — keyboard buttons and
  mouse coordinates reach the guest input aperture, but no real
  gamepad/XInput device is connected yet
- [ ] The Rust GUI and the C++ window manager are **not integrated** —
  the C++ module is standalone infrastructure
- [ ] Multi-window managed-app features (real secondary windows in the
  emulator UI) — the WM supports them, the GUI doesn't use them yet

## Tests

`cargo test --features cpu-stub` → all green (unit + GUI + GPU-API +
framerate integration tests). C++ side:
`cpp/window_manager` builds with MSVC + CMake/Ninja; its test binary
creates real HWNDs and passes on a desktop session.

## Empfhlung (OpenCode, 2026-11-23)

Phase 19 macht Retail-XEX2-Header sichtbar, aber weiterhin nicht
ausführbar. Als nächster Schritt würde ich die **Image-Dekodierung**
(File-Format-Info: `XEX_ENCRYPTION_NORMAL` + `XEX_COMPRESSION_BASIC`)
angehen, nicht NEON oder Pre-emption: Sie baut direkt auf dem neuen
Security-Block/Page-Descriptor-Layout auf, ist mit synthetischen
Fixtures deterministisch testbar und schließt die Lücke zwischen
„Header gelesen" und „Image im Guest-Speicher". Begründung: ohne
dekomprimiertes Bild bleibt jede weitere CPU-Arbeit wirkungslos;
NEON/SIMD setzt ein lauffähiges Image voraus, und Pre-emption
(Timer/IRQ + SMP) sowie ein Guest→GPU-Draw-Pfad sind architektonisch
größer bzw. kosmetisch, solange das Guest-Frame schon im Preview
ankommt. Ehrliche Grenze: auch danach booten keine Retail-Titel —
der PowerPC-Befehlssatz fehlt weiterhin.

## Empfhlung (OpenCode, 2026-11-23) — Bild-Dekodierung

Phase 20 schließt die Decoder-Lücke: aus einem Retail-XEX2 wird jetzt
ein echtes, dekomprimiertes Guest-Image statt eines rohen, verschlüsselten
Blobs. Ich habe bewusst die Image-Dekodierung vor der GPU-Arbeit gemacht,
weil ohne dekodiertes Bild jeder spätere Renderpfad nur ein synthetisches
Bitmap zeigt. Begründung: AES/BASIC/LZX sind deterministisch und gegen
NIST- sowie libmspack-Testvektoren prüfbar; die GPU-Entscheidung ist
plattformabhängig und lässt sich sauber in einem eigenen Schritt mit
reinen Entscheidungstests abhandeln. Ehrliche Grenze: der dekodierte
PowerPC-PE wird nicht interpretiert und die SHA1-Blöcke werden nicht
verifiziert — Retail-Titel booten weiterhin nicht; es schließt nur die
Decoder-Lücke.

## Empfhlung (OpenCode, 2026-11-23) — GPU-Architektur

Phase 21 trifft die Plattformentscheidung (Windows→D3D12, Linux→Vulkan,
macOS→Metal) und führt wgpu-Software-Fallbacks als geordnete Kette ein;
die Entscheidung ist plattformunabhängig getestet. Ich habe sie nach der
Dekodierung gemacht, damit die GPU-Wahl auf einem echten Image-Pfad
aufbaut. Begründung: Eine native D3D12/Metal-Geräteschicht wäre ein
viel größeres FFI-Projekt und würde am aktuellen CPU-Compositing nichts
ändern — die wgpu-Backend-Bits liefern dieselbe Entscheidung mit
prüfbarem Verhalten und klaren Grenzen. Als Nächstes würde ich den
Guest-Frame endlich als wgpu-Texture hochladen statt ihn in den UI-
Framebuffer zu blitten; das ist der sichtbare Gewinn, den die
Plattformentscheidung vorbereitet. Ehrliche Grenze: Retail-Titel booten
weiterhin nicht (NEON, Bitmask-Immediates, LDP/STP, Sysregs fehlen) —
es schließt nur die GPU-Entscheidungslücke.

## Empfhlung (OpenCode, 2026-11-23) — Spielbares Gast-Spiel

Phasen 22/23 machen aus dem Scaffold zum ersten Mal etwas Spielbares:
ein echtes XEX2 bootet, läuft als persistenter Frame-Loop, reagiert auf
gehaltene Tasten und zeigt seine Frames direkt auf der GPU. Ich habe
zuerst das Image und den Session-Loop gebaut und erst danach den GPU-
Pfad, weil ein direkter Blit ohne laufendes Guest-Frame wertlos wäre.
Begründung: der Input-Apertur-Ansatz braucht keine neue Device-FFI, ist
vom Guest mit normalen `LDR`-Befehlen lesbar und damit deterministisch
testbar; der `SYS_FRAME_END`-Yield hält die Guest-Zeit über Frames hinweg,
ohne Threads oder VM-Snapshots einzuführen. Dabei fiel ein echter
Interpreter-Fehler auf (imm26-Fortzeichen), der jetzt behoben ist. Als
Nächstes würde ich die TUI auf dieselbe `GuestSession` umstellen und
den 160×120-FB auflösungsunabhängig machen. Ehrliche Grenze: Retail-Titel
booten weiterhin nicht (NEON, Bitmask-Immediates, LDP/STP, Sysregs
fehlen) — es schließt nur die Decoder-, Loop- und GPU-Lücke.
