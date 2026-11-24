//! STATUS: research subset (x86-64 to WASM, interpreted). Not yet the Xbox path, see ADR 0001 phase 24E.
//! xHoly JIT subsystem — Phase 13.
//!
//! x86-64 → WASM JIT pipeline: decode → schedule → allocate → emit
//! real WASM 1.0 binaries → execute in the built-in interpreter
//! ([`runtime`]). No external dependencies — pure Rust.
//!
//! Honest scope: the decoder covers a useful x86-64 subset (integer
//! reg-reg ALU ops, immediates, branches, calls, push/pop, syscall);
//! memory operands, SIMD and most of the opcode space are rejected
//! with [`JitError::Unsupported`] rather than mistranslated.

pub mod allocator;
pub mod cache;
pub mod compiler;
pub mod disassembler;
pub mod emulator_loop;
pub mod exception;
pub mod flags;
pub mod memory;
pub mod runtime;
pub mod scheduler;
pub mod wasm_binary;

/// JIT error type.
#[derive(Debug)]
pub enum JitError {
    /// Byte sequence could not be decoded.
    ParseError(String),
    /// Valid x86-64 outside the supported subset (SIMD, memory ops,
    /// privileged instructions, ...).
    Unsupported(String),
    /// Register allocation failed.
    AllocationError(String),
    /// Address outside the managed address space.
    MemoryOverflow(u64),
    /// Unhandled guest/runtime exception.
    Exception(u64, String),
    /// Requested translation unit was evicted / not compiled.
    CacheMiss,
    /// Binary emission / relocation failed.
    RelocationError(String),
    /// Runtime failure (trap, bad call target, ...).
    Runtime(String),
    /// JIT path disabled and no interpreter fallback configured.
    FallbackDisabled,
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseError(s) => write!(f, "parse error: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::AllocationError(s) => write!(f, "allocation: {s}"),
            Self::MemoryOverflow(a) => write!(f, "memory overflow at {a:#x}"),
            Self::Exception(c, s) => write!(f, "exception {c}: {s}"),
            Self::CacheMiss => write!(f, "cache miss"),
            Self::RelocationError(s) => write!(f, "relocation: {s}"),
            Self::Runtime(s) => write!(f, "runtime: {s}"),
            Self::FallbackDisabled => write!(f, "jit disabled and fallback unavailable"),
        }
    }
}

impl std::error::Error for JitError {}

/// Result alias used across the JIT.
pub type JitResult<T> = Result<T, JitError>;

pub use compiler::{CompiledBlock, CompiledModule, JitCompiler};
pub use disassembler::{decode_all, disassemble, Cond, InstrKind, MemOperand, Operand, X86Instr};
pub use emulator_loop::{JitConfig, JitEmulator, WasmJitEmulator, X86Interpreter};
pub use flags::JitFlags;
pub use runtime::{JitRuntime, RuntimeConfig};
