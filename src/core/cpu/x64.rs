//! Guest x86-64 execution subset — Phase 29.
//!
//! This is the CPU layer of the guest execution pipeline:
//!
//! ```text
//! PE loader → import resolution → X64Cpu → HLE dispatch → results
//! ```
//!
//! # What this is (and what it is not)
//!
//! A **small, explicitly bounded** x86-64 interpreter used to execute
//! synthetic PE32+ guests. It is not a complete x86-64 emulator: only the
//! encodings listed below decode, and anything else raises
//! [`CpuFault::IllegalInstruction`] instead of being guessed at. The
//! other CPU layers in this crate are separate and unrelated:
//! [`super::interpreter`] is an AArch64 subset used by the game demos,
//! [`super::Cpu`] is the Phase-1 scaffold decoder driving the GUI loop,
//! and the x86-64 → WASM JIT research lives in [`crate::jit`].
//!
//! # Supported encodings
//!
//! | Class | Encodings |
//! |---|---|
//! | Data movement | `MOV r32/r64, imm` (`B8+r`, `REX.W B8+r`), `MOV r/m, r` (`89`), `MOV r, r/m` (`8B`), `MOV r/m, imm32` (`C7 /0`, sign-extended) |
//! | Integer ALU | `ADD OR AND SUB XOR CMP` via `83 /n imm8`, `81 /n imm32`, `01/09/21/29/31/39` (r/m ← r), `03/0B/23/2B/33/3B` (r ← r/m) |
//! | Compare | `TEST r/m, r` (`85`) |
//! | Branches | `JMP rel8/rel32` (`EB`, `E9`), `Jcc rel8/rel32` (`70..7F`, `0F 80..8F`), `CALL rel32` (`E8`), `CALL r/m64` (`FF /2`) |
//! | Stack | `PUSH r64` (`50+r`), `POP r64` (`58+r`), `RET` (`C3`) |
//! | System | `NOP` (`90`), `HLT` (`F4`) |
//!
//! Addressing modes: register, `[reg]`, `[reg+disp8/32]`, SIB
//! (`[reg+index*scale+disp]`) and RIP-relative. `ADC`/`SBB`, byte/word
//! operands, `LEA`, `INT`/`INT3`, far jumps and the parity-based jumps
//! `JP`/`JNP` are **not** implemented and fault explicitly.
//!
//! # Flags
//!
//! `CF`, `ZF`, `SF` and `OF` are modelled for the ALU operations above.
//! `PF`, `AF`, `DF` and `IF` are not modelled — which is exactly why
//! `JP`/`JNP` fault instead of silently reading an unmodelled flag.
//!
//! # Calling-convention seam
//!
//! Arguments are read from `RCX, RDX, R8, R9` and the return value is
//! written to `RAX`: the documented Microsoft x64 calling convention
//! (<https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention>).
//! It is also the convention this project already relies on elsewhere
//! (`src/core/boot.rs`: "x86-64 host ABI: Win64 passes arg0 in RCX").
//! xHoly does **not** claim any Xbox-specific register usage; if a title
//! is ever shown to use a different convention, this seam is the single
//! place to change.
//!
//! # HLE transition points
//!
//! An indirect `CALL` whose memory operand addresses a registered HLE
//! slot ([`X64Cpu::set_hle_slots`]) does not jump anywhere: it pushes
//! the return address and returns [`StepOutcome::HleCall`], so the
//! pipeline can dispatch the import. The CPU never calls the HLE runtime
//! itself and never invents a return value for it.

use std::collections::BTreeSet;
use std::fmt;

use crate::core::memory::GuestMemory;
use serde::{Deserialize, Serialize};

/// Number of general-purpose registers (RAX..R15).
pub const REGISTER_COUNT: usize = 16;

/// Register indices in x86-64 encoding order.
pub mod reg {
    /// Accumulator / return value.
    pub const RAX: usize = 0;
    /// Counter / argument 0.
    pub const RCX: usize = 1;
    /// Data / argument 1.
    pub const RDX: usize = 2;
    /// Base.
    pub const RBX: usize = 3;
    /// Stack pointer.
    pub const RSP: usize = 4;
    /// Frame pointer.
    pub const RBP: usize = 5;
    /// Source index.
    pub const RSI: usize = 6;
    /// Destination index.
    pub const RDI: usize = 7;
    /// Argument 2.
    pub const R8: usize = 8;
    /// Argument 3.
    pub const R9: usize = 9;
    /// Scratch.
    pub const R10: usize = 10;
    /// Scratch.
    pub const R11: usize = 11;
    /// Scratch.
    pub const R12: usize = 12;
    /// Callee-saved.
    pub const R13: usize = 13;
    /// Callee-saved.
    pub const R14: usize = 14;
    /// Callee-saved.
    pub const R15: usize = 15;
}

const REGISTER_NAMES: [&str; REGISTER_COUNT] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// Registers holding arguments 0..3 of the Microsoft x64 convention.
pub const ARG_REGISTERS: [usize; 4] = [reg::RCX, reg::RDX, reg::R8, reg::R9];

/// Register holding the return value (Microsoft x64 convention).
pub const RETURN_REGISTER: usize = reg::RAX;

/// Carry flag.
pub const FLAG_CF: u64 = 1 << 0;
/// Zero flag.
pub const FLAG_ZF: u64 = 1 << 6;
/// Sign flag.
pub const FLAG_SF: u64 = 1 << 7;
/// Overflow flag.
pub const FLAG_OF: u64 = 1 << 11;
/// RFLAGS bit 1 is reserved and always reads as 1.
pub const FLAG_RESERVED: u64 = 1 << 1;

/// Name of a general-purpose register (for diagnostics).
pub fn register_name(index: usize) -> &'static str {
    REGISTER_NAMES.get(index).copied().unwrap_or("?")
}

/// A CPU fault raised while stepping guest code.
///
/// Deserializable since M34 so exported traces can be imported into the
/// GUI. Fault reasons are owned `String`s (the decoder's static messages
/// are copied), so no host pointer or borrowed data is ever in a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuFault {
    /// Instruction fetch left mapped guest memory.
    FetchOutOfBounds {
        /// Address of the instruction that could not be fetched.
        rip: u64,
    },
    /// Encoding outside the supported subset.
    IllegalInstruction {
        /// Address of the faulting instruction.
        rip: u64,
        /// Primary opcode byte.
        opcode: u8,
        /// Why the encoding was rejected.
        reason: String,
    },
    /// A data access was refused by guest memory.
    Memory {
        /// Guest address that failed.
        addr: u64,
        /// `true` for a write, `false` for a read.
        write: bool,
        /// Address of the faulting instruction.
        rip: u64,
    },
    /// A stack push or call moved `RSP` below the configured stack bottom.
    StackOverflow {
        /// Target `RSP` that overflowed.
        rsp: u64,
        /// Lower bound of the guest stack (inclusive).
        bottom: u64,
        /// Upper bound of the guest stack (exclusive).
        top: u64,
        /// Address of the faulting instruction.
        rip: u64,
    },
    /// A stack pop or return read at or above the configured stack top.
    StackUnderflow {
        /// `RSP` at the time of the underflow.
        rsp: u64,
        /// Lower bound of the guest stack (inclusive).
        bottom: u64,
        /// Upper bound of the guest stack (exclusive).
        top: u64,
        /// Address of the faulting instruction.
        rip: u64,
    },
    /// `RSP` violated a required stack alignment constraint.
    MisalignedStack {
        /// Observed `RSP`.
        rsp: u64,
        /// Required alignment in bytes.
        required_align: u64,
        /// Address of the faulting instruction.
        rip: u64,
    },
}

/// Configured guest stack bounds `[bottom, top)` in guest virtual address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StackBounds {
    /// Lowest valid stack byte address (inclusive).
    pub bottom: u64,
    /// One byte past the highest valid stack byte address (exclusive).
    pub top: u64,
}

impl StackBounds {
    /// Create validated stack bounds; returns `None` if `bottom >= top`.
    pub fn new(bottom: u64, top: u64) -> Option<Self> {
        if bottom < top {
            Some(Self { bottom, top })
        } else {
            None
        }
    }

    /// Total stack size in bytes.
    pub fn size(&self) -> u64 {
        self.top - self.bottom
    }

    /// True when `[addr, addr + len)` lies completely within `[bottom, top)`.
    pub fn contains_range(&self, addr: u64, len: u64) -> bool {
        addr >= self.bottom && addr.checked_add(len).is_some_and(|end| end <= self.top)
    }
}

impl fmt::Display for CpuFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FetchOutOfBounds { rip } => {
                write!(f, "instruction fetch out of bounds at {rip:#x}")
            }
            Self::IllegalInstruction {
                rip,
                opcode,
                reason,
            } => write!(
                f,
                "unsupported x86-64 instruction {opcode:#04x} ({reason}) at {rip:#x}"
            ),
            Self::Memory { addr, write, rip } => write!(
                f,
                "guest memory {} failed at {addr:#x} (instruction at {rip:#x})",
                if *write { "write" } else { "read" }
            ),
            Self::StackOverflow {
                rsp,
                bottom,
                top,
                rip,
            } => write!(
                f,
                "guest stack overflow at rsp={rsp:#x} (bounds {bottom:#x}..{top:#x}, instruction at {rip:#x})"
            ),
            Self::StackUnderflow {
                rsp,
                bottom,
                top,
                rip,
            } => write!(
                f,
                "guest stack underflow at rsp={rsp:#x} (bounds {bottom:#x}..{top:#x}, instruction at {rip:#x})"
            ),
            Self::MisalignedStack {
                rsp,
                required_align,
                rip,
            } => write!(
                f,
                "misaligned guest stack rsp={rsp:#x} (required {required_align}-byte alignment, instruction at {rip:#x})"
            ),
        }
    }
}

impl std::error::Error for CpuFault {}

/// Immutable view of the CPU state (diagnostics and reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuSnapshot {
    /// Instruction pointer.
    pub rip: u64,
    /// Flags register.
    pub rflags: u64,
    /// Whether `HLT` has been executed.
    pub halted: bool,
    /// Instructions retired (faulting instructions are not counted).
    pub instructions: u64,
    /// General-purpose registers.
    pub regs: [u64; REGISTER_COUNT],
}

impl CpuSnapshot {
    /// One register by index.
    pub fn register(&self, index: usize) -> u64 {
        self.regs.get(index).copied().unwrap_or(0)
    }

    /// Multi-line rendering for diagnostics.
    pub fn render(&self) -> String {
        let mut out = format!(
            "rip={:#x} rflags={:#x} instructions={} halted={}\n",
            self.rip, self.rflags, self.instructions, self.halted
        );
        for (i, name) in REGISTER_NAMES.iter().enumerate() {
            if i % 4 == 0 && i > 0 {
                out.push('\n');
            } else if i > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{name}={:#018x}", self.regs[i]));
        }
        out
    }
}

/// Result of one instruction step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// The instruction executed normally.
    Executed,
    /// `HLT` was executed; the CPU is halted.
    Halted,
    /// Indirect call through a registered HLE transition slot: the
    /// return address has been pushed and `rip` already points at it.
    /// The caller decides what the call returns.
    HleCall {
        /// Guest address of the IAT slot that was called through.
        slot_addr: u64,
    },
    /// The instruction could not be executed.
    Fault(CpuFault),
}

/// Internal control flow of one instruction.
enum Flow {
    /// Continue with the next instruction.
    Next,
    /// Continue at this address.
    Jump(u64),
    /// `HLT` executed.
    Halted,
    /// HLE transition through this slot address.
    HleCall(u64),
    /// Fault; `rip` is left untouched.
    Fault(CpuFault),
}

/// Instruction fetch cursor.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    rip: u64,
    start: u64,
}

/// A decoded operand: either a register or a memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Reg(usize),
    Mem {
        /// Absolute effective address (non-RIP-relative forms).
        ea: u64,
        /// RIP-relative displacement, resolved against the address of
        /// the *next* instruction once all immediates are consumed.
        rip_rel: Option<i64>,
    },
}

impl Target {
    /// Resolve to a guest address, or `None` for register operands.
    fn addr(&self, next_rip: u64) -> Option<u64> {
        match self {
            Self::Reg(_) => None,
            Self::Mem { ea, rip_rel } => Some(match rip_rel {
                Some(disp) => next_rip.wrapping_add(*disp as u64),
                None => *ea,
            }),
        }
    }
}

fn fetch_bytes<M: GuestMemory>(mem: &M, cur: &mut Cursor, len: usize) -> Result<Vec<u8>, Flow> {
    match mem.read_bytes(cur.rip, len) {
        Some(bytes) if bytes.len() == len => {
            cur.rip = cur.rip.wrapping_add(len as u64);
            Ok(bytes)
        }
        _ => Err(Flow::Fault(CpuFault::FetchOutOfBounds { rip: cur.start })),
    }
}

fn fetch_u8<M: GuestMemory>(mem: &M, cur: &mut Cursor) -> Result<u8, Flow> {
    Ok(fetch_bytes(mem, cur, 1)?[0])
}

fn fetch_u32<M: GuestMemory>(mem: &M, cur: &mut Cursor) -> Result<u32, Flow> {
    let b = fetch_bytes(mem, cur, 4)?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn fetch_u64<M: GuestMemory>(mem: &M, cur: &mut Cursor) -> Result<u64, Flow> {
    let b = fetch_bytes(mem, cur, 8)?;
    Ok(u64::from_le_bytes(b.try_into().unwrap()))
}

fn illegal(rip: u64, opcode: u8, reason: &str) -> Flow {
    Flow::Fault(CpuFault::IllegalInstruction {
        rip,
        opcode,
        reason: reason.to_string(),
    })
}

/// The x86-64 subset interpreter.
#[derive(Debug, Clone)]
pub struct X64Cpu {
    /// General-purpose registers in encoding order.
    pub regs: [u64; REGISTER_COUNT],
    /// Instruction pointer.
    pub rip: u64,
    /// Flags register.
    pub rflags: u64,
    /// Set once `HLT` executed.
    pub halted: bool,
    /// Instructions retired.
    pub instructions: u64,
    /// Guest addresses whose indirect call is an HLE transition point.
    hle_slots: BTreeSet<u64>,
    /// Optional stack bounds `[bottom, top)` enforced on push/pop/call/ret.
    stack_bounds: Option<StackBounds>,
}

impl Default for X64Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl X64Cpu {
    /// A fresh CPU: all registers zero, `rip = 0`, RFLAGS = reserved bit.
    pub fn new() -> Self {
        Self {
            regs: [0; REGISTER_COUNT],
            rip: 0,
            rflags: FLAG_RESERVED,
            halted: false,
            instructions: 0,
            hle_slots: BTreeSet::new(),
            stack_bounds: None,
        }
    }

    /// Configure explicit `[bottom, top)` bounds for stack operations.
    pub fn set_stack_bounds(&mut self, bounds: Option<StackBounds>) {
        self.stack_bounds = bounds;
    }

    /// Configured guest stack bounds, if any.
    pub fn stack_bounds(&self) -> Option<StackBounds> {
        self.stack_bounds
    }

    /// Registers holding the first `count` arguments (max 4).
    ///
    /// See the module docs for the calling-convention evidence. Calls
    /// never receive more than the four integer argument registers.
    pub fn read_args(&self, count: usize) -> Vec<u64> {
        ARG_REGISTERS
            .iter()
            .take(count.min(ARG_REGISTERS.len()))
            .map(|&index| self.regs[index])
            .collect()
    }

    /// Write the return value into `RAX`.
    pub fn write_return(&mut self, value: u64) {
        self.regs[RETURN_REGISTER] = value;
    }

    /// Current `RAX` (the return-value register).
    pub fn return_value(&self) -> u64 {
        self.regs[RETURN_REGISTER]
    }

    /// Register the guest addresses whose indirect call is an HLE
    /// transition point (the IAT slots produced by import resolution).
    pub fn set_hle_slots<I: IntoIterator<Item = u64>>(&mut self, slots: I) {
        self.hle_slots = slots.into_iter().collect();
    }

    /// Registered HLE transition slots.
    pub fn hle_slots(&self) -> impl Iterator<Item = u64> + '_ {
        self.hle_slots.iter().copied()
    }

    /// Complete an HLE transition: pop the return address that the
    /// [`StepOutcome::HleCall`] outcome pushed, exactly as the callee's
    /// `ret` would. The pipeline plays the callee half of the call, so
    /// the stack stays balanced across the transition.
    pub fn finish_hle_call<M: GuestMemory>(&mut self, mem: &M) -> Result<u64, CpuFault> {
        let rip = self.rip;
        self.pop(mem, rip)
    }

    /// Immutable view of the CPU state.
    pub fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot {
            rip: self.rip,
            rflags: self.rflags,
            halted: self.halted,
            instructions: self.instructions,
            regs: self.regs,
        }
    }

    /// Execute one instruction at `rip`.
    ///
    /// Never panics on bad guest input: every refusal is a structured
    /// [`CpuFault`].
    pub fn step<M: GuestMemory>(&mut self, mem: &mut M) -> StepOutcome {
        if self.halted {
            return StepOutcome::Halted;
        }
        let mut cur = Cursor {
            rip: self.rip,
            start: self.rip,
        };
        match self.execute(mem, &mut cur) {
            Flow::Fault(fault) => StepOutcome::Fault(fault),
            Flow::Next => {
                self.rip = cur.rip;
                self.instructions += 1;
                StepOutcome::Executed
            }
            Flow::Jump(target) => {
                self.rip = target;
                self.instructions += 1;
                StepOutcome::Executed
            }
            Flow::Halted => {
                self.rip = cur.rip;
                self.halted = true;
                self.instructions += 1;
                StepOutcome::Halted
            }
            Flow::HleCall(slot_addr) => {
                self.rip = cur.rip;
                self.instructions += 1;
                StepOutcome::HleCall { slot_addr }
            }
        }
    }

    /// Decode and execute one instruction.
    fn execute<M: GuestMemory>(&mut self, mem: &mut M, cur: &mut Cursor) -> Flow {
        let start = cur.start;
        // A REX prefix must directly precede the opcode.
        let mut rex: u8 = 0;
        let mut opcode = match fetch_u8(mem, cur) {
            Ok(b) => b,
            Err(flow) => return flow,
        };
        while (0x40..=0x4F).contains(&opcode) {
            rex = opcode;
            opcode = match fetch_u8(mem, cur) {
                Ok(b) => b,
                Err(flow) => return flow,
            };
        }
        let width: u8 = if rex & 0x8 != 0 { 8 } else { 4 };
        let mask: u64 = if width == 8 {
            u64::MAX
        } else {
            u32::MAX as u64
        };

        match opcode {
            // NOP
            0x90 => Flow::Next,
            // HLT
            0xF4 => Flow::Halted,
            // RET
            0xC3 => match self.pop(mem, start) {
                Ok(target) => Flow::Jump(target),
                Err(fault) => Flow::Fault(fault),
            },
            // PUSH r64
            0x50..=0x57 => {
                let index = (opcode - 0x50) as usize + if rex & 0x1 != 0 { 8 } else { 0 };
                let value = self.regs[index];
                match self.push(mem, value, start) {
                    Ok(()) => Flow::Next,
                    Err(fault) => Flow::Fault(fault),
                }
            }
            // POP r64
            0x58..=0x5F => {
                let index = (opcode - 0x58) as usize + if rex & 0x1 != 0 { 8 } else { 0 };
                match self.pop(mem, start) {
                    Ok(value) => {
                        self.regs[index] = value;
                        Flow::Next
                    }
                    Err(fault) => Flow::Fault(fault),
                }
            }
            // MOV r32/r64, imm32/imm64
            0xB8..=0xBF => {
                let index = (opcode - 0xB8) as usize + if rex & 0x1 != 0 { 8 } else { 0 };
                let value = match width {
                    8 => match fetch_u64(mem, cur) {
                        Ok(v) => v,
                        Err(flow) => return flow,
                    },
                    _ => match fetch_u32(mem, cur) {
                        Ok(v) => u64::from(v),
                        Err(flow) => return flow,
                    },
                };
                self.regs[index] = value;
                Flow::Next
            }
            // MOV r/m, r
            0x89 => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let value = self.regs[reg_field as usize];
                match self.write_target(mem, target, cur.rip, start, width, value) {
                    Ok(()) => Flow::Next,
                    Err(fault) => Flow::Fault(fault),
                }
            }
            // MOV r, r/m
            0x8B => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let value = match self.read_target(mem, target, cur.rip, start, width) {
                    Ok(v) => v,
                    Err(fault) => return Flow::Fault(fault),
                };
                self.regs[reg_field as usize] = value & mask;
                Flow::Next
            }
            // MOV r/m, imm32 (group 3, /0 only; sign-extended to 64 bits)
            0xC7 => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                if reg_field != 0 {
                    return illegal(start, opcode, "group 3: only /0 MOV is implemented");
                }
                let immediate = match fetch_u32(mem, cur) {
                    Ok(v) => v as i32 as i64 as u64,
                    Err(flow) => return flow,
                };
                match self.write_target(mem, target, cur.rip, start, width, immediate) {
                    Ok(()) => Flow::Next,
                    Err(fault) => Flow::Fault(fault),
                }
            }
            // TEST r/m, r — flags only, like AND without the write-back.
            0x85 => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let lhs = match self.read_target(mem, target, cur.rip, start, width) {
                    Ok(v) => v,
                    Err(fault) => return Flow::Fault(fault),
                };
                let rhs = self.regs[reg_field as usize];
                // Index 4 (AND) sets exactly the flags TEST sets.
                let _ = self.alu(4, lhs, rhs, width);
                Flow::Next
            }
            // ALU r/m, imm8 / imm32
            0x83 | 0x81 => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let immediate = if opcode == 0x83 {
                    match fetch_u8(mem, cur) {
                        Ok(v) => v as i8 as i64 as u64,
                        Err(flow) => return flow,
                    }
                } else {
                    match fetch_u32(mem, cur) {
                        Ok(v) => v as i32 as i64 as u64,
                        Err(flow) => return flow,
                    }
                };
                let lhs = match self.read_target(mem, target, cur.rip, start, width) {
                    Ok(v) => v,
                    Err(fault) => return Flow::Fault(fault),
                };
                match self.alu(reg_field, lhs, immediate, width) {
                    None => illegal(start, opcode, "unsupported ALU sub-opcode (ADC/SBB)"),
                    Some(result) => {
                        if reg_field != 7 {
                            // CMP does not write back.
                            match self.write_target(mem, target, cur.rip, start, width, result) {
                                Ok(()) => Flow::Next,
                                Err(fault) => Flow::Fault(fault),
                            }
                        } else {
                            Flow::Next
                        }
                    }
                }
            }
            // ALU r/m ← r/m op r
            0x01 | 0x09 | 0x21 | 0x29 | 0x31 | 0x39 => {
                let index = (opcode >> 3) & 0x7;
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let lhs = match self.read_target(mem, target, cur.rip, start, width) {
                    Ok(v) => v,
                    Err(fault) => return Flow::Fault(fault),
                };
                let rhs = self.regs[reg_field as usize];
                match self.alu(index, lhs, rhs, width) {
                    None => illegal(start, opcode, "unsupported ALU sub-opcode"),
                    Some(result) => {
                        if index != 7 {
                            match self.write_target(mem, target, cur.rip, start, width, result) {
                                Ok(()) => Flow::Next,
                                Err(fault) => Flow::Fault(fault),
                            }
                        } else {
                            Flow::Next
                        }
                    }
                }
            }
            // ALU r ← r op r/m
            0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B => {
                let index = (opcode >> 3) & 0x7;
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                let lhs = self.regs[reg_field as usize];
                let rhs = match self.read_target(mem, target, cur.rip, start, width) {
                    Ok(v) => v,
                    Err(fault) => return Flow::Fault(fault),
                };
                match self.alu(index, lhs, rhs, width) {
                    None => illegal(start, opcode, "unsupported ALU sub-opcode"),
                    Some(result) => {
                        if index != 7 {
                            self.regs[reg_field as usize] = result & mask;
                            Flow::Next
                        } else {
                            Flow::Next
                        }
                    }
                }
            }
            // CALL rel32
            0xE8 => {
                let rel = match fetch_u32(mem, cur) {
                    Ok(v) => v as i32,
                    Err(flow) => return flow,
                };
                let target = cur.rip.wrapping_add(rel as u64);
                match self.push(mem, cur.rip, start) {
                    Ok(()) => Flow::Jump(target),
                    Err(fault) => Flow::Fault(fault),
                }
            }
            // JMP rel32
            0xE9 => match fetch_u32(mem, cur) {
                Ok(rel) => Flow::Jump(cur.rip.wrapping_add(rel as i32 as u64)),
                Err(flow) => flow,
            },
            // JMP rel8
            0xEB => match fetch_u8(mem, cur) {
                Ok(rel) => Flow::Jump(cur.rip.wrapping_add(rel as i8 as u64)),
                Err(flow) => flow,
            },
            // Jcc rel8
            0x70..=0x7F => {
                let rel = match fetch_u8(mem, cur) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                match self.condition(opcode & 0xF) {
                    None => illegal(start, opcode, "parity flag is not modelled"),
                    Some(true) => Flow::Jump(cur.rip.wrapping_add(rel as i8 as u64)),
                    Some(false) => Flow::Next,
                }
            }
            // Two-byte opcode escape.
            0x0F => {
                let second = match fetch_u8(mem, cur) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                match second {
                    // Jcc rel32
                    0x80..=0x8F => {
                        let rel = match fetch_u32(mem, cur) {
                            Ok(v) => v,
                            Err(flow) => return flow,
                        };
                        match self.condition(second - 0x80) {
                            None => illegal(start, opcode, "parity flag is not modelled"),
                            Some(true) => Flow::Jump(cur.rip.wrapping_add(rel as i32 as u64)),
                            Some(false) => Flow::Next,
                        }
                    }
                    _ => illegal(start, second, "unsupported two-byte opcode"),
                }
            }
            // Group 5: only /2 CALL r/m64 is implemented.
            0xFF => {
                let (reg_field, target) = match self.decode_target(mem, cur, rex) {
                    Ok(v) => v,
                    Err(flow) => return flow,
                };
                if reg_field != 2 {
                    return illegal(start, opcode, "group 5: only /2 CALL is implemented");
                }
                match target {
                    Target::Reg(index) => {
                        let call_target = self.regs[index];
                        match self.push(mem, cur.rip, start) {
                            Ok(()) => Flow::Jump(call_target),
                            Err(fault) => Flow::Fault(fault),
                        }
                    }
                    Target::Mem { .. } => {
                        let addr = match target.addr(cur.rip) {
                            Some(a) => a,
                            None => return illegal(start, opcode, "unresolvable call target"),
                        };
                        if self.hle_slots.contains(&addr) {
                            // HLE transition: push the return address and
                            // let the pipeline dispatch the import. The
                            // stale slot contents are never read.
                            return match self.push(mem, cur.rip, start) {
                                Ok(()) => Flow::HleCall(addr),
                                Err(fault) => Flow::Fault(fault),
                            };
                        }
                        let call_target = match mem.read_u64(addr) {
                            Some(v) => v,
                            None => {
                                return Flow::Fault(CpuFault::Memory {
                                    addr,
                                    write: false,
                                    rip: start,
                                })
                            }
                        };
                        match self.push(mem, cur.rip, start) {
                            Ok(()) => Flow::Jump(call_target),
                            Err(fault) => Flow::Fault(fault),
                        }
                    }
                }
            }
            _ => illegal(
                start,
                opcode,
                "opcode is outside the supported x86-64 subset",
            ),
        }
    }

    /// Decode a ModRM byte (plus SIB/disp) into a register or address.
    ///
    /// Returns `(reg field, operand)`. RIP-relative operands stay
    /// unresolved until the instruction's immediates are consumed.
    fn decode_target<M: GuestMemory>(
        &self,
        mem: &M,
        cur: &mut Cursor,
        rex: u8,
    ) -> Result<(u8, Target), Flow> {
        let byte = fetch_u8(mem, cur)?;
        let mode = byte >> 6;
        let reg_field = ((byte >> 3) & 0x7) + if rex & 0x4 != 0 { 8 } else { 0 };
        let rm = byte & 0x7;
        if mode == 3 {
            let index = rm as usize + if rex & 0x1 != 0 { 8 } else { 0 };
            return Ok((reg_field, Target::Reg(index)));
        }

        let mut displacement: i64 = 0;
        let mut rip_rel: Option<i64> = None;
        let mut base_value: Option<u64> = None;
        let mut index_value: Option<u64> = None;

        if rm == 4 {
            // SIB byte follows.
            let sib = fetch_u8(mem, cur)?;
            let scale = 1u64 << (sib >> 6);
            let index_field = (sib >> 3) & 0x7;
            let base_field = sib & 0x7;
            // index == RSP (4) without REX.X means "no index".
            if index_field != 4 || rex & 0x2 != 0 {
                let index = index_field as usize + if rex & 0x2 != 0 { 8 } else { 0 };
                index_value = Some(self.regs[index].wrapping_mul(scale));
            }
            if base_field == 5 && mode == 0 {
                // disp32 with no base register.
                displacement += fetch_u32(mem, cur)? as i32 as i64;
            } else {
                let base = base_field as usize + if rex & 0x1 != 0 { 8 } else { 0 };
                base_value = Some(self.regs[base]);
            }
        } else if rm == 5 && mode == 0 {
            // RIP-relative: resolved against the next instruction.
            rip_rel = Some(fetch_u32(mem, cur)? as i32 as i64);
        } else {
            let base = rm as usize + if rex & 0x1 != 0 { 8 } else { 0 };
            base_value = Some(self.regs[base]);
        }

        match mode {
            0 => {}
            1 => displacement += fetch_u8(mem, cur)? as i8 as i64,
            2 => displacement += fetch_u32(mem, cur)? as i32 as i64,
            _ => unreachable!("mode 3 returns earlier"),
        }

        let mut ea: i64 = displacement;
        if let Some(base) = base_value {
            ea = ea.wrapping_add(base as i64);
        }
        if let Some(index) = index_value {
            ea = ea.wrapping_add(index as i64);
        }
        Ok((
            reg_field,
            Target::Mem {
                ea: ea as u64,
                rip_rel,
            },
        ))
    }

    /// Read an operand (register or memory) as `width` bytes.
    fn read_target<M: GuestMemory>(
        &self,
        mem: &M,
        target: Target,
        next_rip: u64,
        rip: u64,
        width: u8,
    ) -> Result<u64, CpuFault> {
        match target {
            Target::Reg(index) => Ok(self.regs[index]),
            Target::Mem { .. } => {
                let addr = target.addr(next_rip).ok_or(CpuFault::IllegalInstruction {
                    rip,
                    opcode: 0,
                    reason: "unresolvable memory operand".into(),
                })?;
                let bytes = mem
                    .read_bytes(addr, width as usize)
                    .ok_or(CpuFault::Memory {
                        addr,
                        write: false,
                        rip,
                    })?;
                let mut value = 0u64;
                for (i, byte) in bytes.iter().enumerate() {
                    value |= u64::from(*byte) << (8 * i);
                }
                Ok(value)
            }
        }
    }

    /// Write an operand (register or memory) as `width` bytes.
    fn write_target<M: GuestMemory>(
        &mut self,
        mem: &mut M,
        target: Target,
        next_rip: u64,
        rip: u64,
        width: u8,
        value: u64,
    ) -> Result<(), CpuFault> {
        match target {
            Target::Reg(index) => {
                // Narrower writes zero-extend, as x86-64 requires.
                self.regs[index] = if width == 8 {
                    value
                } else {
                    value & u32::MAX as u64
                };
                Ok(())
            }
            Target::Mem { .. } => {
                let addr = target.addr(next_rip).ok_or(CpuFault::IllegalInstruction {
                    rip,
                    opcode: 0,
                    reason: "unresolvable memory operand".into(),
                })?;
                let bytes = value.to_le_bytes();
                let len = width as usize;
                if !mem.write_bytes(addr, &bytes[..len]) {
                    return Err(CpuFault::Memory {
                        addr,
                        write: true,
                        rip,
                    });
                }
                Ok(())
            }
        }
    }

    /// Push a `u64` onto the guest stack (grows downwards).
    ///
    /// `rip` is only used to describe a fault. Refuses with a
    /// structured [`CpuFault`] when the target is not mapped — guest
    /// input can never cause a panic here.
    pub fn push<M: GuestMemory>(
        &mut self,
        mem: &mut M,
        value: u64,
        rip: u64,
    ) -> Result<(), CpuFault> {
        let Some(new_rsp) = self.regs[reg::RSP].checked_sub(8) else {
            return Err(CpuFault::StackOverflow {
                rsp: 0,
                bottom: self.stack_bounds.map_or(0, |b| b.bottom),
                top: self.stack_bounds.map_or(0, |b| b.top),
                rip,
            });
        };
        if let Some(bounds) = self.stack_bounds {
            if new_rsp < bounds.bottom {
                return Err(CpuFault::StackOverflow {
                    rsp: new_rsp,
                    bottom: bounds.bottom,
                    top: bounds.top,
                    rip,
                });
            }
            if new_rsp.checked_add(8).is_none_or(|end| end > bounds.top) {
                return Err(CpuFault::StackUnderflow {
                    rsp: new_rsp,
                    bottom: bounds.bottom,
                    top: bounds.top,
                    rip,
                });
            }
        }
        if !mem.write_u64(new_rsp, value) {
            return Err(CpuFault::Memory {
                addr: new_rsp,
                write: true,
                rip,
            });
        }
        self.regs[reg::RSP] = new_rsp;
        Ok(())
    }

    /// Pop a `u64` from the guest stack.
    fn pop<M: GuestMemory>(&mut self, mem: &M, rip: u64) -> Result<u64, CpuFault> {
        let rsp = self.regs[reg::RSP];
        if let Some(bounds) = self.stack_bounds {
            if rsp < bounds.bottom {
                return Err(CpuFault::StackOverflow {
                    rsp,
                    bottom: bounds.bottom,
                    top: bounds.top,
                    rip,
                });
            }
            if rsp.checked_add(8).is_none_or(|end| end > bounds.top) {
                return Err(CpuFault::StackUnderflow {
                    rsp,
                    bottom: bounds.bottom,
                    top: bounds.top,
                    rip,
                });
            }
        }
        let value = mem.read_u64(rsp).ok_or(CpuFault::Memory {
            addr: rsp,
            write: false,
            rip,
        })?;
        self.regs[reg::RSP] = rsp.wrapping_add(8);
        Ok(value)
    }

    /// Evaluate a condition-code against the modelled flags.
    ///
    /// `None` when the condition depends on a flag xHoly does not model
    /// (`PF`) — the caller turns that into an explicit fault.
    fn condition(&self, code: u8) -> Option<bool> {
        let cf = self.rflags & FLAG_CF != 0;
        let zf = self.rflags & FLAG_ZF != 0;
        let sf = self.rflags & FLAG_SF != 0;
        let of = self.rflags & FLAG_OF != 0;
        Some(match code {
            0 => of,
            1 => !of,
            2 => cf,
            3 => !cf,
            4 => zf,
            5 => !zf,
            6 => cf || zf,
            7 => !cf && !zf,
            8 => sf,
            9 => !sf,
            10 | 11 => return None, // JP / JNP need PF
            12 => sf != of,
            13 => sf == of,
            14 => zf || sf != of,
            15 => !zf && sf == of,
            _ => return None,
        })
    }

    /// Apply an ALU operation and set `CF/ZF/SF/OF`.
    ///
    /// Returns `None` for sub-opcodes xHoly does not implement (2 = ADC,
    /// 3 = SBB). The result is masked to `width`.
    fn alu(&mut self, index: u8, a: u64, b: u64, width: u8) -> Option<u64> {
        let mask: u64 = if width == 8 {
            u64::MAX
        } else {
            u32::MAX as u64
        };
        let bits: u32 = if width == 8 { 64 } else { 32 };
        let a = a & mask;
        let b = b & mask;
        let sign_bit = 1u64 << (bits - 1);
        let (result, cf, of) = match index {
            0 => {
                let result = a.wrapping_add(b) & mask;
                let cf = u128::from(a) + u128::from(b) > u128::from(mask);
                let of = (!(a ^ b) & (a ^ result)) & sign_bit != 0;
                (result, cf, of)
            }
            1 => (a | b, false, false),
            4 => (a & b, false, false),
            5 | 7 => {
                let result = a.wrapping_sub(b) & mask;
                let cf = a < b;
                let of = ((a ^ b) & (a ^ result)) & sign_bit != 0;
                (result, cf, of)
            }
            6 => (a ^ b, false, false),
            _ => return None,
        };
        self.rflags &= !(FLAG_CF | FLAG_ZF | FLAG_SF | FLAG_OF);
        if cf {
            self.rflags |= FLAG_CF;
        }
        if of {
            self.rflags |= FLAG_OF;
        }
        if result == 0 {
            self.rflags |= FLAG_ZF;
        }
        if result & sign_bit != 0 {
            self.rflags |= FLAG_SF;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small flat guest buffer: code at the base, a stack at the top.
    ///
    /// Keeping CPU tests off the 1 GiB resident window makes them fast
    /// and independent of the memory subsystem's sizing policy.
    struct Flat {
        base: u64,
        data: Vec<u8>,
    }

    impl Flat {
        fn new(code: &[u8]) -> Self {
            let mut data = vec![0u8; 0x2000];
            data[..code.len()].copy_from_slice(code);
            Self {
                base: 0x1_0000,
                data,
            }
        }

        fn stack_top(&self) -> u64 {
            self.base + self.data.len() as u64 - 8
        }
    }

    impl GuestMemory for Flat {
        fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            let off = addr.checked_sub(self.base)? as usize;
            self.data
                .get(off..off.checked_add(len)?)
                .map(<[u8]>::to_vec)
        }

        fn write_bytes(&mut self, addr: u64, data: &[u8]) -> bool {
            let Some(off) = addr.checked_sub(self.base).map(|o| o as usize) else {
                return false;
            };
            match self.data.get_mut(off..off + data.len()) {
                Some(dst) => {
                    dst.copy_from_slice(data);
                    true
                }
                None => false,
            }
        }
    }

    /// CPU with `rip` at the code base and the stack at the top of the
    /// flat buffer.
    fn machine(code: &[u8]) -> (X64Cpu, Flat) {
        let mem = Flat::new(code);
        let mut cpu = X64Cpu::new();
        cpu.rip = mem.base;
        cpu.regs[reg::RSP] = mem.stack_top();
        (cpu, mem)
    }

    /// Step until an outcome other than `Executed`, at most `max` times.
    fn run(cpu: &mut X64Cpu, mem: &mut Flat, max: usize) -> Vec<StepOutcome> {
        let mut out = Vec::new();
        for _ in 0..max {
            let outcome = cpu.step(mem);
            let done = outcome != StepOutcome::Executed;
            out.push(outcome);
            if done {
                break;
            }
        }
        out
    }

    #[test]
    fn executes_mov_and_ret() {
        // mov eax, 42 ; ret
        let (mut cpu, mut mem) = machine(&[0xB8, 42, 0, 0, 0, 0xC3]);
        assert_eq!(cpu.step(&mut mem), StepOutcome::Executed);
        assert_eq!(cpu.step(&mut mem), StepOutcome::Executed);
        assert_eq!(cpu.regs[reg::RAX], 42);
        assert_eq!(cpu.instructions, 2);
        // RET popped the (zeroed) stack top as the next rip.
        assert_eq!(cpu.rip, 0);
    }

    #[test]
    fn mov_imm64_and_register_moves() {
        // mov rax, 0x1122334455667788 ; mov rcx, rax
        let code = [
            0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // mov rax, imm64
            0x48, 0x89, 0xC1, // mov rcx, rax
        ];
        let (mut cpu, mut mem) = machine(&code);
        run(&mut cpu, &mut mem, 8);
        assert_eq!(cpu.regs[reg::RAX], 0x1122_3344_5566_7788);
        assert_eq!(cpu.regs[reg::RCX], 0x1122_3344_5566_7788);
    }

    #[test]
    fn thirty_two_bit_writes_zero_extend() {
        // mov eax, 0xFFFFFFFF — the upper half must clear (x86-64 rule).
        let (mut cpu, mut mem) = machine(&[0xB8, 0xFF, 0xFF, 0xFF, 0xFF]);
        run(&mut cpu, &mut mem, 4);
        assert_eq!(cpu.regs[reg::RAX], 0xFFFF_FFFF);
        assert_ne!(cpu.regs[reg::RAX], u64::MAX);
    }

    #[test]
    fn cmp_sets_carry_sign_and_zero_flags() {
        // mov eax, 5 ; cmp eax, 7 ; cmp eax, 5
        let code = [
            0xB8, 0x05, 0x00, 0x00, 0x00, // mov eax, 5
            0x83, 0xF8, 0x07, // cmp eax, 7
        ];
        let (mut cpu, mut mem) = machine(&code);
        run(&mut cpu, &mut mem, 4);
        assert_ne!(cpu.rflags & FLAG_CF, 0, "5 < 7 sets CF");
        assert_ne!(cpu.rflags & FLAG_SF, 0, "5 - 7 is negative");
        assert_eq!(cpu.rflags & FLAG_ZF, 0, "not equal");
        assert_eq!(cpu.rflags & FLAG_OF, 0, "no signed overflow");
        assert_eq!(cpu.regs[reg::RAX], 5, "CMP never writes back");

        let mut code = code.to_vec();
        code.extend_from_slice(&[0x83, 0xF8, 0x05]); // cmp eax, 5
        let (mut cpu, mut mem) = machine(&code);
        run(&mut cpu, &mut mem, 4);
        assert_ne!(cpu.rflags & FLAG_ZF, 0, "equal sets ZF");
        assert_eq!(cpu.rflags & FLAG_CF, 0);
    }

    #[test]
    fn sub_detects_signed_overflow() {
        // mov eax, 0x80000000 ; sub eax, 1 → 0x7FFFFFFF with OF set.
        let code = [
            0xB8, 0x00, 0x00, 0x00, 0x80, // mov eax, 0x80000000
            0x83, 0xE8, 0x01, // sub eax, 1
        ];
        let (mut cpu, mut mem) = machine(&code);
        run(&mut cpu, &mut mem, 4);
        assert_eq!(cpu.regs[reg::RAX], 0x7FFF_FFFF);
        assert_ne!(cpu.rflags & FLAG_OF, 0, "signed overflow");
        assert_eq!(cpu.rflags & FLAG_CF, 0, "no borrow");
    }

    #[test]
    fn push_pop_roundtrip_restores_the_stack_pointer() {
        let mut cpu = X64Cpu::new();
        let mut mem = Flat::new(&[]);
        cpu.regs[reg::RSP] = mem.stack_top();
        cpu.regs[reg::RAX] = 0x1122_3344_5566_7788;
        let rsp = cpu.regs[reg::RSP];
        cpu.push(&mut mem, cpu.regs[reg::RAX], 0).expect("push");
        assert_eq!(cpu.regs[reg::RSP], rsp - 8);
        let value = cpu.pop(&mem, 0).expect("pop");
        assert_eq!(value, 0x1122_3344_5566_7788);
        assert_eq!(cpu.regs[reg::RSP], rsp);
    }

    #[test]
    fn call_rel32_pushes_the_return_address() {
        // 0x00 call +6 → 0x0B ; 0x05 mov eax,99 ; 0x0A hlt ;
        // 0x0B mov eax,7 (callee) ; 0x10 ret
        let code = [
            0xE8, 0x06, 0x00, 0x00, 0x00, // call rel32 → base+0x0B
            0xB8, 0x63, 0x00, 0x00, 0x00, // mov eax, 99 (return site)
            0xF4, //                         hlt
            0xB8, 0x07, 0x00, 0x00, 0x00, // mov eax, 7  (callee)
            0xC3, //                         ret
        ];
        let (mut cpu, mut mem) = machine(&code);
        let base = mem.base;
        let outcomes = run(&mut cpu, &mut mem, 8);
        // Callee ran, RET returned to base+5, the return site ran, then HLT.
        assert_eq!(outcomes.last(), Some(&StepOutcome::Halted));
        assert_eq!(cpu.regs[reg::RAX], 99, "return site must run after RET");
        assert!(cpu.halted);
        // RET pushed base+5 and restored the stack pointer on the way back.
        assert_eq!(cpu.regs[reg::RSP], mem.stack_top());
        assert!(cpu.instructions >= 5);
        assert_eq!(base, mem.base);
    }

    #[test]
    fn conditional_jumps_respect_the_flags() {
        // mov eax,1 ; cmp eax,1 ; je +5 ; mov eax,99 ; hlt
        let taken = [
            0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
            0x83, 0xF8, 0x01, // cmp eax, 1
            0x74, 0x05, // je +5 (skips the mov)
            0xB8, 0x63, 0x00, 0x00, 0x00, // mov eax, 99
            0xF4, // hlt
        ];
        let (mut cpu, mut mem) = machine(&taken);
        let outcomes = run(&mut cpu, &mut mem, 16);
        assert_eq!(outcomes.last(), Some(&StepOutcome::Halted));
        assert_eq!(cpu.regs[reg::RAX], 1, "je must skip the mov eax,99");

        // Not taken: the fall-through instruction executes.
        let not_taken = [
            0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
            0x83, 0xF8, 0x02, // cmp eax, 2
            0x74, 0x05, // je +5
            0xB8, 0x63, 0x00, 0x00, 0x00, // mov eax, 99
            0xF4, // hlt
        ];
        let (mut cpu, mut mem) = machine(&not_taken);
        run(&mut cpu, &mut mem, 16);
        assert_eq!(cpu.regs[reg::RAX], 99, "fall-through executes");
    }

    #[test]
    fn relative_memory_addressing_works() {
        // mov qword [rax+8], rcx   (48 89 48 08)
        // mov rdx, qword [rax+8]   (48 8B 50 08)
        let code = [
            0x48, 0x89, 0x48, 0x08, // mov [rax+8], rcx
            0x48, 0x8B, 0x50, 0x08, // mov rdx, [rax+8]
        ];
        let (mut cpu, mut mem) = machine(&code);
        cpu.regs[reg::RAX] = mem.base + 0x100;
        cpu.regs[reg::RCX] = 0xDEAD_BEEF;
        run(&mut cpu, &mut mem, 8);
        assert_eq!(cpu.regs[reg::RDX], 0xDEAD_BEEF);
        assert_eq!(mem.read_u64(mem.base + 0x108), Some(0xDEAD_BEEF));
    }

    #[test]
    fn mov_memory_immediate_writes_through_a_guest_pointer() {
        // mov qword ptr [rax], 0x11   (48 C7 00 11 00 00 00)
        let code = [0x48, 0xC7, 0x00, 0x11, 0x00, 0x00, 0x00];
        let (mut cpu, mut mem) = machine(&code);
        cpu.regs[reg::RAX] = mem.base + 0x200;
        assert_eq!(cpu.step(&mut mem), StepOutcome::Executed);
        assert_eq!(mem.read_u64(mem.base + 0x200), Some(0x11));
        assert_eq!(cpu.regs[reg::RAX], mem.base + 0x200, "pointer untouched");
        // A negative immediate sign-extends into the full 64-bit word.
        let code = [0x48, 0xC7, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let (mut cpu, mut mem) = machine(&code);
        cpu.regs[reg::RAX] = mem.base + 0x200;
        cpu.step(&mut mem);
        assert_eq!(mem.read_u64(mem.base + 0x200), Some(0xFFFF_FFFF_FFFF_FFFF));
        // Only /0 is a MOV; other sub-opcodes fault explicitly.
        let (mut cpu, mut mem) = machine(&[0xC7, 0x08, 0x01, 0x00, 0x00, 0x00]);
        assert!(matches!(
            cpu.step(&mut mem),
            StepOutcome::Fault(CpuFault::IllegalInstruction { .. })
        ));
    }

    #[test]
    fn finish_hle_call_pops_the_transition_return_address() {
        // call qword ptr [rip+disp] through a registered slot, then let
        // the pipeline complete the callee half of the call.
        let slot_offset = 0x100usize;
        let mut code = vec![0xFF, 0x15, 0x00, 0x00, 0x00, 0x00];
        code.resize(slot_offset, 0x90);
        let slot = 0x1_0000u64 + slot_offset as u64;
        let disp = (slot as i64) - ((0x1_0000u64 + 6) as i64);
        code[2..6].copy_from_slice(&(disp as i32).to_le_bytes());
        let mut mem = Flat::new(&code);
        let mut cpu = X64Cpu::new();
        cpu.rip = mem.base;
        cpu.regs[reg::RSP] = mem.stack_top();
        cpu.set_hle_slots([slot]);

        assert_eq!(cpu.step(&mut mem), StepOutcome::HleCall { slot_addr: slot });
        let return_address = cpu.rip;
        assert_eq!(cpu.regs[reg::RSP], mem.stack_top() - 8);
        // The callee returns: the pushed address is popped again, so a
        // later RET sees the entry's return address, not the HLE frame.
        let popped = cpu.finish_hle_call(&mem).expect("stack is mapped");
        assert_eq!(popped, return_address);
        assert_eq!(cpu.regs[reg::RSP], mem.stack_top());
        assert_eq!(cpu.rip, return_address, "rip stays at the return site");
    }

    #[test]
    fn rip_relative_reads_resolve_against_the_next_instruction() {
        // mov rax, qword [rip+8] → 7-byte instruction, target = base+7+8 = base+15.
        let code = [0x48, 0x8B, 0x05, 0x08, 0x00, 0x00, 0x00];
        let (mut cpu, mut mem) = machine(&code);
        let payload = 0x0123_4567_89AB_CDEFu64.to_le_bytes();
        mem.data[15..15 + 8].copy_from_slice(&payload);
        assert_eq!(cpu.step(&mut mem), StepOutcome::Executed);
        assert_eq!(cpu.regs[reg::RAX], 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn hlt_stops_execution() {
        let (mut cpu, mut mem) = machine(&[0xF4]);
        assert_eq!(cpu.step(&mut mem), StepOutcome::Halted);
        assert!(cpu.halted);
        assert_eq!(cpu.step(&mut mem), StepOutcome::Halted);
    }

    #[test]
    fn unsupported_encodings_fault_explicitly() {
        // LEA (0x8D) is not in the subset.
        let (mut cpu, mut mem) = machine(&[0x48, 0x8D, 0x00]);
        match cpu.step(&mut mem) {
            StepOutcome::Fault(CpuFault::IllegalInstruction { opcode, .. }) => {
                assert_eq!(opcode, 0x8D);
            }
            other => panic!("expected illegal instruction, got {other:?}"),
        }
        // ADC (group 1 /2) is not implemented.
        let (mut cpu, mut mem) = machine(&[0x83, 0xD0, 0x01]);
        assert!(matches!(
            cpu.step(&mut mem),
            StepOutcome::Fault(CpuFault::IllegalInstruction { .. })
        ));
        // JP depends on PF, which is not modelled.
        let (mut cpu, mut mem) = machine(&[0x0F, 0x8A, 0x00, 0x00, 0x00, 0x00]);
        assert!(matches!(
            cpu.step(&mut mem),
            StepOutcome::Fault(CpuFault::IllegalInstruction { .. })
        ));
        // Faulting instructions do not retire or advance the pointer.
        assert_eq!(cpu.instructions, 0);
    }

    #[test]
    fn fetch_out_of_bounds_is_a_structured_fault() {
        let (mut cpu, mut mem) = machine(&[0x90]);
        cpu.rip = mem.base + 0x10_0000;
        assert_eq!(
            cpu.step(&mut mem),
            StepOutcome::Fault(CpuFault::FetchOutOfBounds {
                rip: mem.base + 0x10_0000
            })
        );
    }

    #[test]
    fn unmapped_data_access_is_a_memory_fault() {
        // mov [rax], rcx with rax = 0 (never mapped).
        let (mut cpu, mut mem) = machine(&[0x48, 0x89, 0x08]);
        cpu.regs[reg::RAX] = 0;
        cpu.regs[reg::RCX] = 1;
        assert_eq!(
            cpu.step(&mut mem),
            StepOutcome::Fault(CpuFault::Memory {
                addr: 0,
                write: true,
                rip: mem.base
            })
        );
    }

    #[test]
    fn registered_slot_calls_become_hle_transitions() {
        // call qword ptr [rip+disp] → slot at base+0x100.
        let slot_offset = 0x100usize;
        let mut code = vec![0xFF, 0x15, 0x00, 0x00, 0x00, 0x00];
        code.resize(slot_offset, 0x90);
        let slot = 0x1_0000u64 + slot_offset as u64;
        let disp = (slot as i64) - ((0x1_0000u64 + 6) as i64);
        code[2..6].copy_from_slice(&(disp as i32).to_le_bytes());
        let mut mem = Flat::new(&code);
        // Stale IAT content: reading it would jump somewhere absurd.
        mem.data[slot_offset..slot_offset + 8].copy_from_slice(&0xAAu64.to_le_bytes());
        let mut cpu = X64Cpu::new();
        cpu.rip = mem.base;
        cpu.regs[reg::RSP] = mem.stack_top();
        cpu.set_hle_slots([slot]);

        let outcome = cpu.step(&mut mem);
        assert_eq!(outcome, StepOutcome::HleCall { slot_addr: slot });
        // Return address pushed, rip parked after the call.
        assert_eq!(cpu.rip, mem.base + 6);
        assert_eq!(cpu.regs[reg::RSP], mem.stack_top() - 8);
        assert_eq!(mem.read_u64(cpu.regs[reg::RSP]), Some(mem.base + 6));
        // The stale slot contents were never used as a jump target.
        assert_ne!(cpu.rip, 0xAAAA_AAAA_AAAA_AAAA);
    }

    #[test]
    fn unregistered_indirect_calls_jump_to_the_slot_contents() {
        let slot_offset = 0x100usize;
        let mut code = vec![0xFF, 0x15, 0x00, 0x00, 0x00, 0x00];
        code.resize(slot_offset, 0x90);
        let mut mem = Flat::new(&code);
        let slot = mem.base + slot_offset as u64;
        let disp = (slot as i64) - ((mem.base + 6) as i64);
        mem.data[2..6].copy_from_slice(&(disp as i32).to_le_bytes());
        // Slot holds the address of a guest function at base+0x180.
        let callee = mem.base + 0x180;
        mem.data[slot_offset..slot_offset + 8].copy_from_slice(&callee.to_le_bytes());
        // callee: hlt
        mem.data[0x180] = 0xF4;

        let mut cpu = X64Cpu::new();
        cpu.rip = mem.base;
        cpu.regs[reg::RSP] = mem.stack_top();
        // No HLE slots registered → ordinary indirect call.
        assert_eq!(cpu.step(&mut mem), StepOutcome::Executed);
        assert_eq!(cpu.rip, callee, "jumped to the slot contents");
        assert_eq!(cpu.step(&mut mem), StepOutcome::Halted);
    }

    #[test]
    fn argument_and_return_registers_follow_the_x64_convention() {
        let mut cpu = X64Cpu::new();
        cpu.regs[reg::RCX] = 1;
        cpu.regs[reg::RDX] = 2;
        cpu.regs[reg::R8] = 3;
        cpu.regs[reg::R9] = 4;
        cpu.regs[reg::RAX] = 0;
        assert_eq!(cpu.read_args(2), vec![1, 2]);
        assert_eq!(cpu.read_args(4), vec![1, 2, 3, 4]);
        // Never more than the four integer argument registers.
        assert_eq!(cpu.read_args(9).len(), 4);
        assert_eq!(cpu.read_args(0), Vec::<u64>::new());
        cpu.write_return(0x484C_4501);
        assert_eq!(cpu.return_value(), 0x484C_4501);
        assert_eq!(cpu.regs[reg::RAX], 0x484C_4501);
    }

    #[test]
    fn snapshot_renders_state_for_diagnostics() {
        let (mut cpu, mut mem) = machine(&[0xB8, 0x2A, 0, 0, 0]);
        run(&mut cpu, &mut mem, 4);
        let snap = cpu.snapshot();
        assert_eq!(snap.register(reg::RAX), 42);
        assert_eq!(snap.instructions, 1);
        let rendered = snap.render();
        assert!(rendered.contains("rip=0x"));
        assert!(rendered.contains("rax=0x000000000000002a"));
        assert!(rendered.contains("r15="));
        assert_eq!(register_name(reg::R15), "r15");
        assert_eq!(register_name(99), "?");
    }

    #[test]
    fn halted_cpu_does_not_step() {
        let (mut cpu, mut mem) = machine(&[0x90, 0x90]);
        cpu.halted = true;
        assert_eq!(cpu.step(&mut mem), StepOutcome::Halted);
        assert_eq!(cpu.instructions, 0);
    }
}
