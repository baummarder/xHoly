//! x86-64 subset disassembler/decoder — Phase 13.
//!
//! Decodes the integer register subset the JIT can translate:
//! REX.W-prefixed `mov/add/sub/xor/and/or/cmp/test`, reg-reg and
//! reg-imm forms, `movzx/movsx`, `push/pop`, short/near `jmp`/`jcc`,
//! `call`, `ret`, `nop`, `syscall`, `inc/dec/neg/not`.
//!
//! Deliberately rejected (returns [`JitError::Unsupported`]):
//! memory operands (ModR/M mod != 3), SIMD/FP opcodes, segment
//! overrides, string ops, privileged instructions. Honest errors —
//! never a silent mistranslation.

use super::{JitError, JitResult};

/// x86-64 register index (0=rax … 15=r15).
pub type Reg = u8;

/// Decoded operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// Register (0–15).
    Reg(Reg),
    /// Sign-extended immediate.
    Imm(i64),
}

/// Condition codes for `jcc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    O,
    No,
    B,
    Ae,
    E,
    Ne,
    Be,
    A,
    S,
    Ns,
    P,
    Np,
    L,
    Ge,
    Le,
    G,
}

impl Cond {
    /// Opcode low-nibble for 0x70–0x7F.
    fn from_low(n: u8) -> Self {
        match n {
            0 => Self::O,
            1 => Self::No,
            2 => Self::B,
            3 => Self::Ae,
            4 => Self::E,
            5 => Self::Ne,
            6 => Self::Be,
            7 => Self::A,
            8 => Self::S,
            9 => Self::Ns,
            10 => Self::P,
            11 => Self::Np,
            12 => Self::L,
            13 => Self::Ge,
            14 => Self::Le,
            _ => Self::G,
        }
    }

    /// AT&T-ish mnemonic.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::O => "jo",
            Self::No => "jno",
            Self::B => "jb",
            Self::Ae => "jae",
            Self::E => "je",
            Self::Ne => "jne",
            Self::Be => "jbe",
            Self::A => "ja",
            Self::S => "js",
            Self::Ns => "jns",
            Self::P => "jp",
            Self::Np => "jnp",
            Self::L => "jl",
            Self::Ge => "jge",
            Self::Le => "jle",
            Self::G => "jg",
        }
    }
}

/// Instruction kinds covered by the JIT subset.
#[derive(Debug, Clone, PartialEq)]
pub enum InstrKind {
    /// `mov dst, src`
    Mov {
        dst: Operand,
        src: Operand,
    },
    /// `movzx dst, srcN` / `movsx` — zero/sign extend; `bits` = 8 or 16.
    Movzx {
        dst: Operand,
        src: Operand,
        bits: u8,
    },
    Movsx {
        dst: Operand,
        src: Operand,
        bits: u8,
    },
    /// `add/sub/xor/and/or dst, src`
    Add {
        dst: Operand,
        src: Operand,
    },
    Sub {
        dst: Operand,
        src: Operand,
    },
    Xor {
        dst: Operand,
        src: Operand,
    },
    And {
        dst: Operand,
        src: Operand,
    },
    Or {
        dst: Operand,
        src: Operand,
    },
    /// `mul dst, src` (imul r64, r/m64 — low 64 bits kept).
    Mul {
        dst: Operand,
        src: Operand,
    },
    /// `cmp a, b` (computes a - b, sets flags, no writeback)
    Cmp {
        a: Operand,
        b: Operand,
    },
    /// `test a, b` (a & b, sets flags)
    Test {
        a: Operand,
        b: Operand,
    },
    /// `inc/dec/neg/not reg`
    Inc(Operand),
    Dec(Operand),
    Neg(Operand),
    Not(Operand),
    /// `push reg` / `pop reg`
    Push(Operand),
    Pop(Operand),
    /// `jmp target` (absolute byte offset in `code`)
    Jmp {
        target: usize,
    },
    /// `j<cc> target`
    Jcc {
        cond: Cond,
        target: usize,
    },
    /// `call target`
    Call {
        target: usize,
    },
    /// `ret`
    Ret,
    /// `syscall` — host call (svc-equivalent for x86-64 guests).
    Syscall,
    /// `nop`
    Nop,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct X86Instr {
    /// Byte offset of the instruction start.
    pub offset: usize,
    /// Instruction length in bytes.
    pub len: usize,
    /// Decoded kind.
    pub kind: InstrKind,
}

/// Canonical register names (index → name).
pub fn reg_name(r: Reg) -> &'static str {
    const NAMES: [&str; 16] = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    NAMES[(r & 15) as usize]
}

/// Format an operand AT&T-style.
pub fn fmt_operand(o: &Operand) -> String {
    match o {
        Operand::Reg(r) => format!("%{}", reg_name(*r)),
        Operand::Imm(v) => format!("${v}"),
    }
}

impl X86Instr {
    /// AT&T-syntax disassembly.
    pub fn to_asm(&self) -> String {
        match &self.kind {
            InstrKind::Mov { dst, src } => {
                format!("mov {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Movzx { dst, src, bits } => {
                format!("movzx {}, {}", fmt_operand(dst), fmt_operand(src))
                    + &format!(" ({bits}-bit)")
            }
            InstrKind::Movsx { dst, src, bits } => {
                format!("movsx {}, {}", fmt_operand(dst), fmt_operand(src))
                    + &format!(" ({bits}-bit)")
            }
            InstrKind::Add { dst, src } => {
                format!("add {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Sub { dst, src } => {
                format!("sub {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Xor { dst, src } => {
                format!("xor {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::And { dst, src } => {
                format!("and {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Or { dst, src } => {
                format!("or {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Mul { dst, src } => {
                format!("imul {}, {}", fmt_operand(dst), fmt_operand(src))
            }
            InstrKind::Cmp { a, b } => {
                format!("cmp {}, {}", fmt_operand(a), fmt_operand(b))
            }
            InstrKind::Test { a, b } => {
                format!("test {}, {}", fmt_operand(a), fmt_operand(b))
            }
            InstrKind::Inc(r) => format!("inc {}", fmt_operand(r)),
            InstrKind::Dec(r) => format!("dec {}", fmt_operand(r)),
            InstrKind::Neg(r) => format!("neg {}", fmt_operand(r)),
            InstrKind::Not(r) => format!("not {}", fmt_operand(r)),
            InstrKind::Push(r) => format!("push {}", fmt_operand(r)),
            InstrKind::Pop(r) => format!("pop {}", fmt_operand(r)),
            InstrKind::Jmp { target } => format!("jmp 0x{target:x}"),
            InstrKind::Jcc { cond, target } => format!("{} 0x{target:x}", cond.mnemonic()),
            InstrKind::Call { target } => format!("call 0x{target:x}"),
            InstrKind::Ret => "ret".to_string(),
            InstrKind::Syscall => "syscall".to_string(),
            InstrKind::Nop => "nop".to_string(),
        }
    }
}

/// Decode all instructions in `code`. Stops at the first byte that
/// doesn't decode (caller decides whether trailing data is padding).
pub fn decode_all(code: &[u8]) -> JitResult<Vec<X86Instr>> {
    let mut out = Vec::new();
    let mut pc = 0usize;
    while pc < code.len() {
        let (instr, len) = decode_one(code, pc)?;
        if len == 0 {
            break; // trailing padding
        }
        out.push(instr);
        pc += len;
    }
    Ok(out)
}

/// Disassemble to AT&T text (one instruction per line).
pub fn disassemble(code: &[u8]) -> JitResult<String> {
    let instrs = decode_all(code)?;
    Ok(instrs
        .iter()
        .map(|i| format!("{:04x}: {}", i.offset, i.to_asm()))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Internal decode state (REX prefix flags).
struct Rex {
    w: bool,
    r: bool,
    x: bool,
    b: bool,
}

fn read_imm(code: &[u8], pc: usize, len: usize) -> JitResult<i64> {
    if pc + len > code.len() {
        return Err(JitError::ParseError("truncated immediate".into()));
    }
    Ok(match len {
        1 => code[pc] as i8 as i64,
        2 => i16::from_le_bytes([code[pc], code[pc + 1]]) as i64,
        4 => i32::from_le_bytes(code[pc..pc + 4].try_into().unwrap()) as i64,
        8 => i64::from_le_bytes(code[pc..pc + 8].try_into().unwrap()),
        _ => return Err(JitError::ParseError("bad imm len".into())),
    })
}

/// Decode the ModR/M byte; only mod==3 (register direct) supported.
fn decode_modrm(code: &[u8], pc: usize, rex: &Rex) -> JitResult<(Operand, Operand, usize)> {
    if pc >= code.len() {
        return Err(JitError::ParseError("truncated modrm".into()));
    }
    let m = code[pc];
    let modrm = pc + 1;
    if (m >> 6) != 3 {
        return Err(JitError::Unsupported(format!(
            "memory operand (modrm {m:#04x}) not supported"
        )));
    }
    let reg = ((m >> 3) & 7) | if rex.r { 8 } else { 0 };
    let rm = (m & 7) | if rex.b { 8 } else { 0 };
    let _ = rex.x;
    let _ = modrm;
    Ok((
        Operand::Reg(rm),
        Operand::Reg(reg),
        1, // consumed: modrm byte only
    ))
}

fn decode_one(code: &[u8], pc: usize) -> JitResult<(X86Instr, usize)> {
    let start = pc;
    let mut p = pc;
    let mut rex = Rex {
        w: false,
        r: false,
        x: false,
        b: false,
    };
    let mut has_rex = false;
    // REX prefix: 0x40–0x4F (only meaningful with W/R/X/B bits).
    if p < code.len() && (0x40..=0x4F).contains(&code[p]) {
        let r = code[p];
        rex = Rex {
            w: r & 8 != 0,
            r: r & 4 != 0,
            x: r & 2 != 0,
            b: r & 1 != 0,
        };
        has_rex = true;
        p += 1;
    }
    if p >= code.len() {
        return Err(JitError::ParseError("truncated at opcode".into()));
    }
    let op = code[p];
    p += 1;
    let _consumed_prefix = p - start;

    let mk = |kind: InstrKind, total_len: usize| -> JitResult<(X86Instr, usize)> {
        Ok((
            X86Instr {
                offset: start,
                len: total_len,
                kind,
            },
            total_len,
        ))
    };

    // Two-byte opcodes.
    if op == 0x0F {
        if p >= code.len() {
            return Err(JitError::ParseError("truncated 0F opcode".into()));
        }
        let op2 = code[p];
        p += 1;
        return match op2 {
            0x05 => mk(InstrKind::Syscall, p - start),
            0xB6 => {
                let (rm, reg, used) = decode_modrm(code, p, &rex)?;
                p += used;
                mk(
                    InstrKind::Movzx {
                        dst: reg,
                        src: rm,
                        bits: 8,
                    },
                    p - start,
                )
            }
            0xB7 => {
                let (rm, reg, used) = decode_modrm(code, p, &rex)?;
                p += used;
                mk(
                    InstrKind::Movzx {
                        dst: reg,
                        src: rm,
                        bits: 16,
                    },
                    p - start,
                )
            }
            0xBE => {
                let (rm, reg, used) = decode_modrm(code, p, &rex)?;
                p += used;
                mk(
                    InstrKind::Movsx {
                        dst: reg,
                        src: rm,
                        bits: 8,
                    },
                    p - start,
                )
            }
            0xBF => {
                let (rm, reg, used) = decode_modrm(code, p, &rex)?;
                p += used;
                mk(
                    InstrKind::Movsx {
                        dst: reg,
                        src: rm,
                        bits: 16,
                    },
                    p - start,
                )
            }
            0xAF => {
                let (rm, reg, used) = decode_modrm(code, p, &rex)?;
                p += used;
                mk(InstrKind::Mul { dst: rm, src: reg }, p - start)
            }
            _ => Err(JitError::Unsupported(format!("0F {op2:#04x}"))),
        };
    }

    match op {
        // mov r64, imm64 (REX.W + B8+rd) or imm32 (no REX.W).
        0xB8..=0xBF => {
            let reg = (op - 0xB8) | if rex.b { 8 } else { 0 };
            let imm_len = if rex.w { 8 } else { 4 };
            let imm = read_imm(code, p, imm_len)?;
            p += imm_len;
            mk(
                InstrKind::Mov {
                    dst: Operand::Reg(reg),
                    src: Operand::Imm(imm),
                },
                p - start,
            )
        }
        // mov r/m64, r64 (89) / mov r64, r/m64 (8B).
        0x89 | 0x8B => {
            let (rm, reg, used) = decode_modrm(code, p, &rex)?;
            p += used;
            let kind = if op == 0x89 {
                InstrKind::Mov { dst: rm, src: reg }
            } else {
                InstrKind::Mov { dst: reg, src: rm }
            };
            mk(kind, p - start)
        }
        // ALU r/m64, r64 (reg field is source).
        0x01 | 0x29 | 0x31 | 0x21 | 0x09 | 0x39 => {
            let (rm, reg, used) = decode_modrm(code, p, &rex)?;
            p += used;
            let kind = match op {
                0x01 => InstrKind::Add { dst: rm, src: reg },
                0x29 => InstrKind::Sub { dst: rm, src: reg },
                0x31 => InstrKind::Xor { dst: rm, src: reg },
                0x21 => InstrKind::And { dst: rm, src: reg },
                0x09 => InstrKind::Or { dst: rm, src: reg },
                _ => InstrKind::Cmp { a: rm, b: reg },
            };
            mk(kind, p - start)
        }
        // ALU r64, r/m64 (reg field is dest).
        0x03 | 0x2B | 0x33 | 0x23 | 0x0B | 0x3B | 0x85 => {
            let (rm, reg, used) = decode_modrm(code, p, &rex)?;
            p += used;
            let kind = match op {
                0x03 => InstrKind::Add { dst: reg, src: rm },
                0x2B => InstrKind::Sub { dst: reg, src: rm },
                0x33 => InstrKind::Xor { dst: reg, src: rm },
                0x23 => InstrKind::And { dst: reg, src: rm },
                0x0B => InstrKind::Or { dst: reg, src: rm },
                0x3B => InstrKind::Cmp { a: reg, b: rm },
                _ => InstrKind::Test { a: reg, b: rm },
            };
            mk(kind, p - start)
        }
        // ALU r/m64, imm32 (sign-extended).
        0x81 => {
            let (rm, sub, used) = decode_modrm_sub(code, p, &rex)?;
            p += used;
            let imm = read_imm(code, p, 4)?;
            p += 4;
            let dst = Operand::Reg(rm);
            let kind = match sub {
                0 => InstrKind::Add {
                    dst,
                    src: Operand::Imm(imm),
                },
                5 => InstrKind::Sub {
                    dst,
                    src: Operand::Imm(imm),
                },
                4 => InstrKind::And {
                    dst,
                    src: Operand::Imm(imm),
                },
                1 => InstrKind::Or {
                    dst,
                    src: Operand::Imm(imm),
                },
                6 => InstrKind::Xor {
                    dst,
                    src: Operand::Imm(imm),
                },
                7 => InstrKind::Cmp {
                    a: dst,
                    b: Operand::Imm(imm),
                },
                _ => {
                    return Err(JitError::Unsupported(format!("grp1 sub-op {sub}")));
                }
            };
            mk(kind, p - start)
        }
        // mov r/m64, imm32 (C7 /0) — the common `mov reg, imm` form.
        0xC7 => {
            let (rm, sub, used) = decode_modrm_sub(code, p, &rex)?;
            p += used;
            if sub != 0 {
                return Err(JitError::Unsupported(format!("C7 sub-op {sub}")));
            }
            let imm = read_imm(code, p, 4)?;
            p += 4;
            mk(
                InstrKind::Mov {
                    dst: Operand::Reg(rm),
                    src: Operand::Imm(imm),
                },
                p - start,
            )
        }
        // ALU r/m64, imm8.
        0x83 => {
            let (rm, sub, used) = decode_modrm_sub(code, p, &rex)?;
            p += used;
            let imm = read_imm(code, p, 1)?;
            p += 1;
            let dst = Operand::Reg(rm);
            let kind = match sub {
                0 => InstrKind::Add {
                    dst,
                    src: Operand::Imm(imm),
                },
                5 => InstrKind::Sub {
                    dst,
                    src: Operand::Imm(imm),
                },
                7 => InstrKind::Cmp {
                    a: dst,
                    b: Operand::Imm(imm),
                },
                4 => InstrKind::And {
                    dst,
                    src: Operand::Imm(imm),
                },
                1 => InstrKind::Or {
                    dst,
                    src: Operand::Imm(imm),
                },
                6 => InstrKind::Xor {
                    dst,
                    src: Operand::Imm(imm),
                },
                _ => {
                    return Err(JitError::Unsupported(format!("grp1b sub-op {sub}")));
                }
            };
            mk(kind, p - start)
        }
        // inc/dec reg (FF /0 /1) and push/pop (50–5F).
        0xFF => {
            let (rm, sub, used) = decode_modrm_sub(code, p, &rex)?;
            p += used;
            match sub {
                0 => mk(InstrKind::Inc(Operand::Reg(rm)), p - start),
                1 => mk(InstrKind::Dec(Operand::Reg(rm)), p - start),
                _ => Err(JitError::Unsupported(format!("FF sub-op {sub}"))),
            }
        }
        0x50..=0x57 => {
            let reg = (op - 0x50) | if rex.b { 8 } else { 0 };
            mk(InstrKind::Push(Operand::Reg(reg)), p - start)
        }
        0x58..=0x5F => {
            let reg = (op - 0x58) | if rex.b { 8 } else { 0 };
            mk(InstrKind::Pop(Operand::Reg(reg)), p - start)
        }
        // jcc short.
        0x70..=0x7F => {
            let disp = read_imm(code, p, 1)?;
            p += 1;
            let target = (p as i64 + disp) as usize;
            mk(
                InstrKind::Jcc {
                    cond: Cond::from_low(op - 0x70),
                    target,
                },
                p - start,
            )
        }
        // jmp short / near.
        0xEB => {
            let disp = read_imm(code, p, 1)?;
            p += 1;
            let target = (p as i64 + disp) as usize;
            mk(InstrKind::Jmp { target }, p - start)
        }
        0xE9 => {
            let disp = read_imm(code, p, 4)? as i32 as i64;
            p += 4;
            let target = (p as i64 + disp) as usize;
            mk(InstrKind::Jmp { target }, p - start)
        }
        // call rel32.
        0xE8 => {
            let disp = read_imm(code, p, 4)? as i32 as i64;
            p += 4;
            let target = (p as i64 + disp) as usize;
            mk(InstrKind::Call { target }, p - start)
        }
        0xC3 => mk(InstrKind::Ret, p - start),
        0x90 => mk(InstrKind::Nop, p - start),
        // F7 /2 = neg, /3 = not (reg only).
        0xF7 => {
            let (rm, sub, used) = decode_modrm_sub(code, p, &rex)?;
            p += used;
            match sub {
                2 => mk(InstrKind::Neg(Operand::Reg(rm)), p - start),
                3 => mk(InstrKind::Not(Operand::Reg(rm)), p - start),
                _ => Err(JitError::Unsupported(format!("F7 sub-op {sub}"))),
            }
        }
        _ => Err(JitError::Unsupported(format!(
            "opcode {op:#04x}{}",
            if has_rex { " (REX)" } else { "" }
        ))),
    }
}

/// Decode ModR/M returning (rm_operand, reg_field, consumed).
/// `reg_field` is the raw 3-bit reg opcode (for group ops) or the
/// register operand (already REX-extended where applicable).
fn decode_modrm_sub(code: &[u8], pc: usize, rex: &Rex) -> JitResult<(Reg, u8, usize)> {
    if pc >= code.len() {
        return Err(JitError::ParseError("truncated modrm".into()));
    }
    let m = code[pc];
    if (m >> 6) != 3 {
        return Err(JitError::Unsupported(format!(
            "memory operand (modrm {m:#04x}) not supported"
        )));
    }
    let rm = (m & 7) | if rex.b { 8 } else { 0 };
    // Group-opcode extension: the /digit is NOT REX.R-extended.
    let reg = (m >> 3) & 7;
    Ok((rm, reg, 1))
}

/// Decode a single instruction at `pc`; returns (instr, len).
pub fn decode_one_pub(code: &[u8], pc: usize) -> JitResult<(X86Instr, usize)> {
    decode_one(code, pc)
}
