<p align="center">
  <img src="xHoly-logo.svg" width="160" alt="xHoly logo"/>
</p>

<p align="center"><b>Redeem Your Games</b></p>

# xHoly — Xbox Series X|S HLE Emulator (Research)

xHoly is a research high-level emulator (HLE) for **Xbox Series X|S** (Gen9) executables. It runs on **Windows x86-64** and **Linux x86-64**, built on a Rust core with an optional Scala runtime.

> **Status: experimental research scaffold.** No retail title runs yet. The binding target and architecture decisions are in [ROADMAP.md](ROADMAP.md) and [docs/adr/0001-target-architecture.md](docs/adr/0001-target-architecture.md).

- Primary target: Xbox Series X|S (ADR 0001). Xbox One support is a stated project goal; no Xbox One-specific implementation exists yet.
- Not supported: Xbox 360 (legacy code only, feature-gated), ARM64 hosts, macOS.

## Project Independence

xHoly is an independent, community-oriented open-source emulator research project.

It is not affiliated with, endorsed by or sponsored by Microsoft, Xbox, Rockstar Games or Take-Two Interactive.

## Experimental Status

xHoly is experimental software under active development.

Retail game compatibility is not guaranteed. Only explicitly tested functionality may be described as working. The progress figures in this README measure roadmap tasks, not game support.

## Microsoft SDK Independence

The architectural goal is to build and run xHoly without requiring an installed Microsoft Xbox GDK or GDKX.

Xbox functionality should be implemented through independently developed HLE components and documented interfaces.

This is a goal, not an audited claim: complete SDK independence has not yet been verified against the project's dependency and build requirements.

## Graphics Architecture

The project aims for a cross-vendor graphics architecture supporting NVIDIA and AMD host GPUs. Host rendering goes through wgpu (Vulkan, OpenGL or a software adapter, auto-detected with `--gpu-api`); Vulkan may be evaluated as a cross-platform backend.

No full Xbox shader compatibility and no working guest GPU translation are claimed. Shader work (DXIL parsing, SPIR-V/WGSL emission) is partial and has not been demonstrated with retail titles.

## Platforms

| Platform | State |
|---|---|
| Windows x86-64 | **Implemented and tested** on the development host: unit tests, integration tests, windowed GUI and Discord presence were executed here. |
| Linux x86-64 | **Build target, partially tested**: the CI workflow (`.github/workflows/ci.yml`, Ubuntu) runs `cargo test` and `cargo clippy`. Not executed in this milestone's local verification. |
| ARM64 hosts, macOS | **Untested and unsupported.** |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        xHoly Host                       │
│                                                          │
│  ┌────────────┐   ┌──────────────┐   ┌───────────────┐  │
│  │   Rust     │   │    Scala     │   │   Scala.js    │  │
│  │   Core     │◄─►│   Runtime    │   │   Web Bundle  │  │
│  │            │   │   (JVM)      │   │               │  │
│  │ • CPU xlat │   │ • GameLoader │   │ • Browser UI  │  │
│  │ • Memory   │   │ • JVM game   │   │ • WASM-ish    │  │
│  │ • GPU xlat │   │   modules    │   │   deployment  │  │
│  │ • Config   │   │ • Legacy     │   │               │  │
│  │ • I/O,HID  │   │   shims      │   │               │  │
│  └────────────┘   └──────────────┘   └───────────────┘  │
│         │                  │                            │
│         └──────────┬───────┘                            │
│                    ▼                                    │
│            Guest (Xbox) memory space                    │
└─────────────────────────────────────────────────────────┘
```

| Component | Language | Purpose |
|---|---|---|
| Core emulation engine | Rust | CPU translator, memory allocator, frame loop |
| GPU translation layer | Rust | DXIL parsing, SPIR-V/WGSL emission (partial); host frames via wgpu — no guest draw-call translation |
| Memory management | Rust | Xbox-kernel-style flat guest address space |
| Audio driver | Rust | WASAPI / Core Audio / ALSA abstraction |
| I/O subsystem | Rust | Controllers, HID layer, Kinect, storage |
| Game runtime loader | Scala | JVM game modules, stdio JSON bridge |
| Web deployment | Scala.js | Browser bundle |
| Configuration | Rust + Scala | TOML persistence, hot-reload, profiles |

## Repository layout

```
xHoly/
├── Cargo.toml               # Rust workspace manifest
├── build.sbt                # Root sbt build
├── xHoly-logo.svg           # Logo (bold X + crucifix, gold on dark)
├── .github/
│   ├── CODEOWNERS
│   ├── CITATION.cff
│   ├── CONTRIBUTORS.md
│   └── workflows/ci.yml
├── CONTRIBUTING.md
├── INSTALL.md               # Prerequisites & setup guide
├── MISSING.md               # Audit of components still to implement
├── src/                     # Rust crate
│   ├── lib.rs  main.rs
│   ├── core/      (cpu, memory, emulation)
│   ├── gpu/       (shader, translation, passthrough)
│   ├── audio/     (driver)
│   ├── io/        (input, hid, kinect, storage, screenshot)
│   ├── runtime/   (scala_bridge)
│   └── config/    (settings)
├── scala/                   # sbt crossProject (JVM + Scala.js)
│   ├── shared/  jvm/  js/  project/
├── cpp/window_manager/      # Native C++17 Win32 window manager (see its README)
├── benchmarks/              # Criterion benches
└── docs/
    ├── whitepaper.md        # Hybrid-architecture technical paper
    ├── gpu-api-support.md   # GPU backend recommendations
    └── framerate-fix.md     # Frame pacing / VSync notes
```

## Building

### Rust core

```sh
cargo build --release
cargo test
cargo bench
```

Produces a single `xholy` binary. Supported hosts: Windows x86-64 and Linux x86-64. ARM64 and armv7 are not supported.

### Scala runtime

```sh
cd scala
sbt assembly        # JVM fat jar → target/scala-*/xholy-runtime.jar
sbt fullOptJS       # Scala.js bundle → target/scala-*/xholy-web-opt.js
```

The Rust host locates these artifacts automatically (see
`src/runtime/scala_bridge.rs`).

### Optical drives

```sh
xholy --drives                              # detect physical drives
xholy --mount "D:\\" --list                  # scan a physical disc
xholy --mount "D:\\Games\\mygame.iso" --list # scan an ISO image
xholy --mount game.iso --mount game-disc2.iso game.xex   # multi-disc
```

### GUI

```sh
xholy --gui                 # windowed GUI (winit + wgpu, auto GPU API)
xholy --gui-tui             # terminal UI (ratatui)
xholy --gui-headless        # offscreen render loop (no window)
xholy --gpu-api vulkan      # force Vulkan (auto|vulkan|opengl|software)
xholy --fps-limit 120       # cap GUI framerate (0 = unlimited)
xholy --resolution 1280x720 # window size
xholy --fullscreen          # start fullscreen
xholy --theme gold          # dark | gold | light
xholy --frame-pacing auto   # auto | fixed | uncapped
xholy --no-vsync            # disable VSync (manual pacing takes over)
xholy --renderer vulkan     # alias for --gpu-api
xholy --bloom               # bloom post-FX (default on; --no-bloom off)
xholy --tone-mapping aces   # linear | reinhard | filmic | aces
xholy --aa fxaa             # off | fxaa | msaa4
xholy --game game.bin       # run AArch64 test image headless (Phase 8; test harness, not an Xbox target)
xholy --game games/minimal  # bundled minimal guest (XEX2 + xboxconfig)
xholy --native-wm           # native C++ WM demo window (Windows)
```

#### Minimal guest game (Phase 17, test fixture)

This AArch64 fixture tests the loader and kernel pipeline. It is not Xbox guest code.

`games/minimal/` is a real on-disk guest image: `default.xex` carries a
little-endian `XEX2` header (entry RVA, image base, title-id quad) followed
by an AArch64 payload that writes a two-tone pattern into the guest
framebuffer aperture (`kernel::GUEST_FB_BASE`, 160×120 RGBA8), prints
`Hello, xHoly!` through the kernel's SYS_WRITE and exits via SYS_EXIT;
`xboxconfig.xml` supplies the title and TitleID. The loader strips the
header, maps the payload at `USER_BASE`, and the ARM64 interpreter runs
it end-to-end:

```sh
xholy --game games/minimal          # prints "Hello, xHoly!"
xholy --game games/minimal/default.xex   # same, direct XEX file
xholy --mount games --list          # lists "xHoly Minimal Guest"
xholy --gui-tui                     # Enter on the game launches it;
                                    # output lands in the Console pane
xholy --gui                         # LAUNCH button (or Enter / preview
                                    # click) runs the same path; the guest
                                    # framebuffer frame is blitted into the
                                    # preview and output shows in CONSOLE
```

Regenerate the fixture with `cargo run --example gen_minimal_game`.
The XEX reader handles only this contiguous-payload layout — retail
section tables, compression, encryption and real import directories are
not implemented (see STATUS.md).

### Framerate

- **Default:** VSync on (`PresentMode::Fifo`) + `fps_limit` cap — the
  loop holds the configured rate via `FramePacer`.
- **`--fps-limit N`** caps the GUI at N FPS; `0`/absent = uncapped.
- **`--frame-pacing`**: `auto` (VSync→Fifo, otherwise manual sleep),
  `fixed` (always manual pacing), `uncapped` (no limit).
- **`--no-vsync`** switches the surface to `Immediate`/`Mailbox` and
  lets `FramePacer` pace manually — use for benchmarking.
- See `docs/framerate-fix.md` for the diagnosis and
  `STATUS.md` for an honest feature checklist.

The GUI auto-detects the GPU API (Vulkan → OpenGL → Software) and
persists the result to `~/.config/xholy/gpu_config.toml`. See
`docs/gpu-api-support.md` for per-vendor API recommendations.

**Troubleshooting:** if the windowed GUI fails with "no GPU API
available", use `--gpu-software` (CPU renderer, works anywhere) or
`--gui-tui` (terminal UI). Press `O` in the windowed GUI for the
settings panel. If FPS is capped unexpectedly, check `--vsync`/`--no-vsync`
and `--frame-pacing`.

### Running

```sh
./target/release/xholy /path/to/game.xvc
./target/release/xholy game.iso --max-frames 600 --config ~/.config/xholy/config.toml
```

### Development reports (read-only)

```sh
xholy --progress              # roadmap progress derived from progress.json
xholy --analyze game.exe      # read-only PE32+ import analysis (file is never executed)
xholy --analyze game.exe --json   # machine-readable compatibility report
```

`--analyze` only reads the file: it parses the PE32+ import table, compares
it against the HLE API registry (`src/core/hle/`) and reports implemented,
partially implemented, unsupported and unknown imports. It never executes
the executable and never modifies it. Import coverage is **not** game
compatibility, and no compatibility percentage is reported.

Config persists to `~/.config/xholy/config.toml` and is hot-reloaded on
change. User-defined profiles live under `[profiles]`.

## Infrastructure stack (Phase 11.5)

Non-game-specific subsystems live under `src/` as independently
testable modules. All public data types are serde-friendly; network,
cloud, and telemetry I/O go through injectable transports/senders so
everything runs headless in CI.

| Module | Public API (selection) | Notes / limitations |
|---|---|---|
| `config` | `Config` (emulation), `InfraConfig`, `load_effective`, `ConfigWatcher`, `DEFAULTS_TOML` | CLI > file > defaults merge; TOML+JSON; mtime hot-reload. Infra sections (`save`, `mod_system`, `cheat_engine`, `benchmark`, `debug`, `cloud`) are opt-in. |
| `logging` | `Logger`, `Level`, `Record`, `Sink`, `FileSink`, `JsonlSink`, `RotatingFile`, `Span` | ANSI-colored console + file/JSON-lines sinks, module filters, timing spans. Separate from `log`/`env_logger`. |
| `input` | `KeyboardState`, `KeyEvent`, `MouseState`, `GamepadState`, `ActionMap`, `apply_deadzone`, `PressState` | Backend-agnostic events; winit adapts into these. No OS gamepad driver — states are fed by the host. |
| `save` | `SaveManager`, `SavePayload`, `SaveMeta`, `SaveSlot`, `SaveOutcome`, `MAX_SLOTS` | Slots 1–8 + quick/auto; versioned `XHSV` header + CRC32 + RLE compression (not gzip). Cloud upload is a stub transport. |
| `mod_system` | `ModManifest`, `ModLoader`, `ModRegistry`, `resolve`, `ModExecutor`, `LifecycleState` | `xholy.mod.toml` manifests, semver-ish dep resolution, lifecycle Init→Enable→Update→Disable→Unload. Mods are Rust callbacks, not sandboxed scripts. |
| `cheat_engine` | `CheatManager`, `CheatEntry`, `CheatValue`, `Trigger`, `compile` | Single-player memory patches + triggers (timer/key/event/watch). Declarative patch format — **no Lua** (documented future work). |
| `benchmark` | `BenchmarkRunner`, `BenchResult`, `MetricStats`, `StatsRing`, `Profiler`, `Report` | CPU/memory/GPU-adapter micro-benches; Markdown+HTML reports. |
| `debug_menu` | `DebugMenu`, `MenuItem`, `DebugOverlay`, `OverlayFlags`, `PanelData`, `dump_region`, `inspect_shader` | Headless debug model (panels, overlay flags, hex dumps, shader info). UI binding is separate. |
| `cloud` | `CloudClient`, `CloudSync`, `CloudAuth`, `CloudCache`, `Transport` | HTTP abstraction over injectable `Transport` — no sockets in tests. Save sync with conflict resolution (local/remote/keep-both). |
| `shader_cache` | `ShaderCache`, `CacheKey`, `CacheManager`, `CacheStore` | FNV-1a keyed blob cache (memory + disk), LRU eviction, stats. Sits above `core::shader::shader_cache` (DXIL artifacts). |
| `network` | `Transport`, `HttpRequest`, `HttpResponse`, `NetCache`, `ProxyConfig` | Transport-injected HTTP primitives, offline-first cache, proxy bypass list. No real `reqwest` backend yet. |
| `telemetry` | `TelemetryCollector`, `TelemetryEvent`, `TelemetryFilter`, `Sender`, `MemorySender` | Anonymized, rule-filtered, batched events (deny attribute keys like `username`/`path`). |
| `multiplayer` | `Lobby`, `Peer`, `Session`, `ReplicatedState`, `SyncEngine`, `Interpolator`, `Predictor` | Lobby/session model + deterministic state merge + prediction/interpolation math. **No real netcode transport.** |
| `storage` | `StorageManager`, `StorageDevice`, `DriveKind`, `MountTable`, `SsdCache` | NVMe/SATA/HDD latency profiles, ISO/dir/optical mounts, LRU block cache for slow media. |
| `utils` | `fnv1a`, `human_bytes`, `slugify`, `ThreadPool`, `Semaphore`, `Permit`, sync helpers | Std-only helpers; no external deps. |

Cross-module coverage lives in `tests/infra_tests.rs` (18 tests):
config precedence, save stress across parallel slots, mod lifecycle,
cheat patching, offline cloud/network, telemetry filtering,
multiplayer rules, and semaphore/channel deadlock detection.

### Honest limitations (Phase 11.5)

- **No real network** — cloud/network/telemetry use injectable
  transports; a `reqwest` backend is future work.
- **No Lua cheat compiler** — `cheat_compiler` is a declarative
  patch format, not a scripting language.
- **Multiplayer is a model** — lobby/state-sync/prediction math only;
  no wire protocol, no rollback netcode.
- **RLE save compression** — not gzip/zstd; adequate for zero-heavy
  state, swap backends for real compression.
- **Debug/menu/storage layers are headless models** — UI and real
  device backends bind on top.
- Existing emulation limits still apply (see STATUS.md): simulated
  syscalls/MMU/interrupts, static shader compilation, wgpu instead of
  D3D12.

## Roadmap

The binding roadmap is in [ROADMAP.md](ROADMAP.md). Phase numbers 1 to 24A are historical. Work on PowerPC and Xbox 360 XAM (local commit 0c76865) is outside the target scope.

## Project Development Progress

The figures below are generated from and validated against [`progress.json`](progress.json) by `src/progress.rs` (a unit test fails if this README section goes stale). Run `xholy --progress` for the same report on the command line.

**Overall roadmap completion: 54.5%**

`[▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░] 54.5%`

| Category | Weight | Completion |
|---|---:|---:|
| CPU & Execution | 20% | 20.0% |
| HLE & System Runtime | 20% | 75.0% |
| GPU & Shader Translation | 20% | 50.0% |
| Memory & Storage | 10% | 65.0% |
| Audio & Input | 10% | 50.0% |
| Game Compatibility & Boot | 15% | 60.0% |
| GUI & Infrastructure | 5% | 100.0% |

Last verification: 2026-11-24.

Calculation: each task adds `weight x credit` to its category, where credit is 0 for Planned and Blocked, 0.5 for Partially Implemented, and 1 for Implemented and Verified. Task weights total 100 within each category, so the category sum is its completion percentage. Overall completion is the category completion weighted by the category weights. Everything is derived from `progress.json` by `src/progress.rs`. This measures roadmap progress only, not game compatibility, production readiness or retail boot success.

## Performance targets

| Metric | Target |
|---|---|
| Frame rate | 60 FPS @ 1440p/4K |
| Input latency | < 15 ms |
| Compatibility | Xbox Series X\|S (Gen9): goal; no retail title runs yet |

These are goals for the completed translator, not current measurements —
see `benchmarks/REPORT.md` for the scaffold baseline.

## GTA VI Statement

xHoly is not specifically developed, marketed, promoted or positioned for Grand Theft Auto VI.

The project makes no GTA VI compatibility claims or promises and does not include GTA VI-specific features or assets.

## Trademarks and Copyright

Xbox, Xbox One, Xbox Series X|S, Microsoft, Grand Theft Auto and Rockstar Games are trademarks or property of their respective owners.

All references are descriptive. xHoly is an independent project.

A disclaimer does not replace compliance with applicable copyright, trademark and software laws.

## License

xHoly's own code is distributed under the [Apache License 2.0](LICENSE).

Third-party dependencies and their respective licenses are documented separately in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). The project's license does not imply any rights to Microsoft SDKs, Xbox games, trademarks or third-party code.

## Contributing & co-authorship

See [CONTRIBUTING.md](CONTRIBUTING.md). Credit only contributors who made a substantive change. AI agents should use their verified bot identity.

**Redeem Your Games.**
