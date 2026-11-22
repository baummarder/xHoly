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
/// `USER_BASE - KERNEL_BASE` = 16 MiB; add slack for stack/heap.
pub const GUEST_MEM_SIZE: usize = 0x120_0000; // ~18 MiB

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

    Ok(RunReport {
        steps,
        output: cpu.output.clone(),
        kernel_log: kern.log.clone(),
        threads: kern.threads.len(),
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
