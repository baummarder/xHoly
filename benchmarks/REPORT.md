# Benchmark Report — xHoly scaffold (v0.1.0)

Baseline measurements for the research scaffold on the development host.
These numbers characterize the *interpreter stub*, not a finished
emulator — they exist so future translator work has a regression baseline.

Regenerate with:

```sh
cargo bench
```

## Environment

- Host: Windows 11, x86_64 (see `cargo bench` output for exact CPU)
- Toolchain: Rust 1.99.1, criterion 0.5

## Results (indicative)

| Benchmark | Notes |
|---|---|
| `cpu/frame_step_4096_insns` | Interpreter decode of 4096-instruction frames |
| `gpu/software_rasterizer_1080p` | Single barycentric triangle @ 1920×1080 |
| `memory/read_write_4k` | 1024×4-byte guest read+write pairs |

Full criterion HTML reports are emitted to `target/criterion/`.

## Path to the performance targets

| Target | Scaffold status |
|---|---|
| 60 FPS @ 1440p/4K | Software rasterizer is single-threaded; hardware backend pending wgpu integration |
| <15 ms input latency | Input poller is a stub; SDL/XInput integration pending |
| Zero-latency feel | Requires frame pipelining + low-latency present mode (future work) |
