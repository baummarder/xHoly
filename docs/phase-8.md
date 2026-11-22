# Phase 8 — Game Execution & C++ WM Integration

## What was built

### 1. Real AArch64 interpreter — `src/core/cpu/interpreter.rs`

Decodes **actual ARM64 machine code** (not the Phase-1 custom ISA in
`arm64.rs`). Supported subset:

- Wide immediate: `MOVZ`/`MOVN`/`MOVK` (32/64-bit, all shifts)
- Add/sub: immediate (imm12, optional LSL#12) and shifted register
  (LSL/LSR/ASR), with NZCV flag computation
- Logical: `AND`/`ORR`/`EOR`/`ANDS` shifted-register forms
- Loads/stores: `LDR`/`STR`/`LDRB`/`STRB`/`LDRH`/`STRH`/`LDRSW`
  unsigned-offset
- Branches: `B`, `BL`, `B.cond`, `CBZ`/`CBNZ`, `RET`, `BRK` (halt)
- System: `SVC #imm` → kernel handler, `NOP`/`HINT`

`Arm64Cpu` has `va_base` translation so guest VAs (kernel/user layout)
map into a flat memory buffer. Unsupported encodings raise
`Arm64Error::IllegalInstruction` rather than silently misexecuting.

### 2. Xbox kernel stub — `src/core/kernel/mod.rs`

- Memory layout constants: `KERNEL_BASE`/`KERNEL_END`, `USER_BASE`,
  `CONSOLE_BASE` MMIO window
- Syscall table (x0 = number, x1–x7 = args):
  `0` Write, `1` CreateThread, `2` CreateProcess, `3` Sleep,
  `4` WriteFile, `5` ReadFile, `6` Yield, `7` Exit, `8` Ticks
- `Thread` = full register snapshot; `Kernel` keeps a thread table and
  a round-robin `schedule_next()` that ages sleep timers and rotates
  the run queue on Sleep/Yield/Exit
- `Kernel::attach(cpu)` installs itself as the CPU's `svc_handler` and
  maps the console aperture so `STR` to `CONSOLE_BASE` appends output

### 3. Game loader — `src/core/loader.rs`

- `load_game(path)`: file or directory (`default.xex` → `*.xex` →
  `*.bin/.img/.xbe`)
- XEX2 header parsing: entry point + image base fields
- `xboxconfig.xml` minimal parser (`TitleID`, `Name`, `Studio`,
  `Path`, `ReleaseDate`)
- `GameImage::load_into(cpu)` copies bytes at `load_base` and sets `pc`

### 4. Execution glue — `src/core/execution.rs`

`execute_game(image, max_steps)` wires loader → interpreter → kernel:
allocates `GUEST_MEM_SIZE` (~18 MiB) with `va_base = KERNEL_BASE`,
attaches the kernel, loads the image at `USER_BASE`, runs bounded.
Returns `RunReport { steps, output, kernel_log, threads }`.
`hello_world_program(msg)` assembles a real ARM64 "Hello" binary
(MOVZ/MOVK address build + SVC write/exit).

### 5. C++ WM ↔ Rust FFI — `cpp/window_manager/window_manager_ffi.*` + `src/gui/native_wm.rs`

- C++ side: `extern "C"` API (`xh_wm_create`, `xh_wm_create_window`,
  `xh_wm_pump`, `xh_wm_*` geometry/z-order/title/event accessors)
- `build.rs` compiles both `.cpp` files via the `cc` crate on Windows
  and links `user32/gdi32/shell32`
- Rust side: safe wrappers (`NativeWindowManager`, `NativeWindow`,
  `NativeWindowType`, `NativeWindowStyle`, flag bits)
- `--native-wm [--max-frames N]` runs a demo: creates a real HWND via
  the C++ manager, pumps messages until close

## Usage

```powershell
# Execute a flat ARM64 game image headless:
xholy --game path\to\game.bin --max-steps 1000000

# XEX / game directory (with xboxconfig.xml):
xholy --game "games\mygame" --max-steps 500000

# Native WM demo window (Windows only):
xholy --native-wm --max-frames 600
```

`hello_world_program` produces a working guest binary — the test
`kernel_write_syscall_produces_console_output` proves the full path:
ARM64 decode → SYS_WRITE → kernel → console output.

## Honest limitations

- **Interpreter subset, not full ARM64**: no NEON/SIMD, no exclusive
  ops, no system registers, no bitmask-immediate logicals, no
  load/store pair/pre/post-index forms. Real games will hit
  `IllegalInstruction` quickly — this runs *small* programs, not
  retail titles.
- **No JIT**: interpreted only (the prompt's `rustc-llvm` idea isn't
  practical; a real dynasm-style JIT is future work).
- **Kernel is a stub**: syscall numbers are xHoly-assigned, not the
  real Xbox kernel ABI (which goes through XAM/loader, not raw SVC).
  Threads are context snapshots — no real preemptive scheduling,
  no synchronization primitives.
- **Loader is XEX-lite**: section table, imports, TLS, and the real
  XEX optional-header chain are not parsed; flat images work, retail
  XEX will load but likely fault early.
- **GPU path unchanged**: the guest can't yet write a framebuffer the
  wgpu GUI displays — `RunReport.output` is console text. Wiring
  guest VRAM → `WgpuRenderer` is the next step.
- **FFI is additive, not a winit replacement**: `--native-wm` proves
  the C++ manager works from Rust, but the main GUI still runs on
  winit. Migrating `windowed/` onto `native_wm` is future work.
- **Windows-only FFI**: `native_wm`/`build.rs` are cfg-gated; other
  targets skip the C++ compile entirely.
