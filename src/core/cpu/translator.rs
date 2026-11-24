//! STATUS: experimental stub, not used by the emulator loop. Decision on the x86-64 path in phase 24E (ADR 0001).
//! CPU translation pipeline (stub).
//!
//! This module defines the translation pipeline the production JIT will
//! implement: guest x86-64 decode → xHoly IR → host lowering. It is a
//! structural stub — the IR and pass boundaries are real, the instruction
//! coverage is intentionally minimal.

use crate::core::memory::{GuestAddr, Memory};

/// Guest ISA targeted by the translator.
///
/// Xbox One runs an AMD Jaguar (x86-64); Xbox Series X|S runs Zen 2
/// (x86-64). Both share the ISA family, so one decoder covers the
/// generation — aarch64 hosts lower through the same IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestIsa {
    /// x86-64 long mode (Xbox One / Series X|S).
    X86_64,
}

/// Intermediate representation ops produced by the decoder.
///
/// Deliberately small: enough to express integer arithmetic, branches,
/// loads/stores, and system ops. Extended incrementally as coverage grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrOp {
    /// rd = rs1 + imm
    AddImm { rd: u8, rs1: u8, imm: i64 },
    /// rd = rs1 op rs2 (ALU)
    AluReg { op: AluOp, rd: u8, rs1: u8, rs2: u8 },
    /// Load `size` bytes from [rs1 + imm] into rd.
    Load { rd: u8, rs1: u8, imm: i64, size: u8 },
    /// Store rs2 to [rs1 + imm].
    Store {
        rs1: u8,
        rs2: u8,
        imm: i64,
        size: u8,
    },
    /// Conditional branch.
    Branch { cond: Cond, target: GuestAddr },
    /// Unconditional jump / call.
    Jump { target: GuestAddr },
    /// Privileged/system operation (MSR, CPUID, HLT...).
    System { id: u16 },
    /// End of translation block.
    BlockEnd,
}

/// Integer ALU operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
}

/// Branch conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Eq,
    Ne,
    Lt,
    Ge,
    LtU,
    GeU,
}

/// A decoded basic block ready for lowering.
#[derive(Debug, Default)]
pub struct TranslationBlock {
    pub ops: Vec<IrOp>,
    /// Guest address of the first instruction.
    pub start: GuestAddr,
    /// Guest address of the next block (fallthrough or branch target).
    pub next: GuestAddr,
}

/// Statistics collected during translation (for benchmarks/profiling).
#[derive(Debug, Default, Clone, Copy)]
pub struct TranslationStats {
    pub blocks_translated: u64,
    pub instructions_covered: u64,
    pub unsupported_instructions: u64,
}

/// The CPU translator: decode guest code → IR → (future) host code.
pub struct Translator {
    isa: GuestIsa,
    stats: TranslationStats,
}

impl Translator {
    pub fn new(isa: GuestIsa) -> Self {
        Self {
            isa,
            stats: TranslationStats::default(),
        }
    }

    pub fn isa(&self) -> GuestIsa {
        self.isa
    }

    pub fn stats(&self) -> TranslationStats {
        self.stats
    }

    /// Translate the basic block starting at `pc`.
    ///
    /// Reads guest bytes from `mem`, decodes instructions into [`IrOp`]s,
    /// and stops at the first control-flow instruction or block boundary.
    /// Returns `None` if no decodable instruction exists at `pc`.
    pub fn translate_block(&mut self, mem: &Memory, pc: GuestAddr) -> Option<TranslationBlock> {
        let mut block = TranslationBlock {
            start: pc,
            next: pc,
            ops: Vec::new(),
        };
        let mut cursor = pc;
        let mut steps = 0u32;

        while steps < 64 {
            // Bound the block at 64 instructions (stub limit; real JIT
            // uses max basic-block size heuristics).
            let insn = mem.read(cursor, 8)?;
            if insn == 0 {
                // Zero page — treat as HLT-like terminator.
                block.ops.push(IrOp::System { id: 0 });
                block.next = cursor;
                break;
            }

            // Minimal x86-64 decode: recognize a handful of patterns so the
            // pipeline is exercisable. Everything else is counted as
            // unsupported rather than silently mistranslated.
            match decode_x86_64_stub(insn) {
                Some(op) => {
                    block.ops.push(op);
                    self.stats.instructions_covered += 1;
                    steps += 1;
                    cursor = cursor.wrapping_add(insn_len_stub(insn));
                }
                None => {
                    self.stats.unsupported_instructions += 1;
                    block.ops.push(IrOp::System { id: u16::MAX });
                    block.next = cursor;
                    break;
                }
            }

            if matches!(
                block.ops.last(),
                Some(
                    IrOp::Jump { .. } | IrOp::Branch { .. } | IrOp::System { .. } | IrOp::BlockEnd
                )
            ) {
                block.next = cursor;
                break;
            }
        }

        if block.ops.is_empty() {
            return None;
        }
        block.ops.push(IrOp::BlockEnd);
        self.stats.blocks_translated += 1;
        Some(block)
    }
}

/// Very small x86-64 decoder stub.
///
/// Recognizes `mov imm→reg`-style and a few ALU encodings well enough to
/// smoke-test the IR pipeline. Returns `None` for anything else.
fn decode_x86_64_stub(insn: u64) -> Option<IrOp> {
    // REX prefix check (0x48-0x4F) for 64-bit operand ops.
    let b0 = (insn & 0xff) as u8;
    if !(0x48..=0x4f).contains(&b0) {
        return None;
    }
    // MOV r64, imm64 is REX.W + B8+r (we accept the common B8-BF range
    // only when the following bytes look like a plausible immediate).
    let op = ((insn >> 8) & 0xff) as u8;
    if (0xb8..=0xbf).contains(&op) {
        let rd = op - 0xb8;
        return Some(IrOp::AddImm {
            rd,
            rs1: 0,
            imm: (insn >> 16) as i64,
        });
    }
    // 83 /0 ib = ADD r/m64, imm8 ; C0 /0 = same with imm32 (approximation).
    if op == 0x83 || op == 0xc0 {
        return Some(IrOp::AluReg {
            op: AluOp::Add,
            rd: 0,
            rs1: 0,
            rs2: 0,
        });
    }
    None
}

/// Approximate instruction length for the stub decoder.
fn insn_len_stub(insn: u64) -> u64 {
    match (insn & 0xff) as u8 {
        0x48..=0x4f => 10, // REX + opcode + imm64 (MOV r64, imm64)
        _ => 1,
    }
}
