//! PowerPC (Xbox 360) decoder + interpreter — Phase 24 foundation.
//!
//! Real **big-endian** PowerPC encodings for an integer/branch subset:
//! `addi`, `addis`, `ori`, `lwz`, `stw`, `add`, `or`, `cmp`, `mfspr`,
//! `mtspr`, `b`/`bl`, `bc`, `blr`, `sc`. 128-bit pixel SIMD lives in
//! [`simd`]. Unsupported encodings raise [`PpcError::IllegalInstruction`]
//! instead of silently mis-executing.
//!
//! Honest scope: this is the *foundation* of a retail-boot path (boot
//! sequence + a useful instruction subset), not a complete PowerPC CPU —
//! no floating point, no full VMX instruction decoder, no real XEX PE
//! relocation. See STATUS.md.

pub mod simd;

use std::fmt;

/// Guest address the boot image is linked at.
pub const BOOT_BASE: u64 = 0x8000_1000;
/// Default guest memory size (64 MiB).
pub const PPC_MEM_SIZE: usize = 0x0400_0000;

/// Interpreter failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpcError {
    /// Unsupported/undefined instruction.
    IllegalInstruction { pc: u64, word: u32 },
    /// Access outside the flat guest buffer.
    MemoryFault { addr: u64, write: bool },
}

impl fmt::Display for PpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IllegalInstruction { pc, word } => {
                write!(f, "illegal PowerPC instruction {word:#010x} at {pc:#x}")
            }
            Self::MemoryFault { addr, write } => {
                write!(
                    f,
                    "PowerPC memory {} at {addr:#x}",
                    if *write { "write" } else { "read" }
                )
            }
        }
    }
}
impl std::error::Error for PpcError {}

/// A supervisor call raised by `sc`.
#[derive(Debug, Clone, Copy)]
pub struct PpcCall {
    /// Syscall number (from r0).
    pub number: u32,
    /// r3–r7 arguments.
    pub args: [u64; 5],
}

/// Callback for guest syscalls. Return `false` to halt the CPU.
pub type PpcSvcHandler = dyn FnMut(&mut PpcCpu, &PpcCall) -> bool;

/// PowerPC architectural state (subset).
pub struct PpcCpu {
    /// General-purpose registers r0..r31.
    pub gpr: [u64; 32],
    /// Condition register (8 fields × 4 bits).
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Integer exception register (CA bit only).
    pub xer: u32,
    /// Program counter (guest VA).
    pub pc: u64,
    /// Flat guest memory; `memory[i]` is guest VA `va_base + i`.
    pub memory: Vec<u8>,
    /// Guest VA of `memory[0]`.
    pub va_base: u64,
    /// Console output bytes.
    pub output: Vec<u8>,
    /// Retired instructions.
    pub steps: u64,
    /// Whether the CPU should keep running.
    pub running: bool,
    /// Optional syscall handler.
    pub svc_handler: Option<Box<PpcSvcHandler>>,
}

impl std::fmt::Debug for PpcCpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PpcCpu")
            .field("pc", &self.pc)
            .field("cr", &self.cr)
            .field("steps", &self.steps)
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

impl Default for PpcCpu {
    fn default() -> Self {
        Self::new(PPC_MEM_SIZE)
    }
}

impl PpcCpu {
    /// Create a CPU with `mem_size` bytes of memory.
    pub fn new(mem_size: usize) -> Self {
        Self {
            gpr: [0; 32],
            cr: 0,
            lr: 0,
            ctr: 0,
            xer: 0,
            pc: 0,
            memory: vec![0; mem_size],
            va_base: 0x8000_0000,
            output: Vec::new(),
            steps: 0,
            running: true,
            svc_handler: None,
        }
    }

    /// Load bytes at a guest VA.
    pub fn load_at(&mut self, addr: u64, bytes: &[u8]) -> Result<(), PpcError> {
        let off = self
            .offset(addr)
            .ok_or(PpcError::MemoryFault { addr, write: true })?;
        if off + bytes.len() > self.memory.len() {
            return Err(PpcError::MemoryFault { addr, write: true });
        }
        self.memory[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    fn offset(&self, addr: u64) -> Option<usize> {
        let rel = addr.checked_sub(self.va_base)?;
        let off = rel as usize;
        (off < self.memory.len()).then_some(off)
    }

    fn read32(&self, addr: u64) -> Result<u32, PpcError> {
        let off = self
            .offset(addr)
            .ok_or(PpcError::MemoryFault { addr, write: false })?;
        let s = self
            .memory
            .get(off..off + 4)
            .ok_or(PpcError::MemoryFault { addr, write: false })?;
        Ok(u32::from_be_bytes(s.try_into().unwrap()))
    }

    fn write32(&mut self, addr: u64, value: u32) -> Result<(), PpcError> {
        let off = self
            .offset(addr)
            .ok_or(PpcError::MemoryFault { addr, write: true })?;
        let s = self
            .memory
            .get_mut(off..off + 4)
            .ok_or(PpcError::MemoryFault { addr, write: true })?;
        s.copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Sign-extend a `bits`-wide value to 64 bits.
    fn sext(v: u64, bits: u32) -> u64 {
        let shift = 64 - bits;
        (((v << shift) as i64) >> shift) as u64
    }

    fn set_cr_field(&mut self, field: usize, a: u64, b: u64) {
        let lt = (a as i32) < (b as i32);
        let gt = (a as i32) > (b as i32);
        let eq = a == b;
        let mut value = 0u32;
        if lt {
            value |= 0b1000;
        }
        if gt {
            value |= 0b0100;
        }
        if eq {
            value |= 0b0010;
        }
        // SO bit (overflow) is left clear — XER[OV] is not modeled.
        let shift = (7 - field) * 4;
        self.cr = (self.cr & !(0xF << shift)) | (value << shift);
    }

    fn cr_bit(&self, bit: usize) -> bool {
        (self.cr >> (31 - bit)) & 1 == 1
    }

    /// Decode + execute one instruction.
    pub fn step(&mut self) -> Result<(), PpcError> {
        let pc = self.pc;
        let word = self.read32(pc)?;
        self.pc = pc.wrapping_add(4);
        let op = word >> 26;

        match op {
            14 => {
                // addi rD, rA, imm16 (signed)
                let rt = ((word >> 21) & 31) as usize;
                let ra = ((word >> 16) & 31) as usize;
                let imm = Self::sext((word & 0xFFFF) as u64, 16);
                let base = if ra == 0 { 0 } else { self.gpr[ra] };
                self.gpr[rt] = base.wrapping_add(imm);
            }
            15 => {
                // addis rD, rA, imm16 (shifted)
                let rt = ((word >> 21) & 31) as usize;
                let ra = ((word >> 16) & 31) as usize;
                let imm = ((word & 0xFFFF) as u64) << 16;
                let base = if ra == 0 { 0 } else { self.gpr[ra] };
                self.gpr[rt] = base.wrapping_add(imm);
            }
            24 => {
                // ori rA, rS, uimm16
                let ra = ((word >> 21) & 31) as usize;
                let rs = ((word >> 16) & 31) as usize;
                let imm = (word & 0xFFFF) as u64;
                self.gpr[ra] = self.gpr[rs] | imm;
            }
            32 => {
                // lwz rD, d(rA)
                let rt = ((word >> 21) & 31) as usize;
                let ra = ((word >> 16) & 31) as usize;
                let d = Self::sext((word & 0xFFFF) as u64, 16);
                let base = if ra == 0 { 0 } else { self.gpr[ra] };
                self.gpr[rt] = self.read32(base.wrapping_add(d))? as u64;
            }
            36 => {
                // stw rS, d(rA)
                let rs = ((word >> 21) & 31) as usize;
                let ra = ((word >> 16) & 31) as usize;
                let d = Self::sext((word & 0xFFFF) as u64, 16);
                let base = if ra == 0 { 0 } else { self.gpr[ra] };
                self.write32(base.wrapping_add(d), self.gpr[rs] as u32)?;
            }
            18 => {
                // b / bl
                // LI occupies bits 25..2; masking off AA/LK already leaves
                // the byte displacement (no second << 2).
                let li = Self::sext((word & 0x03FF_FFFC) as u64, 26);
                let target = if word & 2 != 0 {
                    li
                } else {
                    pc.wrapping_add(li)
                };
                if word & 1 != 0 {
                    self.lr = pc.wrapping_add(4);
                }
                self.pc = target;
            }
            16 => {
                // bc — only the no-CTR-decrement form is decoded.
                let bo = (word >> 21) & 0x1F;
                let bi = ((word >> 16) & 0x1F) as usize;
                // BD occupies bits 15..2 — masking off the low two bits
                // yields the byte displacement directly.
                let bd = Self::sext((word & 0xFFFC) as u64, 16);
                if bo & 0x14 != 0x04 || bo & 0x08 != 0 {
                    return Err(PpcError::IllegalInstruction { pc, word });
                }
                let cond = self.cr_bit(bi);
                let take = if bo & 1 != 0 { cond } else { !cond };
                if take {
                    let target = if word & 2 != 0 {
                        bd
                    } else {
                        pc.wrapping_add(bd)
                    };
                    if word & 1 != 0 {
                        self.lr = pc.wrapping_add(4);
                    }
                    self.pc = target;
                }
            }
            17 => {
                // sc — syscall number in r0, args in r3..r7.
                let call = PpcCall {
                    number: self.gpr[0] as u32,
                    args: [
                        self.gpr[3],
                        self.gpr[4],
                        self.gpr[5],
                        self.gpr[6],
                        self.gpr[7],
                    ],
                };
                let mut handler = self.svc_handler.take();
                let keep = match handler.as_mut() {
                    Some(h) => h(self, &call),
                    None => true,
                };
                self.svc_handler = handler;
                if !keep {
                    self.running = false;
                }
            }
            19 if word == 0x4E80_0020 => {
                // blr — an unset LR (0) is the scaffold's "guest returned"
                // halt, mirroring BRK in the ARM64 interpreter.
                if self.lr == 0 {
                    self.running = false;
                } else {
                    self.pc = self.lr;
                }
            }
            31 => {
                // Extended opcode: add / or / cmp / mfspr / mtspr.
                let xo = word & 0x7FE;
                let rt = ((word >> 21) & 31) as usize;
                let ra = ((word >> 16) & 31) as usize;
                let rb = ((word >> 11) & 31) as usize;
                match xo {
                    0x214 => {
                        // add rD, rA, rB
                        self.gpr[rt] = self.gpr[ra].wrapping_add(self.gpr[rb]);
                    }
                    0x378 => {
                        // or rA, rS, rB
                        self.gpr[ra] = self.gpr[rt] | self.gpr[rb];
                    }
                    0x000 => {
                        // cmp crf, rA, rB
                        let field = ((word >> 23) & 7) as usize;
                        self.set_cr_field(field, self.gpr[ra], self.gpr[rb]);
                    }
                    0x2A6 => {
                        // mfspr rD, spr
                        let spr = (((word >> 16) & 0x1F) << 5) | ((word >> 11) & 0x1F);
                        self.gpr[rt] = match spr {
                            8 => self.lr,
                            9 => self.ctr,
                            _ => 0,
                        };
                    }
                    0x3A6 => {
                        // mtspr spr, rS
                        let spr = (((word >> 16) & 0x1F) << 5) | ((word >> 11) & 0x1F);
                        match spr {
                            8 => self.lr = self.gpr[rt],
                            9 => self.ctr = self.gpr[rt],
                            _ => {}
                        }
                    }
                    _ => return Err(PpcError::IllegalInstruction { pc, word }),
                }
            }
            _ => return Err(PpcError::IllegalInstruction { pc, word }),
        }
        self.steps += 1;
        Ok(())
    }

    /// Run until halt, error, or `max_steps` retired instructions.
    pub fn run(&mut self, max_steps: u64) -> Result<u64, PpcError> {
        let start = self.steps;
        while self.running && self.steps - start < max_steps {
            self.step()?;
        }
        Ok(self.steps - start)
    }
}

/// Minimal retail-boot model: load a PowerPC payload at [`BOOT_BASE`]
/// and begin executing at its first word.
pub struct PpcBootloader;

impl PpcBootloader {
    /// Boot `image` and return the prepared CPU (before execution).
    pub fn prepare(image: &[u8]) -> Result<PpcCpu, PpcError> {
        let mut cpu = PpcCpu::new(PPC_MEM_SIZE);
        cpu.load_at(BOOT_BASE, image)?;
        cpu.pc = BOOT_BASE;
        cpu.gpr[1] = BOOT_BASE + PPC_MEM_SIZE as u64 - 16; // stack pointer
        Ok(cpu)
    }

    /// Prepare and run up to `max_steps` instructions.
    pub fn boot(image: &[u8], max_steps: u64) -> Result<PpcCpu, PpcError> {
        let mut cpu = Self::prepare(image)?;
        cpu.run(max_steps)?;
        Ok(cpu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(insn: u32) -> Vec<u8> {
        insn.to_be_bytes().to_vec()
    }

    fn code(words: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        for word in words {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn addi(rt: u32, ra: u32, imm: u16) -> u32 {
        (14 << 26) | (rt << 21) | (ra << 16) | imm as u32
    }
    fn lwz(rt: u32, ra: u32, d: u16) -> u32 {
        (32 << 26) | (rt << 21) | (ra << 16) | d as u32
    }
    fn stw(rs: u32, ra: u32, d: u16) -> u32 {
        (36 << 26) | (rs << 21) | (ra << 16) | d as u32
    }
    fn cmp(crf: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (crf << 23) | (ra << 16) | (rb << 11)
    }
    fn bc(bo: u32, bi: u32, words: i16) -> u32 {
        (16 << 26) | (bo << 21) | (bi << 16) | ((words as u16) << 2) as u32
    }
    fn ext(xo: u32, rt: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | xo
    }
    fn blr() -> u32 {
        0x4E80_0020
    }

    #[test]
    fn decodes_addi_ori_and_stores() {
        // addi r3,r0,42 ; stw r3,0(r4) ; lwz r5,0(r4) ; blr
        let bytes = code(&[addi(3, 0, 42), stw(3, 4, 0), lwz(5, 4, 0), blr()]);
        let mut cpu = PpcBootloader::prepare(&bytes).unwrap();
        cpu.gpr[4] = BOOT_BASE + 0x1000;
        cpu.run(16).unwrap();
        assert_eq!(cpu.gpr[3], 42);
        assert_eq!(cpu.gpr[5], 42);
        assert_eq!(cpu.steps, 4);
    }

    #[test]
    fn branch_and_compare_work() {
        // li r3,1 ; cmp r3,r3 ; bc 5,2,+2 words (branch on EQ) ; li r3,9 ; blr
        let bytes = code(&[
            addi(3, 0, 1),
            cmp(0, 3, 3),
            bc(5, 2, 2),
            addi(3, 0, 9),
            blr(),
        ]);
        let mut cpu = PpcBootloader::prepare(&bytes).unwrap();
        cpu.run(16).unwrap();
        assert_eq!(cpu.gpr[3], 1, "taken branch must skip li r3,9");
    }

    #[test]
    fn illegal_instruction_is_reported() {
        let bytes = w(0xFFFF_FFFF);
        let mut cpu = PpcBootloader::prepare(&bytes).unwrap();
        assert!(matches!(
            cpu.step(),
            Err(PpcError::IllegalInstruction { .. })
        ));
    }

    #[test]
    fn bootloader_runs_sc_syscall() {
        // li r0,7 ; sc ; blr
        let bytes = code(&[addi(0, 0, 7), 0x4400_0022, blr()]);
        let mut cpu = PpcBootloader::prepare(&bytes).unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen2 = seen.clone();
        cpu.svc_handler = Some(Box::new(move |_cpu, call| {
            seen2.borrow_mut().push(call.number);
            false
        }));
        cpu.run(16).unwrap();
        assert_eq!(*seen.borrow(), vec![7]);
    }

    #[test]
    fn extended_add_and_or_decode() {
        // addi r3,r0,5 ; addi r4,r0,3 ; add r5,r3,r4 ; or r6,r3,r4 ; blr
        let bytes = code(&[
            addi(3, 0, 5),
            addi(4, 0, 3),
            ext(0x214, 5, 3, 4), // add r5, r3, r4
            ext(0x378, 3, 6, 4), // or r6, r3, r4 (RA field is the destination)
            blr(),
        ]);
        let mut cpu = PpcBootloader::prepare(&bytes).unwrap();
        cpu.run(16).unwrap();
        assert_eq!(cpu.gpr[5], 8);
        assert_eq!(cpu.gpr[6], 7);
    }
}
