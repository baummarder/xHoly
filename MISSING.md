# Missing Components Audit

Audit of `xHoly` against the target architecture (Xbox Series S|X + Xbox One
emulation, hybrid Rust/Scala). Generated 2026-11-21.

## 📁 Files Missing

### Core / CPU (`src/core/cpu/`)
- [ ] `translator.rs` full implementation — currently a stub: only `MOV r64,imm`
  and two ALU patterns decode; needs complete x86-64 decoder (all prefixes,
  ModRM/SIB, REX, VEX/EVEX, FPU/SSE/AVX, segment ops, string ops, flags)
- [ ] `decoder.rs` — dedicated x86-64 length decoder + instruction table
- [ ] `ir.rs` — expanded IR (flags model, memory ops, atomics, SIMD)
- [ ] `lowering.rs` — IR → host machine code emitter (Cranelift-style)
- [ ] `block_cache.rs` — translated-block cache keyed by guest PC
- [ ] `dispatcher.rs` — block chaining / lookup loop
- [ ] `self_modifying.rs` — SMC detection and block invalidation
- [ ] `interrupts.rs` — vCPU exception/IRQ delivery, GICv3 model, IRQ routing
- [ ] `timers.rs` — ARM generic timer + Xbox timer emulation
- [ ] `cpuid.rs` — CPUID leaf emulation for Xbox CPU identification

### GPU (`src/gpu/`)
- [ ] `dx12/` — D3D12 command-list interpreter → wgpu passes
- [ ] `dx11/`, `dx10/`, `dx9/`, `d3d8/` — older API translation layers
- [ ] `dxgi/` — swapchain/present handling
- [ ] `descriptor_heap.rs` — D3D12 descriptor heap model
- [ ] `pipeline_state.rs` — PSO cache and hash
- [ ] `raytracing.rs` — DXR → Vulkan RT pipeline
- [ ] `mesh_shaders.rs` — mesh/amplification shader path
- [ ] `vrs.rs` — variable rate shading mapping
- [ ] `present.rs` — low-latency present / frame pacing
- [ ] `capture.rs` — gameplay recording (video + input)

### Memory (`src/core/`)
- [ ] `mmu.rs` — page tables, TLB, permission model
- [ ] `paging.rs` — 4K/2M/1G page support, NX bit
- [ ] `allocator.rs` — guest heap emulation (Xbox kernel allocator ABI)
- [ ] `carveout.rs` — GPU memory carve-out management
- [ ] `dma.rs` — guest DMA engine model

### Audio (`src/audio/`)
- [ ] `mixer.rs` — multi-stream mixing, voice management
- [ ] `xma.rs` — XMA/XWM decode (Xbox One audio format)
- [ ] `wasapi.rs`, `coreaudio.rs`, `alsa.rs` — real backend bindings
- [ ] `hrtf.rs` — spatial audio

### I/O (`src/io/`)
- [ ] `xinput.rs` — XInput device layer
- [ ] `dualsense.rs` / `xbox_pad.rs` — modern controller mappings
- [ ] `network.rs` — virtual Xbox Live NIC (XNet emulation)
- [ ] `storage_nvme.rs` — virtual NVMe device
- [ ] `usb_xbox_one_pad.rs` — USB device emulation for pad firmware
- [ ] `kinect_real.rs` — actual Kinect sensor ingest

### Optical (`src/optical/`)
- [ ] `udf.rs` — UDF 2.5/2.6 parser (Blu-ray Video + BDAV)
- [ ] `bluray.rs` — Blu-ray playlist/BUP parsing
- [ ] `ddrive.rs` — direct SCSI pass-through to physical drive
- [ ] `metadata_db.rs` — scraped title metadata DB (titles, cover art)

### Runtime / Scala
- [ ] `scala/shared/.../Settings.scala` — case-class mirror of TOML config
- [ ] `scala/jvm/.../JvmRuntime.scala` — fuller JVM runtime with classloading
- [ ] `scala/js/.../WebMain.scala` — real web UI shell
- [ ] `jni_bridge.c` / `bridge.h` — JNI FFI layer for deep JVM integration

### Tooling / Plugins
- [ ] `src/plugins/` — cheat engine plugin API (`Cheat`, `MemoryWatcher`)
- [ ] `src/mods/` — mod manager (load order, conflict resolution)
- [ ] `src/cloud/` — async asset streaming client
- [ ] `src/hid/` — expanded HID report descriptors per device class
- [ ] `src/netplay.rs` — rollback netplay

## 📁 Directories Missing

- [ ] `assets/` — game covers, fonts, UI resources *(partially created: `assets/icons/`)*
- [ ] `shaders/` — host-side WGSL/GLSL shader sources
- [ ] `firmware/` — Xbox system software blobs (user-supplied)
- [ ] `games/` — user game library root (gitignored)
- [ ] `saves/` — save-state storage root (gitignored)
- [ ] `mods/` — user mod drop folder (gitignored)
- [ ] `tests/` — integration tests (disc images, golden traces)
- [ ] `tools/` — dev utilities *(created for icon renderer)*
- [ ] `packaging/` — Windows installer (WiX), `.desktop` files, AppStream metainfo
- [ ] `docs/api/` — generated rustdoc hosting

## 🔧 Dependencies Missing

### Rust (`Cargo.toml`)
- [ ] `tokio = "1"` — async runtime for cloud streaming / netplay
- [ ] `rayon = "1"` — parallel translation/compile passes
- [ ] `cranelift-*` or `dynasmrt` — JIT backend for CPU translation
- [ ] `naga`/`spirv` — shader bytecode tooling (wgpu already pulls naga)
- [ ] `xdelta3` bindings — save-state/patch delta compression
- [ ] `lz4`/`zstd` — fast decompression for game assets
- [ ] `zip`/`tar` — archive support for mods & DLC
- [ ] `quick-xml` — proper XML parsing for `xboxconfig.xml`
- [ ] `image`/`png` — screenshot PNG encoding (currently BMP-only)
- [ ] `rodio` or `cpal` — real audio output (currently null driver)
- [ ] `gilrs`/`sdl2` — real gamepad input (currently stub poller)
- [ ] `windows`/`core-foundation` crates — platform HID/audio APIs
- [ ] `clap_complete` — shell completions for CLI
- [ ] `tracing` + `tracing-subscriber` — structured logging (currently `log`)

### Scala (`scala/build.sbt`)
- [ ] `org.portable-scala:sbt-platform-deps` *(present)*
- [ ] `com.github.scopt:scopt` — CLI parsing on JVM side
- [ ] `org.typelevel:cats-effect` — effect system for async runtime
- [ ] `org.http4s:http4s-dsl` — HTTP for cloud/metadata APIs
- [ ] `org.scalatest:scalatest` — test framework
- [ ] `com.eed3si9n:sbt-assembly` *(needed for fat jar; referenced but plugin not declared)*

## ⚙️ Configuration Files Missing

- [ ] `.cargo/config.toml` — target flags, linker settings, `rustflags`
- [ ] `rust-toolchain.toml` — pinned toolchain
- [ ] `.config/nextest.toml` — faster test runner config
- [ ] `.rustfmt.toml` — project rustfmt settings
- [ ] `.scalafmt.conf` — Scala formatting
- [ ] `.editorconfig` — cross-editor consistency
- [ ] `deny.toml` — cargo-deny license/advisory policy
- [ ] `clippy.toml` — clippy configuration
- [ ] `codecov.yml` / `tarpaulin` config — coverage
- [ ] `renovate.json` — dependency update automation
- [ ] `Dockerfile` / `docker-compose.yml` — reproducible build env
- [ ] `.env.example` — env var template (paths, API keys)
- [ ] `packaging/io.xholy.appdata.xml` — Linux AppStream metainfo
- [ ] `packaging/xholy.desktop` — freedesktop launcher

## 📝 Documentation Missing

- [ ] `docs/api.md` — public API reference (crate + Scala bridge protocol)
- [ ] `docs/architecture.md` — detailed subsystem diagrams
- [ ] `docs/tutorial.md` — getting-started guide (build → first boot)
- [ ] `docs/hacking.md` — contributor internals guide
- [ ] `docs/formats.md` — XVC/ISO/game config format notes
- [ ] `docs/compatibility.md` — title compatibility matrix
- [ ] `docs/legal.md` — firmware/homebrew disclaimer
- [ ] `docs/roadmap.md` — milestone tracker
- [ ] `CHANGELOG.md` — release notes
- [ ] `docs/protocol.md` — Rust↔Scala stdio JSON schema

## 🧪 Tests Missing

- [ ] `tests/iso_fixtures/` — sample ISO9660/UDF images (tiny, synthetic)
- [ ] `tests/guest_code/` — golden instruction-trace fixtures
- [ ] `tests/integration/` — end-to-end disc-mount → game-list tests
- [ ] CPU translator unit tests per opcode family
- [ ] GPU shader-translation golden tests (DXIL→WGSL)
- [ ] Config hot-reload tests
- [ ] Scala-side specs (`scalatest`) for `GameLoader`

## 🎮 Stubbed Functions Needing Real Implementations

| Location | Stub | Needs |
|---|---|---|
| `src/core/cpu/mod.rs` | `run_until_vblank` instruction subset | Full x86-64 coverage |
| `src/core/cpu/translator.rs` | `decode_x86_64_stub` | Real decoder + JIT |
| `src/core/memory.rs` | 1 GiB flat arena | Sparse paging, carve-outs |
| `src/gpu/passthrough.rs` | `fill_pattern` test pattern | wgpu render passes |
| `src/gpu/shader.rs` | `translate` emits WGSL comment | Real DXIL→SPIR-V |
| `src/gpu/translation.rs` | single triangle | Full command-stream xlat |
| `src/audio/driver.rs` | `submit` counts samples | WASAPI/CoreAudio/ALSA |
| `src/io/input.rs` | `poll_gamepad` returns neutral | SDL/XInput polling |
| `src/io/hid.rs` | `enumerate`/`read_report` empty | Platform HID backends |
| `src/io/kinect.rs` | `latest_frame` empty | Sensor ingest / recorded frames |
| `src/io/storage.rs` | mount bookkeeping only | Guest block-device layer |
| `src/optical/iso_parser.rs` | UDF → error | Full UDF 2.6 parser |
| `src/optical/game_scanner.rs` | heuristic folder scan | Disc-layout database |
| `scala/.../GameLoader.scala` | registry + string replies | Real JVM module loading |
