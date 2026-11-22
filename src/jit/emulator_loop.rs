//! JIT emulator loop — Phase 14 integration layer.
//!
//! [`JitEmulator`] abstracts the translate-and-run path so
//! [`crate::core::EmulationCore`] can treat the x86-64 JIT as an
//! optional branch next to the ARM64 interpreter.
//!
//! [`WasmJitEmulator`] wraps [`JitCompiler`] + [`JitRuntime`]:
//! `compile` produces WASM blocks and loads them into the runtime,
//! `execute` drives the block-dispatch loop, `exception` unwinds the
//! [`ExceptionHandler`] stack.
//!
//! [`X86Interpreter`] is the honest fallback: a scalar interpreter
//! over the *same decoded instruction subset* (including memory
//! operands and the integer SIMD ops). It is slower than the JIT but
//! executes real semantics — used when `fallback_to_interpreter` is
//! set or the WASM runtime is unavailable.

use super::compiler::{CompiledBlock, JitCompiler};
use super::disassembler::{decode_one_pub, Cond, InstrKind, MemOperand, Operand, SimdOp};
use super::exception::{ExceptionFrame, ExceptionHandler};
use super::flags::JitFlags;
use super::runtime::{JitRuntime, RuntimeConfig};
use super::{JitError, JitResult};
use std::cell::RefCell;

/// JIT configuration extracted from the emulator config.
#[derive(Debug, Clone)]
pub struct JitConfig {
    /// Master switch — when false the JIT path is not used.
    pub enabled: bool,
    /// Max executed WASM steps / interpreted instructions.
    pub max_instructions: u64,
    /// Wall-clock budget per `run` call in milliseconds (0 = none).
    /// Enforced between block dispatches.
    pub timeout_ms: u64,
    /// On JIT failure, fall back to the scalar x86 interpreter.
    pub fallback_to_interpreter: bool,
}

impl Default for JitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_instructions: 10_000_000,
            timeout_ms: 0,
            fallback_to_interpreter: true,
        }
    }
}

/// The JIT emulator contract used by the emulation core.
pub trait JitEmulator {
    /// Decode + compile `program` into executable blocks.
    fn compile(&mut self, program: &[u8], config: &JitConfig) -> JitResult<Vec<CompiledBlock>>;
    /// Execute compiled blocks starting at guest offset `entry`;
    /// returns the rax value at halt.
    fn execute(&mut self, blocks: &[CompiledBlock], entry: usize) -> JitResult<u64>;
    /// Register/raise an exception frame; returns the resume PC.
    fn exception(&mut self, frame: ExceptionFrame) -> JitResult<u64>;
}

// ------------------------------------------------------------------
// Host syscall plumbing
// ------------------------------------------------------------------

thread_local! {
    /// Optional user-registered syscall handler (x86-64 `syscall`).
    /// Receives the machine (GPR globals = architectural regs) and
    /// returns the value placed in rax.
    static SYSCALL_HANDLER: RefCell<Option<super::runtime::HostFn>> = const { RefCell::new(None) };
}

/// Install a host syscall handler for subsequently loaded modules.
/// The handler sees GPRs as globals 0–15 (rax=0 … r15=15) and the
/// WASM linear memory as guest RAM.
pub fn set_syscall_handler(f: super::runtime::HostFn) {
    SYSCALL_HANDLER.with(|h| *h.borrow_mut() = Some(f));
}

/// Clear the registered syscall handler.
pub fn clear_syscall_handler() {
    SYSCALL_HANDLER.with(|h| *h.borrow_mut() = None);
}

fn host_dispatch(machine: &mut super::runtime::Machine) -> JitResult<i64> {
    SYSCALL_HANDLER.with(|h| {
        if let Some(f) = h.borrow_mut().as_mut() {
            f(machine)
        } else {
            // Default: syscall nr in rax, args in rdi/rsi/rdx/r10/r8/r9.
            // No kernel attached → return 0 (success-ish stub).
            let _nr = machine.globals[0].as_i64();
            let _args = [
                machine.globals[7].as_i64(),
                machine.globals[6].as_i64(),
                machine.globals[2].as_i64(),
                machine.globals[8].as_i64(),
                machine.globals[9].as_i64(),
                machine.globals[10].as_i64(),
            ];
            Ok(0)
        }
    })
}

// ------------------------------------------------------------------
// WasmJitEmulator
// ------------------------------------------------------------------

/// JIT backend: x86-64 → WASM → built-in interpreter.
pub struct WasmJitEmulator {
    compiler: JitCompiler,
    runtime: JitRuntime,
    module: Option<super::compiler::CompiledModule>,
    exceptions: ExceptionHandler,
    interpreter: X86Interpreter,
}

impl WasmJitEmulator {
    /// New emulator with default configs.
    pub fn new() -> Self {
        Self {
            compiler: JitCompiler::new(),
            runtime: JitRuntime::new(),
            module: None,
            exceptions: ExceptionHandler::new(),
            interpreter: X86Interpreter::new(),
        }
    }

    /// Access the scalar fallback interpreter (e.g. to inspect regs).
    pub fn interpreter(&self) -> &X86Interpreter {
        &self.interpreter
    }

    /// Access the interpreter mutably.
    pub fn interpreter_mut(&mut self) -> &mut X86Interpreter {
        &mut self.interpreter
    }

    /// Access the WASM runtime (registers live in its globals).
    pub fn runtime(&self) -> &JitRuntime {
        &self.runtime
    }

    /// Full pipeline: compile then execute. On JIT failure and
    /// `fallback_to_interpreter`, runs the scalar interpreter instead.
    pub fn run_program(&mut self, program: &[u8], config: &JitConfig) -> JitResult<u64> {
        if config.enabled {
            match self
                .compile(program, config)
                .and_then(|b| self.execute(&b, 0))
            {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if config.fallback_to_interpreter {
                        log::debug!("xHoly JIT failed ({e}); using interpreter fallback");
                        return self.interpreter.run(program, config.max_instructions);
                    }
                    return Err(e);
                }
            }
        }
        if config.fallback_to_interpreter {
            return self.interpreter.run(program, config.max_instructions);
        }
        Err(JitError::FallbackDisabled)
    }
}

impl Default for WasmJitEmulator {
    fn default() -> Self {
        Self::new()
    }
}

impl JitEmulator for WasmJitEmulator {
    fn compile(&mut self, program: &[u8], config: &JitConfig) -> JitResult<Vec<CompiledBlock>> {
        let module = self.compiler.compile(program)?;
        let rt_cfg = RuntimeConfig {
            max_steps: config.max_instructions,
            ..RuntimeConfig::default()
        };
        self.runtime = JitRuntime::with_config(rt_cfg);
        self.runtime.load(
            &module.wasm,
            module.block_map.clone(),
            vec![Box::new(host_dispatch)],
        )?;
        let blocks = module.blocks.clone();
        self.module = Some(module);
        Ok(blocks)
    }

    fn execute(&mut self, blocks: &[CompiledBlock], entry: usize) -> JitResult<u64> {
        // entry must be a known block offset.
        let offset = blocks
            .iter()
            .find(|b| b.offset == entry as u64)
            .map(|b| b.offset)
            .ok_or_else(|| {
                JitError::Runtime(format!("entry offset {entry:#x} is not a block start"))
            })?;
        let _ = offset;
        let v = self.runtime.run(entry as u64)?;
        Ok(v as u64)
    }

    fn exception(&mut self, frame: ExceptionFrame) -> JitResult<u64> {
        self.exceptions.push(frame);
        match self.exceptions.pop() {
            Some(f) => Ok(f.resume_pc),
            None => Err(JitError::Exception(
                u64::MAX,
                "empty exception stack".into(),
            )),
        }
    }
}

// ------------------------------------------------------------------
// X86Interpreter — scalar fallback over the decoded subset.
// ------------------------------------------------------------------

/// Guest memory size for the interpreter (flat address space).
const INTERP_MEM_SIZE: usize = 2 * 1024 * 1024;

/// Syscall handler signature for [`X86Interpreter`].
pub type SyscallHandler = Box<dyn FnMut(&mut X86Interpreter) -> JitResult<i64>>;

/// Scalar x86-64 interpreter for the JIT-supported subset.
///
/// Executes [`decode_one_pub`] instructions directly: GPRs in
/// `regs[0..16]`, XMM as `[u64; 2]` lane pairs, flags in [`JitFlags`],
/// a flat `mem` with `rsp`-relative stack. Returns rax at halt.
pub struct X86Interpreter {
    /// GPRs rax…r15.
    pub regs: [u64; 16],
    /// XMM0–15 as (lo, hi) i64 lanes.
    pub xmm: [[u64; 2]; 16],
    /// Architectural flags (tracked subset).
    pub flags: JitFlags,
    /// Flat guest memory; rsp indexes into it.
    pub mem: Vec<u8>,
    /// Guest call/return stack (byte offsets into `code`).
    ret_stack: Vec<usize>,
    /// Instructions executed (step counter).
    pub steps: u64,
    /// Optional syscall handler: receives &mut self, returns rax value.
    pub syscall_handler: Option<SyscallHandler>,
}

impl X86Interpreter {
    /// New interpreter with `INTERP_MEM_SIZE` bytes and rsp at top.
    pub fn new() -> Self {
        let mut mem = vec![0u8; INTERP_MEM_SIZE];
        // Sentinel return address at top of stack.
        let sp = INTERP_MEM_SIZE - 8;
        mem[sp..sp + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut regs = [0u64; 16];
        regs[4] = sp as u64; // rsp
        Self {
            regs,
            xmm: [[0u64; 2]; 16],
            flags: JitFlags::new(),
            mem,
            ret_stack: Vec::new(),
            steps: 0,
            syscall_handler: None,
        }
    }

    /// Effective address of a memory operand.
    fn addr_of(&self, m: &MemOperand) -> JitResult<u64> {
        let mut a = m.disp as u64;
        if let Some(b) = m.base {
            a = a.wrapping_add(self.regs[b as usize]);
        }
        if let Some(i) = m.index {
            a = a.wrapping_add(self.regs[i as usize].wrapping_mul(m.scale as u64));
        }
        if (a as usize) + 8 > self.mem.len() {
            return Err(JitError::MemoryOverflow(a));
        }
        Ok(a)
    }

    /// Read an operand as u64 (memory loads are little-endian).
    fn read_op(&self, o: &Operand) -> JitResult<u64> {
        match o {
            Operand::Reg(r) => Ok(self.regs[*r as usize]),
            Operand::Imm(v) => Ok(*v as u64),
            Operand::Mem(m) => {
                let a = self.addr_of(m)? as usize;
                Ok(u64::from_le_bytes(self.mem[a..a + 8].try_into().unwrap()))
            }
            Operand::Xmm(_) => Err(JitError::Unsupported("scalar read of xmm".into())),
        }
    }

    /// Write an operand.
    fn write_op(&mut self, o: &Operand, v: u64) -> JitResult<()> {
        match o {
            Operand::Reg(r) => {
                self.regs[*r as usize] = v;
                Ok(())
            }
            Operand::Imm(_) => Err(JitError::Unsupported("write to imm".into())),
            Operand::Mem(m) => {
                let a = self.addr_of(m)? as usize;
                self.mem[a..a + 8].copy_from_slice(&v.to_le_bytes());
                Ok(())
            }
            Operand::Xmm(_) => Err(JitError::Unsupported("scalar write to xmm".into())),
        }
    }

    /// Read a 128-bit value from a memory operand (bounds-checked).
    fn read_m128(&self, o: Operand) -> JitResult<[u64; 2]> {
        match o {
            Operand::Mem(m) => {
                let a = self.addr_of(&m)? as usize;
                if a + 16 > self.mem.len() {
                    return Err(JitError::MemoryOverflow(a as u64 + 8));
                }
                Ok([
                    u64::from_le_bytes(self.mem[a..a + 8].try_into().unwrap()),
                    u64::from_le_bytes(self.mem[a + 8..a + 16].try_into().unwrap()),
                ])
            }
            _ => Err(JitError::Unsupported("expected m128".into())),
        }
    }

    /// Write a 128-bit value to a memory operand.
    fn write_m128(&mut self, o: Operand, v: [u64; 2]) -> JitResult<()> {
        match o {
            Operand::Mem(m) => {
                let a = self.addr_of(&m)? as usize;
                if a + 16 > self.mem.len() {
                    return Err(JitError::MemoryOverflow(a as u64 + 8));
                }
                self.mem[a..a + 8].copy_from_slice(&v[0].to_le_bytes());
                self.mem[a + 8..a + 16].copy_from_slice(&v[1].to_le_bytes());
                Ok(())
            }
            _ => Err(JitError::Unsupported("expected m128 dst".into())),
        }
    }

    /// Run `program` from offset 0; returns rax at halt.
    /// `max_steps` bounds the instruction count.
    pub fn run(&mut self, program: &[u8], max_steps: u64) -> JitResult<u64> {
        let mut pc = 0usize;
        let code_len = program.len();
        while pc < code_len {
            if self.steps >= max_steps {
                return Err(JitError::Runtime("interpreter step limit".into()));
            }
            let (ins, len) = decode_one_pub(program, pc)?;
            self.steps += 1;
            pc += len;
            match ins.kind {
                InstrKind::Nop => {}
                InstrKind::Mov { dst, src } => {
                    let v = self.read_op(&src)?;
                    self.write_op(&dst, v)?;
                }
                InstrKind::Movzx { dst, src, bits } | InstrKind::Movsx { dst, src, bits } => {
                    let v = self.read_op(&src)?;
                    let r = match (bits, matches!(ins.kind, InstrKind::Movzx { .. })) {
                        (8, true) => v & 0xFF,
                        (8, false) => (v as i8 as i64) as u64,
                        (16, true) => v & 0xFFFF,
                        (16, false) => (v as i16 as i64) as u64,
                        (_, true) => v & 0xFFFF_FFFF,
                        (_, false) => v as i32 as i64 as u64,
                    };
                    self.write_op(&dst, r)?;
                }
                InstrKind::Add { dst, src }
                | InstrKind::Sub { dst, src }
                | InstrKind::Xor { dst, src }
                | InstrKind::And { dst, src }
                | InstrKind::Or { dst, src }
                | InstrKind::Mul { dst, src } => {
                    let a = self.read_op(&dst)?;
                    let b = self.read_op(&src)?;
                    let (res, is_sub, is_logic) = match ins.kind {
                        InstrKind::Add { .. } => (a.wrapping_add(b), false, false),
                        InstrKind::Sub { .. } => (a.wrapping_sub(b), true, false),
                        InstrKind::Mul { .. } => (a.wrapping_mul(b), false, true),
                        InstrKind::Xor { .. } => (a ^ b, false, true),
                        InstrKind::And { .. } => (a & b, false, true),
                        _ => (a | b, false, true),
                    };
                    self.write_op(&dst, res)?;
                    self.flags.update_szp(res);
                    if is_logic || matches!(ins.kind, InstrKind::Mul { .. }) {
                        self.flags.clear_cf();
                        self.flags.clear_of();
                        self.flags.clear_af();
                    } else {
                        // CF/OF/AF
                        if is_sub {
                            if a < b {
                                self.flags.set_cf();
                            } else {
                                self.flags.clear_cf();
                            }
                            if ((a ^ b) & (a ^ res)) >> 63 != 0 {
                                self.flags.set_of();
                            } else {
                                self.flags.clear_of();
                            }
                        } else {
                            if res < a {
                                self.flags.set_cf();
                            } else {
                                self.flags.clear_cf();
                            }
                            if ((a ^ res) & (b ^ res)) >> 63 != 0 {
                                self.flags.set_of();
                            } else {
                                self.flags.clear_of();
                            }
                        }
                        if JitFlags::aux_carry_add(a, b, res) {
                            self.flags.set_af();
                        } else {
                            self.flags.clear_af();
                        }
                    }
                }
                InstrKind::Cmp { a, b } => {
                    let av = self.read_op(&a)?;
                    let bv = self.read_op(&b)?;
                    let res = av.wrapping_sub(bv);
                    self.flags.update_szp(res);
                    if av < bv {
                        self.flags.set_cf();
                    } else {
                        self.flags.clear_cf();
                    }
                    if ((av ^ bv) & (av ^ res)) >> 63 != 0 {
                        self.flags.set_of();
                    } else {
                        self.flags.clear_of();
                    }
                    if JitFlags::aux_carry_add(av, bv, res) {
                        self.flags.set_af();
                    } else {
                        self.flags.clear_af();
                    }
                }
                InstrKind::Test { a, b } => {
                    let res = self.read_op(&a)? & self.read_op(&b)?;
                    self.flags.update_szp(res);
                    self.flags.clear_cf();
                    self.flags.clear_of();
                    self.flags.clear_af();
                }
                InstrKind::Inc(r) | InstrKind::Dec(r) | InstrKind::Neg(r) | InstrKind::Not(r) => {
                    let is_dec = matches!(ins.kind, InstrKind::Dec(_));
                    let is_neg = matches!(ins.kind, InstrKind::Neg(_));
                    let is_not = matches!(ins.kind, InstrKind::Not(_));
                    let a = self.read_op(&r)?;
                    let res = if is_not {
                        !a
                    } else if is_neg {
                        0u64.wrapping_sub(a)
                    } else if is_dec {
                        a.wrapping_sub(1)
                    } else {
                        a.wrapping_add(1)
                    };
                    self.write_op(&r, res)?;
                    if is_not {
                        continue;
                    }
                    self.flags.update_szp(res);
                    if is_neg {
                        if a != 0 {
                            self.flags.set_cf();
                        } else {
                            self.flags.clear_cf();
                        }
                        if a == i64::MIN as u64 {
                            self.flags.set_of();
                        } else {
                            self.flags.clear_of();
                        }
                        if ((a ^ res) >> 4) & 1 == 1 {
                            self.flags.set_af();
                        } else {
                            self.flags.clear_af();
                        }
                    } else {
                        let boundary = if is_dec {
                            i64::MIN as u64
                        } else {
                            i64::MAX as u64
                        };
                        if a == boundary {
                            self.flags.set_of();
                        } else {
                            self.flags.clear_of();
                        }
                        // AF ≈ ((a ^ 1 ^ res) >> 4) & 1 for inc/dec.
                        if ((a ^ 1 ^ res) >> 4) & 1 == 1 {
                            self.flags.set_af();
                        } else {
                            self.flags.clear_af();
                        }
                        // CF preserved.
                    }
                }
                InstrKind::Push(r) => {
                    let v = self.read_op(&r)?;
                    let sp = self.regs[4].wrapping_sub(8);
                    if sp as usize + 8 > self.mem.len() {
                        return Err(JitError::MemoryOverflow(sp));
                    }
                    self.mem[sp as usize..sp as usize + 8].copy_from_slice(&v.to_le_bytes());
                    self.regs[4] = sp;
                }
                InstrKind::Pop(r) => {
                    let sp = self.regs[4];
                    if sp as usize + 8 > self.mem.len() {
                        return Err(JitError::MemoryOverflow(sp));
                    }
                    let v = u64::from_le_bytes(
                        self.mem[sp as usize..sp as usize + 8].try_into().unwrap(),
                    );
                    self.regs[4] = sp.wrapping_add(8);
                    self.write_op(&r, v)?;
                }
                InstrKind::Jmp { target } => {
                    pc = target;
                }
                InstrKind::Jcc { cond, target } => {
                    if self.cond_true(cond) {
                        pc = target;
                    }
                }
                InstrKind::Call { target } => {
                    self.ret_stack.push(pc);
                    pc = target;
                }
                InstrKind::Ret => {
                    match self.ret_stack.pop() {
                        Some(t) => pc = t,
                        None => break, // halt
                    }
                }
                InstrKind::Syscall => {
                    // Take the handler out to satisfy the borrow checker.
                    let mut h = self.syscall_handler.take();
                    let v = match h.as_mut() {
                        Some(f) => f(self)?,
                        None => 0,
                    };
                    self.syscall_handler = h;
                    self.regs[0] = v as u64;
                }
                InstrKind::SimdMov { dst, src } => {
                    // xmm↔xmm register move, or xmm↔m128 memory move.
                    match (dst, src) {
                        (Operand::Xmm(d), Operand::Xmm(s)) => {
                            let v = self.xmm[s as usize];
                            self.xmm[d as usize] = v;
                        }
                        (Operand::Mem(_), Operand::Xmm(s)) => {
                            let v = self.xmm[s as usize];
                            self.write_m128(dst, v)?;
                        }
                        (Operand::Xmm(d), Operand::Mem(_)) => {
                            let v = self.read_m128(src)?;
                            self.xmm[d as usize] = v;
                        }
                        _ => {
                            return Err(JitError::Unsupported("simd mov operands".into()));
                        }
                    }
                }
                InstrKind::SimdMovd {
                    xmm,
                    gpr,
                    to_xmm,
                    bits,
                } => {
                    if let Operand::Xmm(x) = xmm {
                        let mask = if bits == 64 { u64::MAX } else { 0xFFFF_FFFF };
                        if to_xmm {
                            let v = self.read_op(&gpr)? & mask;
                            self.xmm[x as usize] = [v, 0];
                        } else {
                            let v = self.xmm[x as usize][0] & mask;
                            self.write_op(&gpr, v)?;
                        }
                    }
                }
                InstrKind::SimdAlu { op: sop, dst, src } => {
                    if let Operand::Xmm(d) = dst {
                        let di = d as usize;
                        let mut dv = self.xmm[di];
                        let sv = if let Operand::Xmm(s) = src {
                            self.xmm[s as usize]
                        } else if matches!(src, Operand::Mem(_)) {
                            self.read_m128(src)?
                        } else {
                            return Err(JitError::Unsupported("simd src".into()));
                        };
                        for l in 0..2 {
                            dv[l] = match sop {
                                SimdOp::Paddq => dv[l].wrapping_add(sv[l]),
                                SimdOp::Psubq => dv[l].wrapping_sub(sv[l]),
                                SimdOp::Pand => dv[l] & sv[l],
                                SimdOp::Por => dv[l] | sv[l],
                                SimdOp::Pxor => dv[l] ^ sv[l],
                            };
                        }
                        self.xmm[di] = dv;
                    } else {
                        return Err(JitError::Unsupported("simd dst not xmm".into()));
                    }
                }
            }
        }
        Ok(self.regs[0])
    }

    /// Evaluate a condition code against current flags.
    fn cond_true(&self, c: Cond) -> bool {
        let f = &self.flags;
        let cf = f.contains(super::flags::bits::CF);
        let pf = f.contains(super::flags::bits::PF);
        let _af = f.contains(super::flags::bits::AF);
        let zf = f.contains(super::flags::bits::ZF);
        let sf = f.contains(super::flags::bits::SF);
        let of = f.contains(super::flags::bits::OF);
        match c {
            Cond::O => of,
            Cond::No => !of,
            Cond::B => cf,
            Cond::Ae => !cf,
            Cond::E => zf,
            Cond::Ne => !zf,
            Cond::Be => cf || zf,
            Cond::A => !cf && !zf,
            Cond::S => sf,
            Cond::Ns => !sf,
            Cond::P => pf,
            Cond::Np => !pf,
            Cond::L => sf != of,
            Cond::Ge => sf == of,
            Cond::Le => zf || (sf != of),
            Cond::G => !zf && (sf == of),
        }
    }
}

impl Default for X86Interpreter {
    fn default() -> Self {
        Self::new()
    }
}
