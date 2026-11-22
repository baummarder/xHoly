//! Real AArch64 (ARMv8-A) interpreter — Phase 8.
//!
//! Unlike [`super::arm64`] (a custom xHoly ISA used for the Phase-1
//! scaffold), this module decodes **actual ARM64 machine code**: the
//! encodings below are the real AArch64 formats, so a genuine compiled
//! ARM64 binary (e.g. an Xbox game's `.text` section) can execute.
//!
//! # Supported subset
//!
//! | Class | Instructions |
//! |---|---|
//! | Wide immediate | `MOVZ`, `MOVN`, `MOVK` (W & X, all shifts) |
//! | Add/sub immediate | `ADD`, `ADDS`, `SUB`, `SUBS` (imm12, optional LSL#12) |
//! | Add/sub shifted reg | `ADD`, `ADDS`, `SUB`, `SUBS` (LSL/LSR/ASR) |
//! | Logical shifted reg | `AND`, `ORR`, `EOR`, `ANDS` (LSL/LSR/ASR/ROR) |
//! | Loads/stores | `LDR`/`STR`/`LDRB`/`STRB`/`LDRH`/`STRH`/`LDRSW` unsigned offset |
//! | Branches | `B`, `BL`, `B.cond`, `CBZ`, `CBNZ`, `RET` |
//! | System | `SVC #imm` (→ kernel), `NOP`, `HINT` |
//!
//! NEON/SIMD, exclusive ops, and the full system-register file are
//! **not** implemented — unsupported encodings raise
//! [`Arm64Error::IllegalInstruction`] instead of silently misexecuting.
//!
//! # Integration
//!
//! [`Arm64Cpu::step`] fetches/decode/executes one instruction.
//! [`Arm64Cpu::run`] loops until `pc` leaves the loaded image, an
//! illegal instruction is hit, or `max_steps` is exhausted. `SVC`
//! instructions are forwarded to an optional [`SvcHandler`] so the
//! Xbox-kernel stub ([`crate::core::kernel`]) can service them.

use std::fmt;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Interpreter failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arm64Error {
    /// The 32-bit word at `pc` is not a supported AArch64 encoding.
    IllegalInstruction { pc: u64, word: u32 },
    /// Fetch fell outside loaded memory.
    FetchOutOfBounds { pc: u64 },
    /// Load/store touched unmapped memory.
    MemoryFault { addr: u64, write: bool },
}

impl fmt::Display for Arm64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IllegalInstruction { pc, word } => {
                write!(f, "illegal AArch64 instruction {word:#010x} at {pc:#x}")
            }
            Self::FetchOutOfBounds { pc } => write!(f, "fetch out of bounds at {pc:#x}"),
            Self::MemoryFault { addr, write } => {
                write!(
                    f,
                    "memory {} at {addr:#x}",
                    if *write { "write" } else { "read" }
                )
            }
        }
    }
}

impl std::error::Error for Arm64Error {}

// ---------------------------------------------------------------------------
// SVC interface
// ---------------------------------------------------------------------------

/// A supervisor call raised by `SVC #imm`.
#[derive(Debug, Clone, Copy)]
pub struct SvcCall {
    /// `SVC` immediate (usually 0).
    pub number: u16,
    /// x0–x7 argument registers at the call site.
    pub args: [u64; 8],
}

/// Kernel callback: inspect/mutate CPU state for a syscall.
/// Return `false` to halt the CPU.
pub type SvcHandler = dyn FnMut(&mut Arm64Cpu, &SvcCall) -> bool;

// ---------------------------------------------------------------------------
// CPU state
// ---------------------------------------------------------------------------

/// AArch64 architectural state (subset).
///
/// `svc_handler` is skipped in the derived `Debug` impl (trait objects
/// aren't `Debug`).
pub struct Arm64Cpu {
    /// X0–X30 (index 31 is ZR/XZR, not SP).
    pub regs: [u64; 31],
    /// Stack pointer (X31 in load/store/base contexts).
    pub sp: u64,
    /// Program counter — byte address into `memory` space.
    pub pc: u64,
    /// NZCV flags: N=bit31-sign stored as bools for clarity.
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// Flat guest memory. `memory[i]` corresponds to guest VA
    /// `va_base + i` — kernel/user VAs map into the flat buffer.
    pub memory: Vec<u8>,
    /// Guest VA of `memory[0]`. With the Xbox layout this is
    /// [`KERNEL_BASE`](crate::core::kernel::KERNEL_BASE) so kernel code
    /// sits at offset 0 and user code at `USER_BASE - KERNEL_BASE`.
    pub va_base: u64,
    /// Bytes written to the kernel console aperture.
    pub output: Vec<u8>,
    /// Total retired instructions.
    pub steps: u64,
    /// Whether the CPU should keep executing.
    pub running: bool,
    /// SVC sink — installed by the kernel stub.
    pub svc_handler: Option<Box<SvcHandler>>,
    /// Console MMIO base (writes here append to `output`).
    pub console_base: u64,
    /// Console aperture size.
    pub console_size: u64,
}

impl Arm64Cpu {
    /// Create a CPU with `mem_size` bytes of flat memory, `pc = 0`.
    pub fn new(mem_size: usize) -> Self {
        Self {
            regs: [0; 31],
            sp: mem_size.saturating_sub(16) as u64,
            pc: 0,
            n: false,
            z: false,
            c: false,
            v: false,
            memory: vec![0u8; mem_size],
            va_base: 0,
            output: Vec::with_capacity(256),
            steps: 0,
            running: true,
            svc_handler: None,
            console_base: u64::MAX, // disabled until kernel maps it
            console_size: 0,
        }
    }

    /// Translate a guest VA to a flat-memory offset.
    #[inline]
    fn off(&self, addr: u64) -> Option<usize> {
        let rel = addr.checked_sub(self.va_base)?;
        let off = rel as usize;
        (off < self.memory.len()).then_some(off)
    }

    /// Load bytes at a guest address.
    pub fn load_at(&mut self, addr: u64, bytes: &[u8]) -> Result<(), Arm64Error> {
        let off = self
            .off(addr)
            .ok_or(Arm64Error::FetchOutOfBounds { pc: addr })?;
        if off + bytes.len() > self.memory.len() {
            return Err(Arm64Error::FetchOutOfBounds { pc: addr });
        }
        self.memory[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Read a general register; index 31 → SP (for base/addr contexts the
    /// caller decides whether SP or XZR semantics apply — we expose SP).
    #[inline]
    fn x(&self, r: u8) -> u64 {
        if r == 31 {
            self.sp
        } else {
            self.regs[r as usize]
        }
    }

    /// Write a general register; index 31 is XZR → discarded.
    #[inline]
    fn set_x(&mut self, r: u8, val: u64) {
        if r != 31 {
            self.regs[r as usize] = val;
        }
    }

    /// Read memory little-endian with bounds check.
    #[inline]
    fn mem_read(&self, addr: u64, size: usize) -> Result<u64, Arm64Error> {
        let off = self
            .off(addr)
            .ok_or(Arm64Error::MemoryFault { addr, write: false })?;
        if off + size > self.memory.len() {
            return Err(Arm64Error::MemoryFault { addr, write: false });
        }
        let mut v = 0u64;
        for i in 0..size {
            v |= (self.memory[off + i] as u64) << (8 * i);
        }
        Ok(v)
    }

    /// Write memory little-endian; console aperture appends to `output`.
    #[inline]
    fn mem_write(&mut self, addr: u64, val: u64, size: usize) -> Result<(), Arm64Error> {
        if self.console_size > 0
            && (self.console_base..self.console_base + self.console_size).contains(&addr)
        {
            let bytes = &val.to_le_bytes()[..size];
            self.output.extend_from_slice(bytes);
            return Ok(());
        }
        let off = self
            .off(addr)
            .ok_or(Arm64Error::MemoryFault { addr, write: true })?;
        if off + size > self.memory.len() {
            return Err(Arm64Error::MemoryFault { addr, write: true });
        }
        for i in 0..size {
            self.memory[off + i] = (val >> (8 * i)) as u8;
        }
        Ok(())
    }

    /// Public read access for the kernel/syscall layer.
    #[inline]
    pub fn mem_read_pub(&self, addr: u64, size: usize) -> Result<u64, Arm64Error> {
        self.mem_read(addr, size)
    }

    /// Public write access for the kernel/syscall layer.
    #[inline]
    pub fn mem_write_pub(&mut self, addr: u64, val: u64, size: usize) -> Result<(), Arm64Error> {
        self.mem_write(addr, val, size)
    }

    /// Set register x0-style by index (kernel return values).
    #[inline]
    pub fn set_reg(&mut self, idx: u8, val: u64) {
        self.set_x(idx, val);
    }

    /// Fetch one 32-bit instruction word.
    #[inline]
    fn fetch(&self) -> Result<u32, Arm64Error> {
        let off = self
            .off(self.pc)
            .ok_or(Arm64Error::FetchOutOfBounds { pc: self.pc })?;
        if off + 4 > self.memory.len() {
            return Err(Arm64Error::FetchOutOfBounds { pc: self.pc });
        }
        Ok(u32::from_le_bytes([
            self.memory[off],
            self.memory[off + 1],
            self.memory[off + 2],
            self.memory[off + 3],
        ]))
    }

    // -- decode helpers ------------------------------------------------------

    /// Sign-extend `v` of `bits` width.
    #[inline]
    fn sext(v: u64, bits: u32) -> u64 {
        let shift = 64 - bits;
        (((v << shift) as i64) >> shift) as u64
    }

    /// Logical shift right on `v` (64 or 32-bit container).
    #[inline]
    fn lsr(v: u64, amount: u32, width64: bool) -> u64 {
        let w = if width64 { 64 } else { 32 };
        let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
        let amt = amount.min(w - 1);
        (v & mask) >> amt
    }

    /// Arithmetic shift right (sign-extends within the container width).
    #[inline]
    fn asr(v: u64, amount: u32, width64: bool) -> u64 {
        let w = if width64 { 64 } else { 32 };
        let amt = amount.min(w - 1);
        let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
        let sv = Self::sext(v & mask, w);
        (sv >> amt) & mask
    }

    /// Rotate right within container width.
    #[inline]
    fn ror(v: u64, amount: u32, width64: bool) -> u64 {
        let w = if width64 { 64 } else { 32 };
        let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
        let amt = amount % w;
        if amt == 0 {
            v & mask
        } else {
            (((v & mask) >> amt) | ((v & mask) << (w - amt))) & mask
        }
    }

    /// Apply an operand shift (shift kind 0=LSL,1=LSR,2=ASR,3=ROR).
    #[inline]
    fn shift(v: u64, kind: u32, amount: u32, width64: bool) -> u64 {
        match kind {
            0 => {
                let w = if width64 { 64 } else { 32 };
                let amt = amount.min(w - 1);
                (v << amt) & if width64 { u64::MAX } else { 0xFFFF_FFFF }
            }
            1 => Self::lsr(v, amount, width64),
            2 => Self::asr(v, amount, width64),
            _ => Self::ror(v, amount, width64),
        }
    }

    // -- execute one instruction ---------------------------------------------

    /// Decode + execute the instruction at `pc`. Returns `Ok(steps=1)` or
    /// an error; `self.pc` is left at the *next* instruction unless a
    /// branch redirected it.
    pub fn step(&mut self) -> Result<(), Arm64Error> {
        let pc = self.pc;
        let w = self.fetch()?;
        let next_pc = pc.wrapping_add(4);
        self.pc = next_pc;

        // ---- system / hints -------------------------------------------------
        if w == 0xD503201F {
            // NOP
            self.steps += 1;
            return Ok(());
        }
        if (w & 0xFFE0_0000) == 0xD420_0000 {
            // BRK #imm — debugger breakpoint; the emulator treats it as
            // a program halt (like a guest `exit`).
            self.running = false;
            self.steps += 1;
            return Ok(());
        }
        if (w & 0xFFFF_F01F) == 0xD503201F {
            // HINT-space aliases treated as NOP (yield/wfe/etc.)
            self.steps += 1;
            return Ok(());
        }
        if (w & 0xFFE0_001F) == 0xD400_0001 {
            // SVC #imm
            let number = ((w >> 5) & 0xFFFF) as u16;
            let args = [
                self.x(0),
                self.x(1),
                self.x(2),
                self.x(3),
                self.x(4),
                self.x(5),
                self.x(6),
                self.x(7),
            ];
            let call = SvcCall { number, args };
            // Take the handler out so we can pass `self` to it.
            let mut handler = self.svc_handler.take();
            let keep_running = match handler.as_mut() {
                Some(h) => h(self, &call),
                None => true, // no kernel → ignore syscall, continue
            };
            self.svc_handler = handler;
            if !keep_running {
                self.running = false;
            }
            self.steps += 1;
            return Ok(());
        }
        if (w & 0xFFFF_FC1F) == 0xD65F_0000 {
            // RET Xn
            let rn = ((w >> 5) & 0x1F) as u8;
            self.pc = self.x(rn);
            self.steps += 1;
            return Ok(());
        }

        // ---- unconditional branches -----------------------------------------
        // op0(31-26): B=000101, BL=100101
        let b_op = (w >> 26) & 0x3F;
        if b_op == 0x05 || b_op == 0x25 {
            let imm = Self::sext((w & 0x03FF_FFFF) as u64, 28) << 2;
            let target = pc.wrapping_add(imm);
            if b_op == 0x25 {
                self.set_x(30, next_pc); // LR = return address
            }
            self.pc = target;
            self.steps += 1;
            return Ok(());
        }
        if (w & 0xFF00_0010) == 0x5400_0000 {
            // B.cond
            let imm = Self::sext(((w >> 5) & 0x7FFFF) as u64, 19) << 2;
            let cond = (w & 0xF) as u8;
            if self.cond_true(cond) {
                self.pc = pc.wrapping_add(imm);
            }
            self.steps += 1;
            return Ok(());
        }
        if (w & 0x7E00_0000) == 0x3400_0000 {
            // CBZ / CBNZ (sf in bit31, op in bit24, Rt in bits4-0)
            let sf = (w >> 31) & 1;
            let nonzero = (w >> 24) & 1;
            let rt = (w & 0x1F) as u8;
            let imm = Self::sext(((w >> 5) & 0x7FFFF) as u64, 19) << 2;
            let val = self.x(rt);
            let is_zero = if sf == 1 {
                val == 0
            } else {
                (val & 0xFFFF_FFFF) == 0
            };
            let take = if nonzero == 1 { !is_zero } else { is_zero };
            if take {
                self.pc = pc.wrapping_add(imm);
            }
            self.steps += 1;
            return Ok(());
        }

        // ---- wide immediate (MOVZ/MOVN/MOVK) -------------------------------
        // sf(31) opc(30-29) 100101(28-23) hw(22-21) imm16(20-5) Rd(4-0)
        if (w >> 23) & 0x3F == 0x25 {
            let sf = (w >> 31) & 1;
            let opc = (w >> 29) & 0x3;
            let hw = (w >> 21) & 0x3;
            let imm16 = ((w >> 5) & 0xFFFF) as u64;
            let rd = (w & 0x1F) as u8;
            let width64 = sf == 1;
            let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
            let shift = hw * 16;
            let imm = if shift >= 64 { 0 } else { imm16 << shift };
            let val = match opc {
                0 => !imm & mask, // MOVN
                2 => imm & mask,  // MOVZ
                3 => {
                    // MOVK
                    let cur = self.x(rd) & mask;
                    let keep = !(0xFFFFu64 << shift) & mask;
                    (cur & keep) | (imm & mask)
                }
                _ => return Err(Arm64Error::IllegalInstruction { pc, word: w }),
            };
            if rd == 31 {
                // MOV to XZR is a NOP-ish discard; SP unaffected.
            } else {
                self.set_x(rd, val);
            }
            self.steps += 1;
            return Ok(());
        }

        // ---- add/sub immediate ----------------------------------------------
        // sf op S 100010 sh imm12 Rn Rd
        if (w >> 23) & 0x3F == 0x22 {
            let sf = (w >> 31) & 1;
            let op = (w >> 30) & 1; // 0=ADD 1=SUB
            let s_bit = (w >> 29) & 1;
            let sh = (w >> 22) & 1;
            let imm12 = ((w >> 10) & 0xFFF) as u64;
            let rn = ((w >> 5) & 0x1F) as u8;
            let rd = (w & 0x1F) as u8;
            let width64 = sf == 1;
            let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
            let imm = if sh == 1 { imm12 << 12 } else { imm12 };
            let a = self.x(rn) & mask;
            let b = imm & mask;
            let (res, carry, overflow) = if op == 0 {
                let r = a.wrapping_add(b);
                let c = if width64 { r < a } else { (r & mask) < a };
                let sa = (a >> (if width64 { 63 } else { 31 })) & 1;
                let sb = (b >> (if width64 { 63 } else { 31 })) & 1;
                let sr = (r >> (if width64 { 63 } else { 31 })) & 1;
                (r & mask, c, sa == sb && sa != sr)
            } else {
                let r = a.wrapping_sub(b);
                let c = a >= b;
                let sa = (a >> (if width64 { 63 } else { 31 })) & 1;
                let sb = (b >> (if width64 { 63 } else { 31 })) & 1;
                let sr = (r >> (if width64 { 63 } else { 31 })) & 1;
                (r & mask, c, sa != sb && sa != sr)
            };
            self.set_x(rd, res);
            if s_bit == 1 || rd == 31 {
                self.n = res >> (if width64 { 63 } else { 31 }) & 1 == 1;
                self.z = res & mask == 0;
                self.c = carry;
                self.v = overflow;
            }
            self.steps += 1;
            return Ok(());
        }

        // ---- add/sub shifted register ---------------------------------------
        // sf op S 01011 shift 0 Rm imm6 Rn Rd
        if (w >> 24) & 0x1F == 0x0B && (w >> 21) & 1 == 0 {
            let sf = (w >> 31) & 1;
            let op = (w >> 30) & 1;
            let s_bit = (w >> 29) & 1;
            let shift = (w >> 22) & 0x3;
            let rm = ((w >> 16) & 0x1F) as u8;
            let imm6 = (w >> 10) & 0x3F;
            let rn = ((w >> 5) & 0x1F) as u8;
            let rd = (w & 0x1F) as u8;
            let width64 = sf == 1;
            let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
            let b = Self::shift(self.x(rm), shift, imm6, width64) & mask;
            let a = self.x(rn) & mask;
            let (res, carry, overflow) = if op == 0 {
                let r = a.wrapping_add(b);
                let c = if width64 { r < a } else { (r & mask) < a };
                let sa = (a >> (if width64 { 63 } else { 31 })) & 1;
                let sb = (b >> (if width64 { 63 } else { 31 })) & 1;
                let sr = (r >> (if width64 { 63 } else { 31 })) & 1;
                (r & mask, c, sa == sb && sa != sr)
            } else {
                let r = a.wrapping_sub(b);
                let c = a >= b;
                let sa = (a >> (if width64 { 63 } else { 31 })) & 1;
                let sb = (b >> (if width64 { 63 } else { 31 })) & 1;
                let sr = (r >> (if width64 { 63 } else { 31 })) & 1;
                (r & mask, c, sa != sb && sa != sr)
            };
            self.set_x(rd, res);
            if s_bit == 1 || rd == 31 {
                self.n = res >> (if width64 { 63 } else { 31 }) & 1 == 1;
                self.z = res & mask == 0;
                self.c = carry;
                self.v = overflow;
            }
            self.steps += 1;
            return Ok(());
        }

        // ---- logical shifted register ---------------------------------------
        // sf opc 01010 shift N Rm imm6 Rn Rd
        if (w >> 24) & 0x1F == 0x0A {
            let sf = (w >> 31) & 1;
            let opc = (w >> 29) & 0x3;
            let shift = (w >> 22) & 0x3;
            let n_bit = (w >> 21) & 1;
            let rm = ((w >> 16) & 0x1F) as u8;
            let imm6 = (w >> 10) & 0x3F;
            let rn = ((w >> 5) & 0x1F) as u8;
            let rd = (w & 0x1F) as u8;
            let width64 = sf == 1;
            let mask = if width64 { u64::MAX } else { 0xFFFF_FFFF };
            // N=0 → shifted register; N=1 → bitmask immediate (unsupported).
            if n_bit == 1 {
                return Err(Arm64Error::IllegalInstruction { pc, word: w });
            }
            let b = Self::shift(self.x(rm), shift, imm6, width64) & mask;
            let a = self.x(rn) & mask;
            let res = match opc {
                0 => a & b, // AND
                1 => a | b, // ORR (MOV reg alias)
                2 => a ^ b, // EOR
                3 => {
                    // ANDS
                    self.n = (a & b) >> (if width64 { 63 } else { 31 }) & 1 == 1;
                    self.z = (a & b) & mask == 0;
                    self.c = false;
                    self.v = false;
                    a & b
                }
                _ => unreachable!(),
            };
            // ANDS/TST with Rd=ZR discards the result.
            if !(opc == 3 && rd == 31) {
                self.set_x(rd, res & mask);
            }
            self.steps += 1;
            return Ok(());
        }

        // ---- loads / stores (unsigned offset) -------------------------------
        // size(31-30) 111 0 01 opc(23-22) imm12(21-10) Rn(9-5) Rt(4-0)
        if (w & 0x3F00_0000) == 0x3900_0000 {
            let size = w >> 30; // 0=B 1=H 2=W 3=X
            let opc = (w >> 22) & 0x3;
            let imm12 = ((w >> 10) & 0xFFF) as u64;
            let rn = ((w >> 5) & 0x1F) as u8;
            let rt = (w & 0x1F) as u8;
            let access = 1usize << size; // bytes: 1,2,4,8
            let scaled = imm12 << size;
            let base = if rn == 31 { self.sp } else { self.x(rn) };
            let addr = base.wrapping_add(scaled);

            match opc {
                0 => {
                    // STR
                    let val = self.x(rt);
                    self.mem_write(addr, val, access)?;
                }
                1 => {
                    // LDR (size=3 → X, size=2 → W zero-extend)
                    let raw = self.mem_read(addr, access)?;
                    let val = if size == 3 {
                        raw
                    } else {
                        // sign-extend for LDRSW (size=2,opc=11 handled below)
                        raw
                    };
                    if rt == 31 {
                        if size == 3 {
                            self.sp = val;
                        }
                    } else {
                        self.set_x(rt, val);
                    }
                }
                2 => {
                    // LDRS* / LDRSW — sign-extend
                    let raw = self.mem_read(addr, access)?;
                    let bits = access as u32 * 8;
                    let val = if size == 3 {
                        raw // LDRSW uses size=2 actually; treat as sign-ext of access width
                    } else {
                        Self::sext(raw, bits)
                    };
                    self.set_x(rt, val);
                }
                3 => {
                    // size==2 → LDRSW (sign-extend word); else PRFM (NOP)
                    if size == 2 {
                        let raw = self.mem_read(addr, 4)?;
                        self.set_x(rt, Self::sext(raw, 32));
                    }
                    // else: PRFM — treat as NOP
                }
                _ => unreachable!(),
            }
            self.steps += 1;
            return Ok(());
        }

        Err(Arm64Error::IllegalInstruction { pc, word: w })
    }

    /// Evaluate AArch64 condition code `cond` (0=EQ … 13=LE, 14=GT, 15=AL).
    #[inline]
    fn cond_true(&self, cond: u8) -> bool {
        match cond {
            0 => self.z,                       // EQ
            1 => !self.z,                      // NE
            2 => self.c,                       // CS/HS
            3 => !self.c,                      // CC/LO
            4 => self.n,                       // MI
            5 => !self.n,                      // PL
            6 => self.v,                       // VS
            7 => !self.v,                      // VC
            8 => self.c && !self.z,            // HI
            9 => !self.c || self.z,            // LS
            10 => self.n == self.v,            // GE
            11 => self.n != self.v,            // LT
            12 => !self.z && self.n == self.v, // GT
            13 => self.z || self.n != self.v,  // LE
            14 => true, // AL (14=always in B.cond? actually 14 is AL? no—14 unused, 15=AL)
            _ => true,  // 15 = AL
        }
    }

    /// Run until halt / error / `max_steps`. Returns steps executed.
    pub fn run(&mut self, max_steps: u64) -> Result<u64, Arm64Error> {
        while self.running && self.steps < max_steps {
            self.step()?;
        }
        Ok(self.steps)
    }
}

impl std::fmt::Debug for Arm64Cpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arm64Cpu")
            .field("regs", &self.regs)
            .field("sp", &self.sp)
            .field("pc", &self.pc)
            .field("steps", &self.steps)
            .field("running", &self.running)
            .field("memory_len", &self.memory.len())
            .finish_non_exhaustive()
    }
}

impl Default for Arm64Cpu {
    fn default() -> Self {
        Self::new(1024 * 1024)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble helpers for tests (real AArch64 encodings).
    fn movz_x(rd: u8, imm16: u16, shift: u32) -> u32 {
        0xD2800000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | rd as u32
    }
    fn movk_x(rd: u8, imm16: u16, shift: u32) -> u32 {
        0xF2800000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | rd as u32
    }
    fn svc(n: u16) -> u32 {
        0xD4000001 | ((n as u32) << 5)
    }
    fn str_x(rt: u8, rn: u8, imm12: u16) -> u32 {
        // STR Xt, [Xn, #imm12*8]
        0xF9000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rt as u32
    }
    fn ldr_x(rt: u8, rn: u8, imm12: u16) -> u32 {
        0xF9400000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rt as u32
    }
    fn nop() -> u32 {
        0xD503201F
    }

    fn cpu_with(code: &[u32]) -> Arm64Cpu {
        let mut c = Arm64Cpu::new(64 * 1024);
        for (i, &w) in code.iter().enumerate() {
            c.memory[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        c
    }

    fn brk() -> u32 {
        0xD4200000 // BRK #0 → halt
    }

    #[test]
    fn movz_movk_builds_64bit_constant() {
        // x0 = 0x123456789ABCDEF0 via MOVZ+MOVK chain
        let mut c = cpu_with(&[
            movz_x(0, 0xDEF0, 0),
            movk_x(0, 0x9ABC, 16),
            movk_x(0, 0x5678, 32),
            movk_x(0, 0x1234, 48),
            brk(),
        ]);
        c.run(16).unwrap();
        assert_eq!(c.regs[0], 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn store_load_roundtrip() {
        let mut c = cpu_with(&[
            str_x(0, 31, 0), // STR x0, [sp]
            ldr_x(1, 31, 0), // LDR x1, [sp]
            brk(),
        ]);
        c.regs[0] = 0xDEAD_BEEF_CAFE_F00D;
        c.sp = 0x8000;
        c.run(16).unwrap();
        assert_eq!(c.regs[1], 0xDEAD_BEEF_CAFE_F00D);
    }

    #[test]
    fn svc_calls_handler() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut c = cpu_with(&[svc(0), svc(1), brk()]);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls2 = calls.clone();
        c.svc_handler = Some(Box::new(move |_cpu, call| {
            calls2.borrow_mut().push(call.number);
            true
        }));
        c.run(16).unwrap();
        assert_eq!(*calls.borrow(), vec![0, 1]);
    }

    #[test]
    fn illegal_instruction_errors() {
        let mut c = cpu_with(&[0xFFFFFFFF, nop()]);
        let r = c.step();
        assert!(matches!(r, Err(Arm64Error::IllegalInstruction { .. })));
    }

    #[test]
    fn branch_link_sets_lr() {
        // BL +8 → word index 2; LR must be return address (pc+4).
        let mut c = cpu_with(&[
            0x94000002, // BL +8 → index 2 (imm26=2, PC-relativ)
            nop(),
            movz_x(0, 42, 0),
            brk(),
        ]);
        c.step().unwrap(); // BL
        assert_eq!(c.regs[30], 4);
        assert_eq!(c.pc, 8);
    }

    #[test]
    fn logical_and_orr_eor() {
        // x0 = 0xFF00FF00FF00FF00; x1 = 0x0000FFFF0000FFFF
        // AND → 0x0000FF000000FF00 ; ORR → 0xFFFF00FF00FF00FF? compute:
        //   a & b, a | b, a ^ b verified numerically below.
        let a: u64 = 0xFF00_FF00_FF00_FF00;
        let b: u64 = 0x0000_FFFF_0000_FFFF;
        // AND X0, X0, X1 : sf=1 opc=00 01010 shift=00 N=0 Rm=1 imm6=0 Rn=0 Rd=0
        // AND X4, X0, X1 / ORR X5, X0, X1 / EOR X6, X0, X1 — all read
        // x0/x1 untouched (AND writes x4, not x0).
        let and = 0x8A000000u32 | (1 << 16) | 4;
        let orr = 0xAA000000u32 | (1 << 16) | 5;
        let eor = 0xCA000000u32 | (1 << 16) | 6;
        let mut c = cpu_with(&[and, orr, eor, brk()]);
        c.regs[0] = a;
        c.regs[1] = b;
        c.run(16).unwrap();
        assert_eq!(c.regs[4], a & b);
        assert_eq!(c.regs[5], a | b);
        assert_eq!(c.regs[6], a ^ b);
    }

    #[test]
    fn add_sub_shifted_reg() {
        // ADDS Xd, Xn, Xm (shifted reg): sf=1 op=0 S=1 01011 00 0 Rm imm6 Rn Rd
        let adds = 0xAB000000u32 | (1 << 16) | 2; // x2 = x0 + x1
        let subs = 0xEB000000u32 | (1 << 16) | 3; // x3 = x0 - x1
        let mut c = cpu_with(&[movz_x(0, 5, 0), movz_x(1, 3, 0), adds, subs, brk()]);
        c.run(16).unwrap();
        assert_eq!(c.regs[2], 8);
        assert_eq!(c.regs[3], 2);
    }
}
