# GPU API Support — xHoly GUI (Phase 6)

Which graphics API should xHoly use per GPU/driver? The GUI auto-detects
via `src/gui/gpu_api_detector.rs` (wgpu backend probing: Vulkan →
OpenGL → Software) and persists the result to
`~/.config/xholy/gpu_config.toml`. Users can force an API with
`--gpu-api vulkan|opengl|software` or the `--gpu-*`/`--gui-*` aliases.

## API landscape

| API | Driver maturity | Headless/offscreen | Notes for emulation |
|---|---|---|---|
| **Vulkan 1.2+** | Excellent on all discrete GPUs | Yes (no display needed for compute/offscreen) | Preferred: closest to the Xbox GDK (DX12→Vulkan translation target) |
| **OpenGL 4.x** | Ubiquitous, incl. Mesa llvmpipe | Partial (needs context; EGL surfaceless works) | Good fallback; wgpu `GL` backend covers it |
| **Metal** | Apple only | Yes | Not directly usable — wgpu maps it via **MoltenVK** (Vulkan-on-Metal) |
| **Software (llvmpipe/SwiftShader)** | Always available via Mesa/ANGLE | Yes | Last resort; fine for UI, too slow for game rendering |

## Per-vendor support

### Intel

| GPU | Vulkan | OpenGL | Recommendation |
|---|---|---|---|
| Iris Xe (11th gen+, Tiger Lake and newer) | ✅ Vulkan 1.3 | ✅ GL 4.6 | **Vulkan** |
| UHD Graphics 620/630 (8th–10th gen) | ✅ Vulkan 1.1/1.2 (driver-dependent) | ✅ GL 4.5/4.6 | **Vulkan** if probe succeeds, else OpenGL |
| UHD 610/630 (older) | ⚠️ Spotty Vulkan | ✅ GL 4.0/4.5 | OpenGL fallback |
| Arc (Alchemist/Lunar Lake) | ✅ Vulkan 1.3 | ✅ GL 4.6 | **Vulkan** |

### AMD

| GPU | Vulkan | OpenGL | Recommendation |
|---|---|---|---|
| Radeon RX 5000/6000/7000 (RDNA) | ✅ Vulkan 1.3 | ✅ GL 4.6 | **Vulkan** |
| Radeon VII / Vega | ✅ Vulkan 1.2/1.3 | ✅ GL 4.6 | **Vulkan** |
| Radeon RX 400/500 (GCN) | ✅ Vulkan 1.0–1.2 | ✅ GL 4.5/4.6 | Vulkan (older conformance) |
| Integrated (Ryzen APU) | ✅ Vulkan 1.2+ | ✅ GL 4.6 | **Vulkan** |

### NVIDIA

| GPU | Vulkan | OpenGL | Recommendation |
|---|---|---|---|
| RTX 3000/4000/5000 | ✅ Vulkan 1.3 | ✅ GL 4.6 | **Vulkan** |
| GTX 10/16 series | ✅ Vulkan 1.2/1.3 | ✅ GL 4.6 | **Vulkan** |
| GTX 900 and older | ⚠️ Vulkan 1.0–1.1 | ✅ GL 4.5 | OpenGL fallback preferred |

### Mesa software rasterizers

| Renderer | API | When to use |
|---|---|---|
| **llvmpipe** (Mesa) | OpenGL 4.5 software | No GPU at all, or driver broken — UI still works |
| **lavapipe** (Mesa) | Vulkan 1.1 software | Rarely chosen by wgpu probe; useful for compute-only tests |
| **SwiftShader** (ANGLE/Chromium) | Vulkan 1.1 / GL ES | Bundled fallback in some Android/Chrome contexts |

Software rendering is the right choice when: CI/headless without GPU,
driver crashes, or the user explicitly wants deterministic CPU output.
It is **never** the right choice for actual game emulation performance.

### Apple Silicon (M1/M2/M3/M4)

| Path | Reality |
|---|---|
| Metal | Native, best performance — but wgpu doesn't expose Metal directly |
| **MoltenVK** | Vulkan 1.1/1.2 on top of Metal — **this is what wgpu uses on macOS** |
| OpenGL | Deprecated, frozen at 4.1 — avoid |

On macOS, wgpu's `VULKAN` backend bit transparently uses MoltenVK.
Detection reports `Vulkan` with adapter name like "Apple M2".

### ARM64 (Raspberry Pi 4/5, ARM servers)

| Platform | Vulkan | OpenGL | Recommendation |
|---|---|---|---|
| Pi 4 (VideoCore VI) | ❌ None | GLES 3.1 / GL 2.1 (VC4 driver) | Software / llvmpipe |
| Pi 5 (VideoCore VII) | ⚠️ Experimental (v3d Vulkan driver in Mesa 24+) | GLES 3.1 | Probe Vulkan first, else software |
| ARM server (no display) | llvmpipe/lavapipe only | llvmpipe | **Software** |

## xHoly detection order (implemented)

```
GpuApiPreference::Auto
  1. wgpu::Backends::VULKAN probe  → adapter? → Vulkan
  2. wgpu::Backends::GL probe      → adapter? → OpenGL
  3. otherwise                     → Software (CPU compositor)
```

Each probe calls `Instance::request_adapter` headlessly, so detection
works in CI. The chosen backend is written to
`~/.config/xholy/gpu_config.toml` for reuse.

## Fallback strategy summary

| Situation | Backend |
|---|---|
| Discrete GPU (any vendor, last ~8 years) | Vulkan |
| Older iGPU / broken Vulkan driver | OpenGL (wgpu GL) |
| Apple Silicon | Vulkan (= MoltenVK) |
| Pi/ARM without GPU driver | Software |
| CI / headless tests | Software or offscreen wgpu |
| User override | `--gpu-api <vulkan|opengl|software>` |
