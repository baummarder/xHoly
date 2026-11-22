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
xholy --native-wm           # native C++ WM demo window (Windows)
```

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
