# xHoly

<p align="center">
  <img src="xHoly-logo.svg" width="160" alt="xHoly logo"/>
</p>

<p align="center"><b>Redeem Your Games</b></p>

xHoly is a research emulator/simulator architecture for **Xbox Series S|X** and
**Xbox One** game images, built on a hybrid **Rust + Scala** stack.

> **Status: early research scaffold.** This repository implements the
> project architecture, subsystem boundaries, configuration system, and
> build pipeline. It is **not** yet a working game emulator — CPU/GPU
> translation passes are stubs pending further research. See
> [`docs/whitepaper.md`](docs/whitepaper.md) for the technical roadmap.

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
| GPU translation layer | Rust | DXIL→SPIR-V/GLSL/WGSL, software rasterizer fallback |
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

Produces a single `xholy` binary. Runs on `x86_64`, `aarch64`, and
`armv7` (Raspberry Pi 4) targets.

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
xholy --game game.bin       # run ARM64 guest image headless (Phase 8)
xholy --game games/minimal  # bundled minimal guest (XEX2 + xboxconfig)
xholy --native-wm           # native C++ WM demo window (Windows)
```

#### Minimal guest game (Phase 17)

`games/minimal/` is a real on-disk guest image: `default.xex` carries a
little-endian `XEX2` header (entry RVA, image base, title-id quad) followed
by an AArch64 payload that writes `Hello, xHoly!` through the kernel's
SYS_WRITE and exits via SYS_EXIT; `xboxconfig.xml` supplies the title and
TitleID. The loader strips the header, maps the payload at `USER_BASE`,
and the ARM64 interpreter runs it end-to-end:

```sh
xholy --game games/minimal          # prints "Hello, xHoly!"
xholy --game games/minimal/default.xex   # same, direct XEX file
xholy --mount games --list          # lists "xHoly Minimal Guest"
xholy --gui-tui                     # Enter on the game launches it;
                                    # output lands in the Console pane
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

## Performance targets

| Metric | Target |
|---|---|
| Frame rate | 60 FPS @ 1440p/4K |
| Input latency | < 15 ms |
| Compatibility | Xbox (2001) → Xbox Series S|X |

These are goals for the completed translator, not current measurements —
see `benchmarks/REPORT.md` for the scaffold baseline.

## License

[Apache License 2.0](LICENSE)

## Contributing & co-authorship

See [CONTRIBUTING.md](CONTRIBUTING.md). Credit only contributors who made a substantive change. AI agents should use their verified bot identity.

**Redeem Your Games.**
