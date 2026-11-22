# xHoly — PC Specs

> Documented on 2026-11-22 18:09:00 +01:00

---

## Hardware

| Component | Value |
|-----------|-------|
| **CPU** | Intel® Core™ i7-12700F (12 Cores / 20 Threads, up to 2.1 GHz) |
| **RAM** | 32 GB DDR4-3467 MHz (G.Skill) |
| **GPU** | NVIDIA GeForce RTX 5070 (16 GB VRAM) |
| **PC Model** | Acer Predator PO3-640 |
| **OS** | Windows 11 Pro 26H2 |

---

## GPU Specs (RTX 5070)

| Property | Value |
|----------|-------|
| **Name** | NVIDIA GeForce RTX 5070 |
| **VRAM** | 16 GB (4 GiB) |
| **Driver** | Version 32.0.16.2124 |
| **Monitor** | Meta Virtual Monitor |

---

## RAM Specs

| DIMM | Capacity | Speed | Manufacturer |
|------|----------|-------|-------------|
| DIMM1 | 32 GB | 3467 MHz | G.Skill Intl |
| DIMM2 | 32 GB | 3467 MHz | G.Skill Intl |

---

## CPU Specs (i7-12700F)

| Property | Value |
|----------|-------|
| **Name** | Intel® Core™ i7-12700F |
| **Cores** | 12 |
| **Threads** | 20 |
| **Max Frequency** | 2.1 GHz |
| **Manufacturer** | GenuineIntel |
| **Family** | 198 |

---

## Storage

| Partition | Filesystem |
|-----------|-----------|
| C:\ | NTFS |

---

## OS

| Property | Value |
|----------|-------|
| **Name** | Microsoft Windows 11 Pro |
| **Version** | 10.0.26300 |
| **Build** | 26300 |

---

## Summary

- ✅ **12 Cores / 20 Threads** — more than enough for parallel JIT compiler (Phase 13)
- ✅ **16 GB VRAM** — more than enough for wgpu renderer (Phase 12)
- ✅ **32 GB RAM** — comfortably handles render loop + JIT + shader cache
- ✅ **RTX 5070** — Blackwell architecture, very performant for Vulkan/OpenGL/Metal fallback

---

## Notes

- The **F** in i7-12700F means "no integrated graphics" — this is good because we can use the full RTX 5070.
- The RTX 5070 uses the **Blackwell architecture**, which is relevant for modern wgpu features (Mesh Shaders, Raytracing, DLSS).
- G.Skill DDR4-3467 is an excellent choice for high bandwidth — helps the JIT compiler during code generation.
