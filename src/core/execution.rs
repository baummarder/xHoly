//! Guest game execution — Phase 8 glue.
//!
//! Ties together the pieces: [`loader`] produces a [`GameImage`], the
//! [`Arm64Cpu`] interpreter executes its ARM64 code, and the
//! [`Kernel`] stub services `SVC` syscalls (console writes, sleeps,
//! threads). Memory uses the Xbox-flavored VA layout: `va_base` =
//! [`KERNEL_BASE`], so user code lands at [`USER_BASE`].

use crate::core::cpu::interpreter::{Arm64Cpu, Arm64Error};
use crate::core::kernel::{self, Kernel};
use crate::core::loader::GameImage;

/// Guest physical memory size covering kernel + user regions.
/// `USER_BASE - KERNEL_BASE` = 16 MiB; the kernel heap arena
/// ([`HEAP_BASE`](crate::core::kernel::HEAP_BASE)) sits at offset
/// 32 MiB, so the buffer must reach past it.
pub const GUEST_MEM_SIZE: usize = 0x220_0000; // ~34 MiB

/// A guest framebuffer frame extracted from guest memory (Phase 18).
#[derive(Debug, Clone)]
pub struct GuestFrame {
    /// Framebuffer width in pixels.
    pub width: u32,
    /// Framebuffer height in pixels.
    pub height: u32,
    /// Frame sequence number written by the guest (0 = never written).
    pub seq: u32,
    /// RGBA8 pixels, row-major (`width * height * 4` bytes).
    pub pixels: Vec<u8>,
}

/// Result of running a guest image.
#[derive(Debug)]
pub struct RunReport {
    /// Instructions retired.
    pub steps: u64,
    /// Console output bytes produced (kernel writes + MMIO stores).
    pub output: Vec<u8>,
    /// Kernel log lines (WriteFile stubs etc.).
    pub kernel_log: Vec<String>,
    /// Thread count the kernel saw.
    pub threads: usize,
    /// Last guest framebuffer frame, if the guest wrote one.
    pub frame: Option<GuestFrame>,
}

/// Read the guest framebuffer aperture from CPU memory.
///
/// Bounds-checked against the flat guest memory **and** the kernel
/// memory-layer region table — returns `None` when the aperture is
/// unmapped, malformed, or was never written (`seq == 0`).
pub fn read_guest_frame(cpu: &Arm64Cpu) -> Option<GuestFrame> {
    use crate::core::kernel::memory::region_of;
    let va = kernel::GUEST_FB_BASE;
    // Memory-layer check: the aperture must live in a mapped,
    // guest-writable region.
    let region = region_of(va)?;
    if !region.permissions.write || region.permissions.supervisor {
        return None;
    }
    // Flat-buffer bounds check.
    let rel = va.checked_sub(cpu.va_base)? as usize;
    let header = cpu.memory.get(rel..rel + kernel::GUEST_FB_HEADER)?;
    let magic = u32::from_le_bytes(header[0..4].try_into().ok()?);
    if magic != kernel::GUEST_FB_MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(header[4..8].try_into().ok()?);
    let height = u32::from_le_bytes(header[8..12].try_into().ok()?);
    let seq = u32::from_le_bytes(header[12..16].try_into().ok()?);
    if seq == 0 || width == 0 || height == 0 {
        return None;
    }
    // Sanity-cap against the declared aperture (a guest can't claim a
    // larger frame than the mapped region holds).
    let want = width as usize * height as usize * 4;
    if want > kernel::GUEST_FB_SIZE - kernel::GUEST_FB_HEADER {
        return None;
    }
    let px = cpu
        .memory
        .get(rel + kernel::GUEST_FB_HEADER..rel + kernel::GUEST_FB_HEADER + want)?;
    Some(GuestFrame {
        width,
        height,
        seq,
        pixels: px.to_vec(),
    })
}

/// Execute a loaded [`GameImage`] on the interpreter + kernel stub.
///
/// `max_steps` bounds the run (a frame's worth of guest instructions
/// in a real emulator; here it's a safety cap for the scaffold).
pub fn execute_game(image: &GameImage, max_steps: u64) -> Result<RunReport, Arm64Error> {
    let mut cpu = Arm64Cpu::new(GUEST_MEM_SIZE);
    cpu.va_base = kernel::KERNEL_BASE;

    // SP starts near the top of guest memory (a VA).
    cpu.sp = cpu.va_base + GUEST_MEM_SIZE as u64 - 16;

    let mut kern = Kernel::new(&cpu);
    kern.attach(&mut cpu);

    image
        .load_into(&mut cpu)
        .map_err(|_| Arm64Error::FetchOutOfBounds {
            pc: image.load_base,
        })?;

    let steps = cpu.run(max_steps)?;
    // Extract the guest framebuffer before the CPU is dropped — this
    // is the Phase-18 blit source for the GUI.
    let frame = read_guest_frame(&cpu);

    Ok(RunReport {
        steps,
        output: cpu.output.clone(),
        kernel_log: kern.log.clone(),
        threads: kern.threads.len(),
        frame,
    })
}

/// Load + execute a game path end-to-end (CLI convenience).
pub fn run_game_path(path: &std::path::Path, max_steps: u64) -> Result<RunReport, String> {
    let image = crate::core::loader::load_game(path).map_err(|e| e.to_string())?;
    eprintln!(
        "xHoly: loaded '{}' ({} bytes @ {:#x}, entry {:#x})",
        image.title,
        image.image.len(),
        image.load_base,
        image.entry
    );
    execute_game(&image, max_steps).map_err(|e| e.to_string())
}

/// Assemble a tiny "Hello, xHoly!" ARM64 program for tests/demos:
/// writes a string via SYS_WRITE then SYS_EXIT.
///
/// Encoding helpers mirror the interpreter's supported subset.
pub fn hello_world_program(msg: &[u8]) -> Vec<u8> {
    // Layout: code at USER_BASE, string data appended after code.
    // Instructions (all real AArch64):
    //   MOVZ x0, #SYS_WRITE(0)          ; syscall number in x0
    //   ADR  x1, msg                    ; -> use MOVZ/MOVK of absolute addr
    //   MOVZ x2, #len                   ; length (assume < 65536, one MOVZ)
    //   SVC   #0
    //   MOVZ x0, #SYS_EXIT(7)
    //   SVC   #0
    //   BRK   #0
    // msg bytes follow, padded to 4.
    let base = crate::core::kernel::USER_BASE;
    let code_len = 10 * 4usize; // 10 instructions
    let msg_addr = base + code_len as u64;

    let mut code: Vec<u32> = Vec::new();
    // x0 = SYS_WRITE = 0 → MOVZ x0, #0
    code.push(0xD2800000);
    // x1 = msg_addr (64-bit constant via MOVZ+MOVK×3, hw = shift/16)
    let a = msg_addr;
    code.push(0xD2800000 | (((a & 0xFFFF) as u32) << 5) | 1); // MOVZ x1, #lo
    code.push(0xF2800000 | (1 << 21) | ((((a >> 16) & 0xFFFF) as u32) << 5) | 1); // MOVK x1, lsl16
    code.push(0xF2800000 | (2 << 21) | ((((a >> 32) & 0xFFFF) as u32) << 5) | 1); // MOVK x1, lsl32
    code.push(0xF2800000 | (3 << 21) | ((((a >> 48) & 0xFFFF) as u32) << 5) | 1); // MOVK x1, lsl48
                                                                                  // x2 = msg.len() (MOVZ, assume < 64KiB)
    code.push(0xD2800000 | ((msg.len() as u32 & 0xFFFF) << 5) | 2);
    // SVC #0
    code.push(0xD4000001);
    // x0 = SYS_EXIT = 7
    code.push(0xD2800000 | (7 << 5));
    // SVC #0
    code.push(0xD4000001);
    // BRK #0 (fallback halt)
    code.push(0xD4200000);

    // Flatten code + message into bytes.
    let mut bytes: Vec<u8> = Vec::with_capacity(code_len + msg.len() + 4);
    for w in &code {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes.extend_from_slice(msg);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loader::GameImage;
    use std::path::PathBuf;

    #[test]
    fn hello_world_runs_and_prints() {
        let bytes = hello_world_program(b"Hello, xHoly!\n");
        let img = GameImage {
            title: "hello".into(),
            title_id: None,
            load_base: crate::core::kernel::USER_BASE,
            entry: crate::core::kernel::USER_BASE,
            image: bytes,
            source: PathBuf::from("mem"),
            metadata: Default::default(),
        };
        let report = execute_game(&img, 10_000).expect("run");
        assert_eq!(report.output, b"Hello, xHoly!\n");
        assert!(report.steps >= 8);
    }

    #[test]
    fn bundled_minimal_game_runs_from_disk() {
        // The real loader path: games/minimal/default.xex (XEX2) →
        // payload → ARM64 interpreter → kernel SVC → console output.
        let img = crate::core::loader::load_game(std::path::Path::new("games/minimal"))
            .expect("load bundled game");
        let report = execute_game(&img, 200_000).expect("run");
        assert_eq!(report.output, b"Hello, xHoly!\n");
        assert!(report.steps >= 8);
    }

    #[test]
    fn bundled_minimal_game_writes_guest_frame() {
        // Phase 18: the guest writes a two-tone pattern into the FB
        // aperture; read_guest_frame must return it bounds-checked.
        let img = crate::core::loader::load_game(std::path::Path::new("games/minimal"))
            .expect("load bundled game");
        let report = execute_game(&img, 200_000).expect("run");
        let frame = report.frame.expect("guest frame present");
        assert_eq!(frame.width, kernel::GUEST_FB_W);
        assert_eq!(frame.height, kernel::GUEST_FB_H);
        assert_eq!(frame.seq, 1);
        assert_eq!(
            frame.pixels.len(),
            (frame.width * frame.height * 4) as usize
        );
        // Top half = "x" color (200,40,40), bottom half = "y" cyan.
        let top = &frame.pixels[0..4];
        let bottom = &frame.pixels[(frame.pixels.len() - 4)..];
        assert_eq!(top, &[200, 40, 40, 255]);
        assert_eq!(bottom, &[30, 226, 226, 255]);
    }

    #[test]
    fn guest_frame_empty_without_guest_write() {
        // A program that never touches the FB aperture yields no frame.
        let mut bytes = Vec::new();
        for w in [0xD2800000u32 | (7 << 5), 0xD4000001, 0xD4200000] {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let img = GameImage {
            title: "t".into(),
            title_id: None,
            load_base: crate::core::kernel::USER_BASE,
            entry: crate::core::kernel::USER_BASE,
            image: bytes,
            source: PathBuf::from("mem"),
            metadata: Default::default(),
        };
        let report = execute_game(&img, 100).expect("run");
        assert!(report.frame.is_none());
    }

    #[test]
    fn unknown_syscall_is_survivable() {
        // Program: SVC with unknown number → kernel logs, continues, BRK.
        let mut bytes = Vec::new();
        for w in [
            (0xD2800000u32 | (99 << 5)), // MOVZ x0, #99 (unknown syscall)
            0xD4000001,                  // SVC #0
            0xD4200000,                  // BRK
        ] {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let img = GameImage {
            title: "t".into(),
            title_id: None,
            load_base: crate::core::kernel::USER_BASE,
            entry: crate::core::kernel::USER_BASE,
            image: bytes,
            source: PathBuf::from("mem"),
            metadata: Default::default(),
        };
        let report = execute_game(&img, 100).expect("run");
        assert!(report
            .kernel_log
            .iter()
            .any(|l| l.contains("unknown syscall")));
    }
}
