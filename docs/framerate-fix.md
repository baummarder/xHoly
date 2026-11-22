# Framerate fix — Phase 7

## Problem

The GUI could sit at ~30 FPS regardless of `fps_limit`, and VSync was
not actually wired to the surface: `WgpuRenderer` always requested
`PresentMode::Fifo`, while the loop pacing was a pair of ad-hoc sleep
helpers (`pace_frame` / `pace`) that measured the wrong interval in the
headless path (it slept a *full* frame period after every frame,
doubling the effective frame time → ~30 FPS at a 60 FPS target).

## Fix

New module `src/gui/frame_pacing.rs`:

- `FramePacer` records each frame's start time, keeps a ring buffer of
  the last 240 frame durations, and exposes `get_fps` / `get_min_fps` /
  `get_max_fps` / `get_avg_fps`.
- `sleep_until_next_frame()` sleeps only the *remainder* of the frame
  budget (`last_frame_start + 1/fps − now`), never a full period.
- `FramePacingMode`: `Auto` (VSync → `Fifo`, manual trim below the
  refresh rate), `Fixed` (always manual pacing), `Uncapped`.
- `present_mode(available)` maps the decision onto `wgpu::PresentMode`:
  VSync → `Fifo`; off/uncapped → `Immediate` → `Mailbox` → `Fifo`.

`WgpuRenderer::new_surface` now takes `vsync` and picks the present
mode through `FramePacer::present_mode`, so `--no-vsync` really
reaches the swapchain instead of being ignored.

`WindowApp` owns one `FramePacer` shared by the windowed and headless
loops; `render_frame()` calls `record_frame_start()` before compositing
and `sleep_until_next_frame()` after presenting.

## Configuration

| Flag | Effect |
|---|---|
| `--fps-limit N` | Cap at N FPS (`0` = uncapped) |
| `--frame-pacing auto\|fixed\|uncapped` | Pacing policy |
| `--no-vsync` | `Immediate`/`Mailbox` present + manual pacing |

## Troubleshooting

- **Still ~30 FPS:** run `--gui-headless --max-frames 60` and time it;
  the software renderer is CPU-bound and legitimately slower. Try
  `--gpu-software` vs auto-detected Vulkan.
- **FPS won't exceed refresh rate:** that's VSync (`Fifo`) — expected.
  Use `--no-vsync --fps-limit 240` to measure headroom.
- **Stutter:** try `--frame-pacing fixed`; `Auto`+VSync trims only
  below the refresh rate.
- **Driver issues:** update GPU drivers; `docs/gpu-api-support.md`
  lists per-vendor fallback behavior.
