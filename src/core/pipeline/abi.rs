//! Guest calling-convention and stack-argument ABI handling — Milestone 30.
//!
//! Keeps ABI rules explicit, configurable, and separate from both the
//! [`X64Cpu`] instruction interpreter and the [`crate::core::hle::runtime::HleRuntime`].
//!
//! Reference: Microsoft x64 calling convention
//! (<https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention>).
//! xHoly does **not** claim that every Xbox One or Xbox Series title uses an
//! identical ABI; unsupported conventions fail explicitly.

use std::fmt;

use serde::Serialize;

use crate::core::cpu::x64::{reg, X64Cpu, ARG_REGISTERS};
use crate::core::memory::GuestMemory;

/// Size of the Microsoft x64 shadow (home) space in bytes (4 × 8-byte slots).
pub const SHADOW_SPACE_SIZE: u64 = 32;

/// Offset from callee-entry `RSP` (after `CALL` pushed the 8-byte return
/// address) to the first stack-passed argument: `8 (ret) + 32 (shadow)`.
pub const STACK_ARGS_OFFSET: u64 = 8 + SHADOW_SPACE_SIZE;

/// Explicit calling-convention policy for HLE transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CallingConvention {
    /// Documented Microsoft x64 integer ABI:
    /// - Args `0..4` in `RCX`, `RDX`, `R8`, `R9`
    /// - 32-byte shadow space at `[RSP + 8 .. RSP + 40)` on callee entry
    /// - Stack-passed integer args `4..` at `[RSP + 40 + 8 * (i - 4)]`
    /// - 16-byte pre-call `RSP` alignment (`RSP % 16 == 8` on callee entry)
    /// - Return value in `RAX`
    ///
    /// When `strict_prologue` is `false`, minimal synthetic callers with
    /// `arity <= 4` are allowed without shadow space or 16-byte pre-call
    /// alignment; calls with `arity > 4` always enforce full stack layout.
    MicrosoftX64 {
        /// Require 16-byte pre-call alignment and 32-byte shadow space even
        /// when `arity <= 4`.
        strict_prologue: bool,
    },
    /// Explicitly unsupported calling convention (e.g., SysV AMD64, vectorcall,
    /// or floating-point/XMM arguments).
    Unsupported(&'static str),
}

impl Default for CallingConvention {
    fn default() -> Self {
        Self::MicrosoftX64 {
            strict_prologue: false,
        }
    }
}

/// Structured ABI failure raised during HLE argument extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AbiFault {
    /// The configured calling convention is not supported.
    UnsupportedConvention {
        /// Name of the unsupported convention.
        convention: String,
    },
    /// Callee-entry `RSP` does not satisfy the convention's alignment rule.
    MisalignedStack {
        /// Observed `RSP` at callee entry (after `CALL` pushed the return address).
        rsp: u64,
        /// Expected `rsp % 16` remainder (8 for Microsoft x64 callee entry).
        expected_mod16: u64,
    },
    /// The 32-byte Microsoft x64 shadow space is outside stack bounds or unmapped.
    InvalidShadowSpace {
        /// Start address of the shadow space (`RSP + 8`).
        start: u64,
        /// Required size in bytes (32).
        size: u64,
    },
    /// A stack argument slot lies outside the configured guest stack bounds.
    StackArgumentOutOfBounds {
        /// Zero-based argument index (`>= 4`).
        index: usize,
        /// Computed guest address of the stack argument slot.
        addr: u64,
    },
    /// A stack argument slot could not be read from guest memory.
    StackArgumentReadFailed {
        /// Zero-based argument index (`>= 4`).
        index: usize,
        /// Guest address of the stack argument slot.
        addr: u64,
    },
}

impl fmt::Display for AbiFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedConvention { convention } => {
                write!(f, "unsupported calling convention: {convention}")
            }
            Self::MisalignedStack {
                rsp,
                expected_mod16,
            } => write!(
                f,
                "misaligned stack at HLE entry: rsp={rsp:#x} (expected rsp % 16 == {expected_mod16})"
            ),
            Self::InvalidShadowSpace { start, size } => write!(
                f,
                "invalid {size}-byte shadow space at {start:#x}"
            ),
            Self::StackArgumentOutOfBounds { index, addr } => write!(
                f,
                "stack argument #{index} at {addr:#x} is outside guest stack bounds"
            ),
            Self::StackArgumentReadFailed { index, addr } => write!(
                f,
                "failed to read stack argument #{index} at {addr:#x} from guest memory"
            ),
        }
    }
}

impl std::error::Error for AbiFault {}

/// Read `arity` integer arguments for an HLE call at callee entry
/// (immediately after `CALL` pushed the 8-byte return address).
pub fn read_hle_arguments<M: GuestMemory>(
    cpu: &X64Cpu,
    mem: &M,
    convention: CallingConvention,
    arity: usize,
) -> Result<Vec<u64>, AbiFault> {
    match convention {
        CallingConvention::Unsupported(name) => Err(AbiFault::UnsupportedConvention {
            convention: name.into(),
        }),
        CallingConvention::MicrosoftX64 { strict_prologue } => {
            let rsp = cpu.regs[reg::RSP];
            let needs_stack_layout = strict_prologue || arity > ARG_REGISTERS.len();

            if needs_stack_layout {
                if rsp % 16 != 8 {
                    return Err(AbiFault::MisalignedStack {
                        rsp,
                        expected_mod16: 8,
                    });
                }

                let shadow_start = rsp.checked_add(8).ok_or(AbiFault::InvalidShadowSpace {
                    start: rsp,
                    size: SHADOW_SPACE_SIZE,
                })?;
                if let Some(bounds) = cpu.stack_bounds() {
                    if !bounds.contains_range(shadow_start, SHADOW_SPACE_SIZE) {
                        return Err(AbiFault::InvalidShadowSpace {
                            start: shadow_start,
                            size: SHADOW_SPACE_SIZE,
                        });
                    }
                }
                if mem
                    .read_bytes(shadow_start, SHADOW_SPACE_SIZE as usize)
                    .is_none()
                {
                    return Err(AbiFault::InvalidShadowSpace {
                        start: shadow_start,
                        size: SHADOW_SPACE_SIZE,
                    });
                }
            }

            let mut args = cpu.read_args(arity);
            for index in ARG_REGISTERS.len()..arity {
                let offset = STACK_ARGS_OFFSET + 8 * (index - ARG_REGISTERS.len()) as u64;
                let addr = rsp
                    .checked_add(offset)
                    .ok_or(AbiFault::StackArgumentOutOfBounds {
                        index,
                        addr: u64::MAX,
                    })?;
                if let Some(bounds) = cpu.stack_bounds() {
                    if !bounds.contains_range(addr, 8) {
                        return Err(AbiFault::StackArgumentOutOfBounds { index, addr });
                    }
                }
                let value = mem
                    .read_u64(addr)
                    .ok_or(AbiFault::StackArgumentReadFailed { index, addr })?;
                args.push(value);
            }
            Ok(args)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cpu::x64::StackBounds;
    use crate::core::memory::{regions, Memory};

    #[test]
    fn rejects_invalid_shadow_space_and_out_of_bounds_stack_args() {
        let mut mem = Memory::new();
        let top = regions::TITLE + 0x10_0000;
        let bottom = top - 0x1_0000;
        let mut cpu = X64Cpu::new();
        cpu.set_stack_bounds(StackBounds::new(bottom, top));
        cpu.regs[reg::RCX] = 1;
        cpu.regs[reg::RDX] = 2;
        cpu.regs[reg::R8] = 3;
        cpu.regs[reg::R9] = 4;

        // Place RSP so RSP % 16 == 8, but only 16 bytes remain below `top` (< 32-byte shadow space).
        cpu.regs[reg::RSP] = top - 24;
        assert_eq!(
            read_hle_arguments(
                &cpu,
                &mem,
                CallingConvention::MicrosoftX64 {
                    strict_prologue: true
                },
                4
            ),
            Err(AbiFault::InvalidShadowSpace {
                start: top - 16,
                size: SHADOW_SPACE_SIZE,
            })
        );

        // Place RSP so 32-byte shadow space fits, but 5th arg at [RSP + 40] is >= top.
        cpu.regs[reg::RSP] = top - 40;
        assert_eq!(
            read_hle_arguments(
                &cpu,
                &mem,
                CallingConvention::MicrosoftX64 {
                    strict_prologue: false
                },
                5
            ),
            Err(AbiFault::StackArgumentOutOfBounds {
                index: 4,
                addr: top,
            })
        );

        // Place RSP so 5th arg fits and succeeds.
        cpu.regs[reg::RSP] = top - 56;
        assert!(mem.write_u64(top - 16, 55));
        assert_eq!(
            read_hle_arguments(
                &cpu,
                &mem,
                CallingConvention::MicrosoftX64 {
                    strict_prologue: false
                },
                5
            ),
            Ok(vec![1, 2, 3, 4, 55])
        );
    }
}
