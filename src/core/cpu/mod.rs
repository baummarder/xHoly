//! Guest CPU emulation.
//!
//! The production design translates Xbox ARM64 (Xbox One: Jaguar x86-64;
//! Series X|S: Zen 2 x86-64) code to host code via a JIT (the "CPU
//! translator"). This scaffold implements an interpreter-style stub that
//! decodes a minimal instruction subset so the emulation loop, memory
//! subsystem, and GPU pipeline can be exercised end-to-end.

use crate::core::memory::{GuestAddr, Memory};
use crate::gpu::GpuContext;

pub mod arm64;
pub mod translator;

/// Number of architectural registers exposed to the translator.
pub const REGISTER_COUNT: usize = 32;

/// A vCPU executing guest code.
pub struct Cpu {
    /// General-purpose registers (x86-64 architectural model).
    pub regs: [u64; REGISTER_COUNT],
    pc: GuestAddr,
    halted: bool,
    /// Instructions retired this frame.
    pub instructions: u64,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: [0; REGISTER_COUNT],
            pc: 0,
            halted: false,
            instructions: 0,
        }
    }

    /// Program counter within guest memory.
    pub fn pc(&self) -> GuestAddr {
        self.pc
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// Run guest code until the emulated vblank (one frame).
    ///
    /// The stub executes a bounded number of "instructions" per frame —
    /// enough to drive the loop and GPU submission path deterministically.
    pub fn run_until_vblank(&mut self, mem: &mut Memory, gpu: &mut GpuContext) {
        const INSTRUCTIONS_PER_FRAME: u64 = 4096;
        for _ in 0..INSTRUCTIONS_PER_FRAME {
            if self.halted {
                break;
            }
            self.step(mem, gpu);
        }
        self.instructions += INSTRUCTIONS_PER_FRAME;
    }

    /// Execute one guest instruction.
    ///
    /// The scaffold implements a tiny RV32-like opcode subset over the
    /// guest memory image: LUI, AUIPC, JAL, JALR, branches, loads, stores,
    /// OP-IMM, OP, FENCE, and ECALL (halt). A real translator would emit
    /// host machine code; see docs/whitepaper.md §3.
    fn step(&mut self, mem: &mut Memory, gpu: &mut GpuContext) {
        let Some(raw) = mem.read(self.pc, 4) else {
            self.halted = true;
            return;
        };
        let insn = raw as u32;
        self.pc = self.pc.wrapping_add(4);

        let opcode = insn & 0x7f;
        let rd = ((insn >> 7) & 0x1f) as usize;
        let f3 = (insn >> 12) & 0x7;
        let rs1 = ((insn >> 15) & 0x1f) as usize;
        let rs2 = ((insn >> 20) & 0x1f) as usize;
        let f7 = insn >> 25;

        match opcode {
            0x37 => self.regs[rd] = (insn as u64) & 0xffff_f000, // LUI
            0x17 => {
                // AUIPC
                let imm = ((insn as i32) >> 12) << 12;
                self.regs[rd] = (self.pc as i32).wrapping_add(imm) as u64;
            }
            0x6f => {
                // JAL
                let imm = (((insn >> 31) as i32) << 20
                    | (((insn >> 12) & 0xff) as i32) << 12
                    | (((insn >> 20) & 1) as i32) << 11
                    | (((insn >> 21) & 0x3ff) as i32) << 1) as i64;
                self.regs[rd] = self.pc;
                self.pc = (self.pc as i64).wrapping_add(imm) as u64;
            }
            0x67 => {
                // JALR
                let imm = ((insn as i32) >> 20) as u64;
                let target = (self.regs[rs1].wrapping_add(imm)) & !1;
                self.regs[rd] = self.pc;
                self.pc = target;
            }
            0x63 => {
                // Branches
                let imm = ((insn >> 31) << 12
                    | ((insn >> 7) & 1) << 11
                    | ((insn >> 25) & 0x3f) << 5
                    | ((insn >> 8) & 0xf) << 1) as u64;
                let a = self.regs[rs1] as i64;
                let b = self.regs[rs2] as i64;
                let take = match f3 {
                    0 => a == b,
                    1 => a != b,
                    4 => a < b,
                    5 => a >= b,
                    6 => a < b, // BLTU (approximation; full impl uses unsigned)
                    7 => a >= b,
                    _ => false,
                };
                if take {
                    self.pc = self.pc.wrapping_sub(4).wrapping_add(imm);
                }
            }
            0x03 => {
                // Loads
                let imm = ((insn as i32) >> 20) as u64;
                let addr = self.regs[rs1].wrapping_add(imm);
                let size = match f3 {
                    0 | 4 => 1,
                    1 | 5 => 2,
                    2 => 4,
                    3 => 8,
                    _ => 4,
                };
                if let Some(v) = mem.read(addr, size) {
                    self.regs[rd] = if f3 >= 4 { v } else { v as i64 as u64 };
                }
            }
            0x23 => {
                // Stores
                let imm = (((insn >> 25) as u64) << 5) | (((insn >> 7) & 0x1f) as u64);
                let addr = self.regs[rs1].wrapping_add(((insn as i32) >> 20) as u64);
                let size = match f3 {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    3 => 8,
                    _ => 4,
                };
                let _ = imm;
                mem.write(addr, self.regs[rs2], size);
            }
            0x13 => {
                // OP-IMM
                let imm = ((insn as i32) >> 20) as u64;
                self.regs[rd] = match f3 {
                    0 => self.regs[rd].wrapping_add(imm),
                    1 => (self.regs[rd] as i64).wrapping_shl((imm & 0x3f) as u32) as u64,
                    2 => self.regs[rd] & imm,
                    3 => self.regs[rd] | imm,
                    4 => self.regs[rd] ^ imm,
                    5 => {
                        if f7 == 0x20 {
                            ((self.regs[rd] as i64) >> ((imm & 0x3f) as u32)) as u64
                        } else {
                            self.regs[rd] >> ((imm & 0x3f) as u32)
                        }
                    }
                    6 => (self.regs[rd] < imm) as u64,
                    7 => self.regs[rd].wrapping_sub(imm),
                    _ => self.regs[rd],
                };
            }
            0x33 => {
                // OP (register-register)
                self.regs[rd] = match (f3, f7) {
                    (0, 0x00) => self.regs[rd].wrapping_add(self.regs[rs2]),
                    (0, 0x20) => self.regs[rd].wrapping_sub(self.regs[rs2]),
                    (1, 0x00) => ((self.regs[rd] as i64) << (self.regs[rs2] & 0x3f)) as u64,
                    (2, 0x00) => self.regs[rd] & self.regs[rs2],
                    (3, 0x00) => self.regs[rd] | self.regs[rs2],
                    (4, 0x00) => self.regs[rd] ^ self.regs[rs2],
                    (5, 0x00) => self.regs[rd] >> (self.regs[rs2] & 0x3f),
                    (5, 0x20) => ((self.regs[rd] as i64) >> (self.regs[rs2] & 0x3f)) as u64,
                    (6, 0x00) => (self.regs[rd] < self.regs[rs2]) as u64,
                    (7, 0x00) => self.regs[rd].wrapping_sub(self.regs[rs2]),
                    _ => self.regs[rd],
                };
            }
            0x0f => {
                // FENCE — no-op in the scaffold
            }
            0x73 => {
                // ECALL / EBREAK — halt
                if f3 == 0 {
                    self.halted = true;
                }
            }
            _ => {
                log::trace!(
                    "xHoly: unimplemented opcode {:#04x} at pc={:#x}",
                    opcode,
                    self.pc - 4
                );
            }
        }

        // Feed the GPU once per frame boundary deterministically.
        if self.pc.is_multiple_of(0x1000) {
            gpu.submit_frame(mem);
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
