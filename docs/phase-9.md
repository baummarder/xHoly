# Phase 9 — GUI & Rendering Upgrade

## What was built

### 1. GPU post-processing pipeline

`shaders/postfx.wgsl` + `src/gui/windowed/postfx.rs` implement a real
multi-pass wgpu pipeline on top of the existing blit:

```
composited UI (RGBA8 texture)
  → bright-pass        (half res, soft-knee luminance threshold)
  → gaussian blur H    (9-tap, half res)
  → gaussian blur V    (9-tap, half res)
  → composite          (bloom add → FXAA-lite → tone map)
  → surface / offscreen target
```

- **Bloom**: threshold + separable Gaussian at half resolution,
  intensity configurable (`--bloom`/`--no-bloom`, default on @ 0.6).
- **Tone mapping**: Linear / Reinhard / Filmic / ACES — WGSL and a Rust
  reference implementation (`PostFxConfig::tonemap`) kept in parity
  for tests.
- **Anti-aliasing**: FXAA-lite (luma-edge blend) in the composite
  shader; `--aa msaa4` parses but currently maps to FXAA (documented).
- Uniforms: two vec4 buffers (params + texel/direction) — split into
  two buffers because `min_uniform_buffer_offset_alignment` is 256.

`WgpuRenderer::set_postfx(PostFxConfig)` wires it in; when the config
is fully inactive the renderer uses the original single-blit path.
Offscreen mode now renders into a persistent `present_texture` so
post-FX output is readable via `readback`.

### 2. UI widget system — `src/gui/widgets.rs`

Retained-mode widget tree on top of the immediate-mode compositor:

- `Widget`: Button / Panel / Label / Slider with stable ids and z-order
- `WidgetTree`: `hit_test` (topmost-by-z), `update(mouse, down,
  pressed)` → hover/pressed/drag state machine, `Clicked`/`Dragged`
  events, `draw()` rasterizes into the RGBA framebuffer with
  state-dependent colors (hover brightens, press darkens)
- Unit-tested: z-order hit-testing, click events, slider drag values,
  pixel output

### 3. Texture management — `src/gui/textures.rs`

- `TextureAtlas`: CPU-side sprite atlas (insert/get, row packing)
- `load_rgba` / `load_rgba_at_offset`: raw `XRGA`-header RGBA and
  ISO-offset extraction (honest scaffold — Xbox ISO textures have no
  standard container; callers supply offsets)
- `textures::gpu::TextureCache`: wgpu texture upload cache keyed by name

### 4. Shader cache — `src/gui/shader_cache.rs`

Persists validated copies of `gui.wgsl`/`postfx.wgsl` under
`%LOCALAPPDATA%\xHoly\cache\shaders\` (Win) / `$XDG_CACHE_HOME/xholy/`
with an FNV-1a manifest. wgpu/naga exposes no portable binary shader
cache, so this caches *verified sources* — corrupted entries are
detected on read.

### 5. Performance monitoring

`WindowApp::tick_fps` now counts consecutive frames below 50 % of the
FPS limit and logs a perf warning — complements the existing
`FramePacer` min/max/avg stats.

### 6. CLI flags (`src/main.rs`)

- `--renderer API` — alias for `--gpu-api`
- `--bloom` / `--no-bloom`
- `--tone-mapping linear|reinhard|filmic|aces`
- `--aa off|fxaa|msaa4`

All land in `GuiConfig.postfx` → `WgpuRenderer::set_postfx` for both
windowed and headless paths.

## Honest limitations

- **No real MSAA/TAA**: `--aa msaa4` maps to FXAA-lite; true MSAA
  needs a multisampled scene target (the UI is a single blit texture).
- **No game-shader translation**: the Xbox DXIL→SPIR-V path remains a
  stub — this phase upgrades the *GUI* renderer, not game shaders.
- **Widget layer is opt-in**: `WidgetTree` exists and is tested but the
  main UI still composites via `ui.rs` immediate mode; migrating
  toolbar/sidebar onto widgets is follow-up work.
- **FXAA-lite ≠ full FXAA**: 8-neighbor luma blend, not the full
  CONSOLAS/FXAA 3.11 algorithm.
- **Bloom is half-res 9-tap** — good enough for UI glow, not cinematic.
- **PCF shadow maps** from the prompt are not implemented (UI is
  flat-colored; no shadow-receiving geometry exists yet).
- **Shader cache stores sources, not binaries** — see module docs.
