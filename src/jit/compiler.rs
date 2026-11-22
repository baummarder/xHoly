//! x86-64 → WASM compiler — Phase 13.
//!
//! Pipeline: [`decode_all`] → split into basic blocks at control-flow
//! boundaries → schedule each block → lower to WASM bytecode →
//! [`wasm_binary::WasmModule`] → bytes.
//!
//! ## Execution model
//!
//! Each basic block becomes one WASM function `[] -> [i64]` that
//! returns the **guest byte offset of the next instruction** to
//! execute (`-1` = halt). The runtime keeps a `offset → func` table
//! and tail-calls in a loop — this models x86 control flow without
//! needing structured WASM control instructions, so arbitrary jumps
//! (including `call`/`ret` via the guest stack) work.
//!
//! ## State mapping
//!
//! - GPRs rax…r15 → mutable `i64` globals 0–15 (rsp = global 4).
//! - Flags zf/sf/cf/of/pf/af → mutable `i32` globals 16–21.
//! - XMM0–15 → `i64` globals 22–53 (lo/hi lane pair per register).
//! - `syscall` → `call` to imported host function `xholy.syscall`
//!   `[] -> [i64]`; result lands in rax.
//! - Expression temporaries → WASM locals (5×i64: a, b, res, t, u).
//!
//! ## Honest limitations
//!
//! - RIP-relative `mod=0 rm=101` memory operands decode as absolute
//!   disp32 (documented in the decoder).
//! - SIMD covers integer lane-wise ops only (paddq/psubq/pand/por/
//!   pxor, movd/movq/movdqa/movdqu); packed FP and byte/word lanes
//!   are rejected.
//! - PF is computed exactly (even parity of the low result byte);
//!   AF follows the standard `(a^b^res)>>4 & 1` rule.

use super::allocator::Allocator;
use super::disassembler::{decode_all, Cond, InstrKind, MemOperand, Operand, SimdOp, X86Instr};
use super::scheduler::Scheduler;
use super::wasm_binary::{CodeEntry, FuncType, Global, Import, ValType, WasmModule};
use super::{JitError, JitResult};
use std::collections::HashMap;

/// Number of GPR globals (rax…r15).
const N_GPRS: u32 = 16;
/// Global index of rsp (reg 4).
const G_RSP: u32 = 4;
/// First flag global (zf, sf, cf, of, pf, af as i32).
const G_ZF: u32 = N_GPRS;
const G_SF: u32 = N_GPRS + 1;
const G_CF: u32 = N_GPRS + 2;
const G_OF: u32 = N_GPRS + 3;
const G_PF: u32 = N_GPRS + 4;
const G_AF: u32 = N_GPRS + 5;
/// Number of i32 flag globals.
const N_FLAGS: u32 = 6;
/// First XMM global: xmmN occupies `G_XMM0 + 2N` (lo) and `+1` (hi).
const G_XMM0: u32 = N_GPRS + N_FLAGS;

/// WASM locals per block function: [a, b, res, t, u] i64 temps.
const N_LOCALS: u32 = 5;
const L_A: u32 = 0;
const L_B: u32 = 1;
const L_R: u32 = 2;
const L_T: u32 = 3;
#[allow(dead_code)]
const L_U: u32 = 4;

/// One compiled basic block.
#[derive(Debug, Clone)]
pub struct CompiledBlock {
    /// Guest byte offset of the block's first instruction.
    pub offset: u64,
    /// WASM function index (in the module's function index space,
    /// imports included).
    pub func_index: u32,
    /// Number of x86 instructions in the block.
    pub instr_count: usize,
}

/// Result of [`JitCompiler::compile`]: WASM bytes + dispatch tables.
#[derive(Debug)]
pub struct CompiledModule {
    /// Serialized WASM 1.0 binary.
    pub wasm: Vec<u8>,
    /// Compiled basic blocks (offset → func index).
    pub blocks: Vec<CompiledBlock>,
    /// guest offset → WASM function index map for the runtime.
    pub block_map: HashMap<u64, u32>,
}

/// JIT compiler: decode → schedule → emit.
pub struct JitCompiler {
    scheduler: Scheduler,
    /// Reserved for IR-level temp allocation (pipeline stage 2);
    /// the current lowering uses fixed WASM locals instead.
    #[allow(dead_code)]
    allocator: Allocator,
    /// Pending compile results.
    pub blocks: Vec<CompiledBlock>,
    /// offset → func_index map for the runtime.
    pub block_map: HashMap<u64, u32>,
}

/// WASM opcode bytes used by the emitter. The full table is kept as
/// documentation of the supported subset; unused entries are allowed.
#[allow(dead_code)]
mod op {
    pub const UNREACHABLE: u8 = 0x00;
    pub const CALL: u8 = 0x10;
    pub const DROP: u8 = 0x1A;
    pub const SELECT: u8 = 0x1B;
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const LOCAL_TEE: u8 = 0x22;
    pub const GLOBAL_GET: u8 = 0x23;
    pub const GLOBAL_SET: u8 = 0x24;
    pub const I32_LOAD: u8 = 0x28;
    pub const I64_LOAD: u8 = 0x29;
    pub const I32_STORE: u8 = 0x36;
    pub const I64_STORE: u8 = 0x37;
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_GT_U: u8 = 0x4B;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I64_EQZ: u8 = 0x50;
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_LT_U: u8 = 0x54;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_GT_U: u8 = 0x56;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_LE_U: u8 = 0x58;
    pub const I64_GE_S: u8 = 0x59;
    pub const I64_GE_U: u8 = 0x5A;
    pub const I64_ADD: u8 = 0x7C;
    pub const I64_SUB: u8 = 0x7D;
    pub const I64_MUL: u8 = 0x7E;
    pub const I64_AND: u8 = 0x83;
    pub const I64_OR: u8 = 0x84;
    pub const I64_XOR: u8 = 0x85;
    pub const I64_SHL: u8 = 0x86;
    pub const I64_SHR_S: u8 = 0x87;
    pub const I64_SHR_U: u8 = 0x88;
    pub const I32_WRAP_I64: u8 = 0xA7;
    pub const I64_EXTEND8_S: u8 = 0xC2;
    pub const I64_EXTEND16_S: u8 = 0xC3;
    pub const I64_EXTEND32_S: u8 = 0xC4;
}

/// Byte emitter with WASM LEB helpers.
struct Emit {
    buf: Vec<u8>,
}

impl Emit {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }
    fn b(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u(&mut self, v: u32) {
        super::wasm_binary::uleb(&mut self.buf, v as u64);
    }
    fn i64c(&mut self, v: i64) {
        self.b(op::I64_CONST);
        super::wasm_binary::sleb(&mut self.buf, v);
    }
    fn i32c(&mut self, v: i32) {
        self.b(op::I32_CONST);
        super::wasm_binary::sleb(&mut self.buf, v as i64);
    }
    fn gget(&mut self, g: u32) {
        self.b(op::GLOBAL_GET);
        self.u(g);
    }
    fn gset(&mut self, g: u32) {
        self.b(op::GLOBAL_SET);
        self.u(g);
    }
    fn lget(&mut self, l: u32) {
        self.b(op::LOCAL_GET);
        self.u(l);
    }
    /// Push the i32 linear address of a memory operand.
    /// `base + index*scale + disp` (missing parts contribute 0).
    fn addr_i32(&mut self, m: &MemOperand) {
        if m.base.is_none() && m.index.is_none() {
            self.i64c(m.disp);
            self.b(op::I32_WRAP_I64);
            return;
        }
        if let Some(b) = m.base {
            self.gget(b as u32);
        } else {
            self.i64c(0);
        }
        if let Some(i) = m.index {
            self.gget(i as u32);
            self.i64c(m.scale as i64);
            self.b(op::I64_MUL);
            self.b(op::I64_ADD);
        }
        if m.disp != 0 {
            self.i64c(m.disp);
            self.b(op::I64_ADD);
        }
        self.b(op::I32_WRAP_I64);
    }
    /// Push the 64-bit value of an operand (reg global, immediate,
    /// or memory load). XMM operands are not scalar — they are handled
    /// by the SIMD paths and trap here if reached.
    fn push_operand(&mut self, o: &Operand) {
        match o {
            Operand::Reg(r) => self.gget(*r as u32),
            Operand::Imm(v) => self.i64c(*v),
            Operand::Mem(m) => {
                self.addr_i32(m);
                self.b(op::I64_LOAD);
                self.u(0);
                self.u(0);
            }
            Operand::Xmm(_) => {
                self.b(op::UNREACHABLE);
            }
        }
    }
    /// Store the i64 in local `src` into `dst` (register global or
    /// memory operand). For memory, the address is recomputed — safe
    /// because no architectural register is written before this point
    /// in a single instruction.
    fn store_from_local(&mut self, dst: &Operand, src: u32) {
        match dst {
            Operand::Reg(r) => {
                self.lget(src);
                self.gset(*r as u32);
            }
            Operand::Mem(m) => {
                self.addr_i32(m);
                self.lget(src);
                self.b(op::I64_STORE);
                self.u(0);
                self.u(0);
            }
            Operand::Imm(_) | Operand::Xmm(_) => {
                self.b(op::UNREACHABLE);
            }
        }
    }
}

impl JitCompiler {
    /// New compiler.
    pub fn new() -> Self {
        Self {
            scheduler: Scheduler::new(),
            allocator: Allocator::new(N_LOCALS as usize),
            blocks: Vec::new(),
            block_map: HashMap::new(),
        }
    }

    /// Compile x86-64 code bytes into a WASM module + block table.
    ///
    /// Returns `(module_bytes, blocks, block_map)`.
    pub fn compile(&mut self, code: &[u8]) -> JitResult<CompiledModule> {
        let instrs = decode_all(code)?;
        if instrs.is_empty() {
            return Err(JitError::ParseError("empty code".into()));
        }
        // Boundaries: instr offsets that start a block = 0, every
        // branch target, and every instruction following a control
        // instruction.
        let mut starts: std::collections::BTreeSet<usize> = [0usize].into_iter().collect();
        for (i, ins) in instrs.iter().enumerate() {
            match ins.kind {
                InstrKind::Jmp { target }
                | InstrKind::Jcc { target, .. }
                | InstrKind::Call { target } => {
                    starts.insert(target);
                    if i + 1 < instrs.len() {
                        starts.insert(instrs[i + 1].offset);
                    }
                }
                InstrKind::Ret if i + 1 < instrs.len() => {
                    starts.insert(instrs[i + 1].offset);
                }
                _ => {}
            }
        }
        // Group instructions into blocks.
        let mut blocks_x86: Vec<Vec<X86Instr>> = Vec::new();
        let mut cur: Vec<X86Instr> = Vec::new();
        let mut cur_start = 0usize;
        for ins in &instrs {
            if starts.contains(&ins.offset) && !cur.is_empty() {
                blocks_x86.push(std::mem::take(&mut cur));
            }
            if cur.is_empty() {
                cur_start = ins.offset;
            }
            cur.push(ins.clone());
            let _ = cur_start;
            let ends = matches!(
                ins.kind,
                InstrKind::Jmp { .. }
                    | InstrKind::Jcc { .. }
                    | InstrKind::Ret
                    | InstrKind::Call { .. }
            );
            if ends {
                blocks_x86.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            blocks_x86.push(cur);
        }

        let code_len = code.len();

        // Validate branch targets land on instruction boundaries.
        let instr_offsets: std::collections::HashSet<usize> =
            instrs.iter().map(|i| i.offset).collect();
        for ins in &instrs {
            let t = match ins.kind {
                InstrKind::Jmp { target }
                | InstrKind::Jcc { target, .. }
                | InstrKind::Call { target } => Some(target),
                _ => None,
            };
            if let Some(t) = t {
                if t < code.len() && !instr_offsets.contains(&t) {
                    return Err(JitError::ParseError(format!(
                        "branch target {t:#x} is not an instruction boundary"
                    )));
                }
            }
        }
        // Build the WASM module.
        let mut module = WasmModule {
            types: vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            imports: vec![Import {
                module: "xholy".into(),
                name: "syscall".into(),
                ty: FuncType {
                    params: vec![],
                    results: vec![ValType::I64],
                },
            }],
            memories: vec![super::wasm_binary::MemoryDecl { min_pages: 1 }],
            globals: Vec::new(),
            exports: Vec::new(),
            funcs: Vec::new(),
            code: Vec::new(),
        };
        // Types: index 0 = block type, index 1 = syscall type.
        module.types.push(FuncType {
            params: vec![],
            results: vec![ValType::I64],
        });
        // Globals: 16×i64 GPRs + 6×i32 flags + 16×2 i64 XMM lanes.
        for _ in 0..N_GPRS {
            module.globals.push(Global {
                ty: ValType::I64,
                init: 0,
            });
        }
        for _ in 0..N_FLAGS {
            module.globals.push(Global {
                ty: ValType::I32,
                init: 0,
            });
        }
        for _ in 0..32 {
            module.globals.push(Global {
                ty: ValType::I64,
                init: 0,
            });
        }

        // Function index space: import(0) then blocks.
        self.blocks.clear();
        self.block_map.clear();
        let mut entries = Vec::new();
        for (bi, block) in blocks_x86.iter().enumerate() {
            let scheduled = self.scheduler.schedule(block);
            let func_idx = 1 + bi as u32; // after the import
            let body = self.emit_block(&scheduled, code_len)?;
            entries.push(CodeEntry {
                locals: vec![(N_LOCALS, ValType::I64)],
                expr: body,
            });
            module.funcs.push(0); // type 0: () -> i64
            self.block_map.insert(block[0].offset as u64, func_idx);
            self.blocks.push(CompiledBlock {
                offset: block[0].offset as u64,
                func_index: func_idx,
                instr_count: block.len(),
            });
        }
        module.code = entries;
        let wasm = module.to_bytes();
        Ok(CompiledModule {
            wasm,
            blocks: self.blocks.clone(),
            block_map: self.block_map.clone(),
        })
    }

    /// Emit WASM expr for one scheduled block; appends the
    /// `i64.const next_offset` return value.
    fn emit_block(&mut self, block: &[X86Instr], code_len: usize) -> JitResult<Vec<u8>> {
        let mut e = Emit::new();
        let mut i = 0;
        while i < block.len() {
            let ins = &block[i];
            self.emit_instr(&mut e, ins, code_len)?;
            i += 1;
        }
        // If the block didn't end in a control instruction, fall
        // through to the next sequential instruction (or halt at end).
        if let Some(last) = block.last() {
            let terminal = matches!(
                last.kind,
                InstrKind::Jmp { .. }
                    | InstrKind::Jcc { .. }
                    | InstrKind::Ret
                    | InstrKind::Call { .. }
            );
            if !terminal {
                // Find next instruction after this block — caller
                // passes blocks contiguously; next offset = offset of
                // the instruction following the block's last instr.
                // We don't have instrs here; use sentinel handled by
                // emit_instr for terminal kinds only. Non-terminal
                // blocks end at a boundary; the next block's offset
                // is resolved via a placeholder: emit -1 halt only
                // when this is truly the last block. To stay correct
                // we encode fallthrough as the next instruction's
                // offset, looked up by caller-provided map — but the
                // simplest correct approach: every non-terminal block
                // is followed by another block; we find its offset via
                // `next_offset_after`.
                if let Some(next) = self.next_offset_after(block, code_len) {
                    e.i64c(next as i64);
                } else {
                    e.i64c(-1);
                }
            }
        } else {
            e.i64c(-1);
        }
        Ok(e.buf)
    }

    /// Offset of the instruction right after `block`'s last instr,
    /// or None at end-of-code. Uses stored decoded offsets — we kept
    /// them implicitly via block boundaries; compute from last len.
    fn next_offset_after(&self, block: &[X86Instr], code_len: usize) -> Option<usize> {
        let last = block.last()?;
        let next = last.offset + last.len;
        if next >= code_len {
            None
        } else {
            Some(next)
        }
    }

    /// Resolve a guest target offset to the WASM return value
    /// (block's first offset, or -1 for end-of-code).
    fn target_value(&self, target: usize, code_len: usize) -> i64 {
        if target >= code_len {
            -1
        } else {
            target as i64
        }
    }

    /// Emit one instruction's WASM.
    fn emit_instr(&mut self, e: &mut Emit, ins: &X86Instr, code_len: usize) -> JitResult<()> {
        use InstrKind::*;
        match &ins.kind {
            Nop => {}
            Mov { dst, src } => {
                if let Operand::Mem(m) = dst {
                    // [addr, value] order for the store.
                    e.addr_i32(m);
                    e.push_operand(src);
                    e.b(op::I64_STORE);
                    e.u(0);
                    e.u(0);
                } else if matches!(dst, Operand::Xmm(_)) {
                    return Err(JitError::Unsupported("scalar mov to xmm".into()));
                } else {
                    e.push_operand(src);
                    if let Operand::Reg(dr) = dst {
                        e.gset(*dr as u32);
                    } else {
                        return Err(JitError::Unsupported("mov to imm".into()));
                    }
                }
            }
            Movzx { dst, src, bits } | Movsx { dst, src, bits } => {
                let Operand::Reg(dr) = dst else {
                    return Err(JitError::Unsupported("movzx/movsx to imm/mem".into()));
                };
                e.push_operand(src);
                match bits {
                    8 => {
                        if matches!(ins.kind, Movzx { .. }) {
                            e.i64c(0xFF);
                            e.b(op::I64_AND);
                        } else {
                            // sign-extend low byte: (x << 56) >>s 56
                            e.i64c(56);
                            e.b(op::I64_SHL);
                            e.i64c(56);
                            e.b(op::I64_SHR_S);
                        }
                    }
                    16 => {
                        if matches!(ins.kind, Movzx { .. }) {
                            e.i64c(0xFFFF);
                            e.b(op::I64_AND);
                        } else {
                            e.i64c(48);
                            e.b(op::I64_SHL);
                            e.i64c(48);
                            e.b(op::I64_SHR_S);
                        }
                    }
                    _ => {
                        if matches!(ins.kind, Movsx { .. }) {
                            e.b(op::I64_EXTEND32_S);
                        }
                    }
                }
                e.gset(*dr as u32);
            }
            Add { dst, src }
            | Sub { dst, src }
            | Xor { dst, src }
            | And { dst, src }
            | Or { dst, src }
            | Mul { dst, src } => {
                self.emit_alu(e, ins, dst, src)?;
            }
            Cmp { a, b } => {
                // cmp = sub without writeback; sets all four flags.
                self.emit_cmp(e, a, b, true)?;
            }
            Test { a, b } => {
                // test = and without writeback; ZF/SF, CF=OF=0.
                self.emit_test(e, a, b)?;
            }
            Inc(_) | Dec(_) | Neg(_) | Not(_) => {
                self.emit_unary(e, &ins.kind)?;
            }
            Push(r) => {
                // addr = rsp - 8; [addr] = operand; rsp = addr
                if matches!(r, Operand::Xmm(_)) {
                    return Err(JitError::Unsupported("push xmm".into()));
                }
                e.gget(G_RSP);
                e.i64c(-8);
                e.b(op::I64_ADD);
                e.b(op::I32_WRAP_I64); // addr i32 — stack: [addr]
                e.push_operand(r); // value i64 — stack: [addr, val]
                e.b(op::I64_STORE);
                e.u(0);
                e.u(0);
                // rsp -= 8
                e.gget(G_RSP);
                e.i64c(-8);
                e.b(op::I64_ADD);
                e.gset(G_RSP);
            }
            Pop(r) => {
                if matches!(r, Operand::Xmm(_)) {
                    return Err(JitError::Unsupported("pop xmm".into()));
                }
                self.emit_pop(e, r);
            }
            SimdMov { dst, src } => {
                self.emit_simd_mov(e, dst, src)?;
            }
            SimdMovd {
                xmm,
                gpr,
                to_xmm,
                bits,
            } => {
                self.emit_simd_movd(e, xmm, gpr, *to_xmm, *bits)?;
            }
            SimdAlu { op: sop, dst, src } => {
                self.emit_simd_alu(e, *sop, dst, src)?;
            }
            Jmp { target } => {
                e.i64c(self.target_value(*target, code_len));
            }
            Jcc { cond, target } => {
                self.emit_jcc(e, *cond, *target, ins, code_len)?;
            }
            Call { target } => {
                // Push return offset (offset after this instr) onto
                // the guest stack, then jump to target.
                let ret_off = (ins.offset + ins.len) as i64;
                e.gget(G_RSP);
                e.i64c(-8);
                e.b(op::I64_ADD);
                e.b(op::I32_WRAP_I64);
                e.i64c(ret_off);
                e.b(op::I64_STORE);
                e.u(0);
                e.u(0);
                e.gget(G_RSP);
                e.i64c(-8);
                e.b(op::I64_ADD);
                e.gset(G_RSP);
                e.i64c(self.target_value(*target, code_len));
            }
            Ret => {
                // next = pop() → return it as i64.
                e.gget(G_RSP);
                e.b(op::I32_WRAP_I64);
                e.b(op::I64_LOAD);
                e.u(0);
                e.u(0); // stack: [ret_offset]
                        // local.tee L_R keeps value for return while we bump rsp
                e.b(op::LOCAL_TEE);
                e.u(L_R);
                e.gget(G_RSP);
                e.i64c(8);
                e.b(op::I64_ADD);
                e.gset(G_RSP);
                e.lget(L_R); // return value
            }
            Syscall => {
                // call import 0 (xholy.syscall) → rax
                e.b(op::CALL);
                e.u(0);
                e.gset(0); // rax
            }
        }
        Ok(())
    }

    /// ALU op with flags (add/sub/xor/and/or/mul).
    /// `dst` may be a register or a memory operand (read-modify-write).
    fn emit_alu(
        &mut self,
        e: &mut Emit,
        ins: &X86Instr,
        dst: &Operand,
        src: &Operand,
    ) -> JitResult<()> {
        if matches!(dst, Operand::Imm(_) | Operand::Xmm(_)) {
            return Err(JitError::Unsupported("alu dst imm/xmm".into()));
        }
        let opcode = match ins.kind {
            InstrKind::Add { .. } => op::I64_ADD,
            InstrKind::Sub { .. } => op::I64_SUB,
            InstrKind::Xor { .. } => op::I64_XOR,
            InstrKind::And { .. } => op::I64_AND,
            InstrKind::Or { .. } => op::I64_OR,
            InstrKind::Mul { .. } => op::I64_MUL,
            _ => unreachable!(),
        };
        // a → L_A, b → L_B, res → L_R
        e.push_operand(dst);
        e.b(op::LOCAL_SET);
        e.u(L_A);
        e.push_operand(src);
        e.b(op::LOCAL_SET);
        e.u(L_B);
        e.lget(L_A);
        e.lget(L_B);
        e.b(opcode);
        e.b(op::LOCAL_TEE);
        e.u(L_R);
        e.store_from_local(dst, L_R);
        // ZF = (res == 0); SF = res < 0; PF = even parity of low byte.
        self.emit_szp(e);
        // CF/OF/AF per operation.
        match ins.kind {
            InstrKind::Sub { .. } => {
                // CF = a <u b ; OF = ((a^b) & (a^res)) >> 63
                e.lget(L_A);
                e.lget(L_B);
                e.b(op::I64_LT_U);
                e.gset(G_CF);
                e.lget(L_A);
                e.lget(L_B);
                e.b(op::I64_XOR);
                e.lget(L_A);
                e.lget(L_R);
                e.b(op::I64_XOR);
                e.b(op::I64_AND);
                e.i64c(63);
                e.b(op::I64_SHR_S);
                e.b(op::I32_WRAP_I64);
                e.gset(G_OF);
                self.emit_af(e);
            }
            InstrKind::Add { .. } => {
                // CF = res <u a (unsigned wrap);
                // OF = ((a^res) & (b^res)) >> 63 for addition.
                e.lget(L_R);
                e.lget(L_A);
                e.b(op::I64_LT_U);
                e.gset(G_CF);
                e.lget(L_A);
                e.lget(L_R);
                e.b(op::I64_XOR);
                e.lget(L_B);
                e.lget(L_R);
                e.b(op::I64_XOR);
                e.b(op::I64_AND);
                e.i64c(63);
                e.b(op::I64_SHR_S);
                e.b(op::I32_WRAP_I64);
                e.gset(G_OF);
                self.emit_af(e);
            }
            _ => {
                // Logic ops and mul: CF/OF/AF cleared (mul's flags are
                // architecturally undefined — we define them as 0).
                e.i32c(0);
                e.gset(G_CF);
                e.i32c(0);
                e.gset(G_OF);
                e.i32c(0);
                e.gset(G_AF);
            }
        }
        Ok(())
    }

    /// ZF/SF/PF from L_R (result local).
    fn emit_szp(&mut self, e: &mut Emit) {
        e.lget(L_R);
        e.b(op::I64_EQZ);
        e.gset(G_ZF);
        e.lget(L_R);
        e.i64c(63);
        e.b(op::I64_SHR_S);
        e.b(op::I32_WRAP_I64);
        e.gset(G_SF);
        self.emit_pf(e);
    }

    /// PF = even parity of the low byte of L_R.\n    /// Nibble-fold then lookup in 0x9669: bit i = 1 iff nibble i has
    /// *even* parity (the complement of the classic 0x6996 odd table).
    fn emit_pf(&mut self, e: &mut Emit) {
        // t = res & 0xFF
        e.lget(L_R);
        e.i64c(0xFF);
        e.b(op::I64_AND);
        e.b(op::LOCAL_SET);
        e.u(L_T);
        // t ^= t >> 4  (fold nibbles)
        e.lget(L_T);
        e.lget(L_T);
        e.i64c(4);
        e.b(op::I64_SHR_U);
        e.b(op::I64_XOR);
        e.b(op::LOCAL_SET);
        e.u(L_T);
        // t &= 0xF
        e.lget(L_T);
        e.i64c(0xF);
        e.b(op::I64_AND);
        e.b(op::LOCAL_SET);
        e.u(L_T);
        // pf = (0x9669 >> t) & 1
        e.i64c(0x9669);
        e.lget(L_T);
        e.b(op::I64_SHR_U);
        e.i64c(1);
        e.b(op::I64_AND);
        e.b(op::I32_WRAP_I64);
        e.gset(G_PF);
    }

    /// AF = bit 4 of (a ^ b ^ res) — standard add/sub aux-carry.
    fn emit_af(&mut self, e: &mut Emit) {
        e.lget(L_A);
        e.lget(L_B);
        e.b(op::I64_XOR);
        e.lget(L_R);
        e.b(op::I64_XOR);
        e.i64c(4);
        e.b(op::I64_SHR_U);
        e.i64c(1);
        e.b(op::I64_AND);
        e.b(op::I32_WRAP_I64);
        e.gset(G_AF);
    }

    /// cmp a, b → flags for a - b (no writeback).
    fn emit_cmp(&mut self, e: &mut Emit, a: &Operand, b: &Operand, _sub: bool) -> JitResult<()> {
        if matches!(a, Operand::Imm(_) | Operand::Xmm(_)) {
            return Err(JitError::Unsupported("cmp imm/xmm lhs".into()));
        }
        e.push_operand(a);
        e.b(op::LOCAL_SET);
        e.u(L_A);
        e.push_operand(b);
        e.b(op::LOCAL_SET);
        e.u(L_B);
        e.lget(L_A);
        e.lget(L_B);
        e.b(op::I64_SUB);
        e.b(op::LOCAL_SET);
        e.u(L_R);
        // ZF/SF/PF
        self.emit_szp(e);
        // CF = a <u b ; OF = ((a^b)&(a^res))>>63 ; AF
        e.lget(L_A);
        e.lget(L_B);
        e.b(op::I64_LT_U);
        e.gset(G_CF);
        e.lget(L_A);
        e.lget(L_B);
        e.b(op::I64_XOR);
        e.lget(L_A);
        e.lget(L_R);
        e.b(op::I64_XOR);
        e.b(op::I64_AND);
        e.i64c(63);
        e.b(op::I64_SHR_S);
        e.b(op::I32_WRAP_I64);
        e.gset(G_OF);
        self.emit_af(e);
        Ok(())
    }

    /// test a, b → ZF/SF/PF from a&b; CF=OF=AF=0.
    fn emit_test(&mut self, e: &mut Emit, a: &Operand, b: &Operand) -> JitResult<()> {
        e.push_operand(a);
        e.b(op::LOCAL_SET);
        e.u(L_A);
        e.push_operand(b);
        e.b(op::LOCAL_SET);
        e.u(L_B);
        e.lget(L_A);
        e.lget(L_B);
        e.b(op::I64_AND);
        e.b(op::LOCAL_SET);
        e.u(L_R);
        self.emit_szp(e);
        e.i32c(0);
        e.gset(G_CF);
        e.i32c(0);
        e.gset(G_OF);
        e.i32c(0);
        e.gset(G_AF);
        Ok(())
    }

    /// inc/dec/neg/not on a register or memory operand.
    /// inc/dec/neg update ZF/SF/PF/AF/OF (CF preserved for inc/dec);
    /// not touches no flags.
    fn emit_unary(&mut self, e: &mut Emit, kind: &InstrKind) -> JitResult<()> {
        let (r, is_dec, is_neg, is_not) = match kind {
            InstrKind::Inc(r) => (r, false, false, false),
            InstrKind::Dec(r) => (r, true, false, false),
            InstrKind::Neg(r) => (r, false, true, false),
            InstrKind::Not(r) => (r, false, false, true),
            _ => unreachable!(),
        };
        if matches!(r, Operand::Imm(_) | Operand::Xmm(_)) {
            return Err(JitError::Unsupported("unary on imm/xmm".into()));
        }
        // old → L_A
        e.push_operand(r);
        e.b(op::LOCAL_SET);
        e.u(L_A);
        let mut res_in_lr = false;
        match (is_dec, is_neg, is_not) {
            (false, false, false) => {
                // inc: res = a + 1
                e.lget(L_A);
                e.i64c(1);
                e.b(op::I64_ADD);
                e.b(op::LOCAL_TEE);
                e.u(L_R);
                res_in_lr = true;
            }
            (true, false, false) => {
                // dec: res = a - 1
                e.lget(L_A);
                e.i64c(1);
                e.b(op::I64_SUB);
                e.b(op::LOCAL_TEE);
                e.u(L_R);
                res_in_lr = true;
            }
            (false, true, false) => {
                // neg: res = 0 - a
                e.i64c(0);
                e.lget(L_A);
                e.b(op::I64_SUB);
                e.b(op::LOCAL_TEE);
                e.u(L_R);
                res_in_lr = true;
            }
            (false, false, true) => {
                // not: res = a ^ -1
                e.lget(L_A);
                e.i64c(-1);
                e.b(op::I64_XOR);
                e.b(op::LOCAL_TEE);
                e.u(L_R);
            }
            _ => unreachable!(),
        }
        e.store_from_local(r, L_R);
        if is_not {
            return Ok(()); // not affects no flags
        }
        // ZF/SF/PF from result.
        self.emit_szp(e);
        if is_neg {
            // neg sets all flags: CF = a != 0; OF = a == i64::MIN;
            // AF ≈ ((a ^ res) >> 4) & 1.
            e.lget(L_A);
            e.b(op::I64_EQZ);
            e.b(op::I32_EQZ); // cf = !eqz(a) = a!=0
            e.gset(G_CF);
            e.lget(L_A);
            e.i64c(i64::MIN);
            e.b(op::I64_EQ);
            e.gset(G_OF);
            // af = ((a ^ res) >> 4) & 1 — reuse emit_af with b=0:
            // (a ^ 0 ^ res) == (a ^ res); emit_af uses L_B, so set
            // L_B = 0 first.
            e.i64c(0);
            e.b(op::LOCAL_SET);
            e.u(L_B);
            self.emit_af(e);
        } else {
            // inc/dec: OF = boundary crossing; AF via (a^1^res)>>4&1
            // approximation; CF preserved (not written).
            let boundary = if is_dec { i64::MIN } else { i64::MAX };
            e.lget(L_A);
            e.i64c(boundary);
            e.b(op::I64_EQ);
            e.gset(G_OF);
            // b = +1 for inc, -1 for dec in the aux formula.
            e.i64c(if is_dec { -1 } else { 1 });
            e.b(op::LOCAL_SET);
            e.u(L_B);
            self.emit_af(e);
        }
        let _ = res_in_lr;
        Ok(())
    }

    /// jcc: push cond(i32) then two i64 targets + select.
    fn emit_jcc(
        &mut self,
        e: &mut Emit,
        cond: Cond,
        target: usize,
        ins: &X86Instr,
        code_len: usize,
    ) -> JitResult<()> {
        let t = self.target_value(target, code_len);
        // jcc is terminal: both targets must be supplied here —
        // `target` on true, fallthrough (next instr) on false.
        let fall = self.target_value(ins.offset + ins.len, code_len);
        // Stack order for select: [val_true, val_false, cond]
        e.i64c(t);
        e.i64c(fall);
        self.emit_cond(e, cond);
        e.b(op::SELECT);
        Ok(())
    }

    /// Emit an i32 condition expression for `cond`.
    fn emit_cond(&mut self, e: &mut Emit, cond: Cond) {
        // Helpers as closures can't borrow e mutably twice; write raw.
        match cond {
            Cond::O => {
                e.gget(G_OF);
            }
            Cond::No => {
                e.gget(G_OF);
                e.b(op::I32_EQZ);
            }
            Cond::B => {
                e.gget(G_CF);
            }
            Cond::Ae => {
                e.gget(G_CF);
                e.b(op::I32_EQZ);
            }
            Cond::E => {
                e.gget(G_ZF);
            }
            Cond::Ne => {
                e.gget(G_ZF);
                e.b(op::I32_EQZ);
            }
            Cond::Be => {
                // cf | zf
                e.gget(G_CF);
                e.gget(G_ZF);
                e.b(op::I32_OR);
            }
            Cond::A => {
                // !cf & !zf
                e.gget(G_CF);
                e.b(op::I32_EQZ);
                e.gget(G_ZF);
                e.b(op::I32_EQZ);
                e.b(op::I32_AND);
            }
            Cond::S => {
                e.gget(G_SF);
            }
            Cond::Ns => {
                e.gget(G_SF);
                e.b(op::I32_EQZ);
            }
            Cond::P => {
                e.gget(G_PF);
            }
            Cond::Np => {
                e.gget(G_PF);
                e.b(op::I32_EQZ);
            }
            Cond::L => {
                // sf != of
                e.gget(G_SF);
                e.gget(G_OF);
                e.b(op::I32_XOR);
            }
            Cond::Ge => {
                // sf == of
                e.gget(G_SF);
                e.gget(G_OF);
                e.b(op::I32_EQ);
            }
            Cond::Le => {
                // zf | (sf != of)
                e.gget(G_ZF);
                e.gget(G_SF);
                e.gget(G_OF);
                e.b(op::I32_XOR);
                e.b(op::I32_OR);
            }
            Cond::G => {
                // !zf & (sf == of)
                e.gget(G_ZF);
                e.b(op::I32_EQZ);
                e.gget(G_SF);
                e.gget(G_OF);
                e.b(op::I32_EQ);
                e.b(op::I32_AND);
            }
        }
    }

    /// pop operand — val=[rsp]; rsp+=8; dst=val.
    /// Register and memory destinations are supported.
    fn emit_pop(&mut self, e: &mut Emit, dst: &Operand) {
        e.gget(G_RSP);
        e.b(op::I32_WRAP_I64);
        e.b(op::I64_LOAD);
        e.u(0);
        e.u(0); // [val]
        e.b(op::LOCAL_TEE);
        e.u(L_R); // keep val in local
        e.b(op::DROP); // consume the stack copy
                       // bump rsp
        e.gget(G_RSP);
        e.i64c(8);
        e.b(op::I64_ADD);
        e.gset(G_RSP);
        // store popped value into dst (reg global or memory).
        match dst {
            Operand::Reg(g) => {
                e.lget(L_R);
                e.gset(*g as u32);
            }
            Operand::Mem(m) => {
                e.addr_i32(m);
                e.lget(L_R);
                e.b(op::I64_STORE);
                e.u(0);
                e.u(0);
            }
            _ => {
                e.b(op::UNREACHABLE);
            }
        }
    }

    // ------------------------------------------------------------------
    // SIMD (SSE2 integer subset): xmm = two i64 lane globals.
    // ------------------------------------------------------------------

    /// Global index of xmm `n`'s low lane.
    fn xmm_lo(n: u8) -> u32 {
        G_XMM0 + 2 * n as u32
    }

    /// Push one 64-bit lane of an xmm/memory source onto the stack.
    /// `lane` is 0 (low) or 1 (high); memory reads at addr+8*lane.
    fn push_xmm_lane(&mut self, e: &mut Emit, src: &Operand, lane: u32) {
        match src {
            Operand::Xmm(n) => {
                e.gget(Self::xmm_lo(*n) + lane);
            }
            Operand::Mem(m) => {
                e.addr_i32(m);
                if lane != 0 {
                    e.i64c(8);
                    e.b(op::I64_ADD);
                    e.b(op::I32_WRAP_I64);
                }
                e.b(op::I64_LOAD);
                e.u(0);
                e.u(0);
            }
            _ => {
                e.b(op::UNREACHABLE);
            }
        }
    }

    /// Store local `val` into one lane of an xmm/memory destination.
    fn store_xmm_lane(&mut self, e: &mut Emit, dst: &Operand, lane: u32, val: u32) {
        match dst {
            Operand::Xmm(n) => {
                e.lget(val);
                e.gset(Self::xmm_lo(*n) + lane);
            }
            Operand::Mem(m) => {
                e.addr_i32(m);
                if lane != 0 {
                    e.i64c(8);
                    e.b(op::I64_ADD);
                    e.b(op::I32_WRAP_I64);
                }
                e.lget(val);
                e.b(op::I64_STORE);
                e.u(0);
                e.u(0);
            }
            _ => {
                e.b(op::UNREACHABLE);
            }
        }
    }

    /// movdqa/movdqu: 128-bit copy xmm↔xmm or xmm↔m128.
    fn emit_simd_mov(&mut self, e: &mut Emit, dst: &Operand, src: &Operand) -> JitResult<()> {
        for lane in 0..2 {
            // value → L_R, then store
            self.push_xmm_lane(e, src, lane);
            e.b(op::LOCAL_SET);
            e.u(L_R);
            self.store_xmm_lane(e, dst, lane, L_R);
        }
        Ok(())
    }

    /// movd/movq between xmm low lane and a gpr/mem operand.
    /// `to_xmm`: gpr → xmm (zeroes high lane); else xmm → gpr.
    fn emit_simd_movd(
        &mut self,
        e: &mut Emit,
        xmm: &Operand,
        gpr: &Operand,
        to_xmm: bool,
        bits: u8,
    ) -> JitResult<()> {
        let Operand::Xmm(xn) = xmm else {
            return Err(JitError::Unsupported("movd xmm side not xmm".into()));
        };
        let mask = if bits == 64 { -1i64 } else { 0xFFFF_FFFF };
        if to_xmm {
            // gpr → xmm.lo = gpr & mask ; xmm.hi = 0
            e.push_operand(gpr);
            e.i64c(mask);
            e.b(op::I64_AND);
            e.b(op::LOCAL_SET);
            e.u(L_R);
            e.lget(L_R);
            e.gset(Self::xmm_lo(*xn));
            e.i64c(0);
            e.gset(Self::xmm_lo(*xn) + 1);
        } else {
            // xmm.lo & mask → gpr/mem
            e.gget(Self::xmm_lo(*xn));
            e.i64c(mask);
            e.b(op::I64_AND);
            e.b(op::LOCAL_SET);
            e.u(L_R);
            match gpr {
                Operand::Reg(g) => {
                    e.lget(L_R);
                    e.gset(*g as u32);
                }
                Operand::Mem(m) => {
                    e.addr_i32(m);
                    e.lget(L_R);
                    e.b(op::I64_STORE);
                    e.u(0);
                    e.u(0);
                }
                _ => {
                    return Err(JitError::Unsupported("movd gpr side imm/xmm".into()));
                }
            }
        }
        Ok(())
    }

    /// Lane-wise i64 logic/arithmetic: dst op= src (xmm or m128).
    fn emit_simd_alu(
        &mut self,
        e: &mut Emit,
        sop: SimdOp,
        dst: &Operand,
        src: &Operand,
    ) -> JitResult<()> {
        let wop = match sop {
            SimdOp::Paddq => op::I64_ADD,
            SimdOp::Psubq => op::I64_SUB,
            SimdOp::Pand => op::I64_AND,
            SimdOp::Por => op::I64_OR,
            SimdOp::Pxor => op::I64_XOR,
        };
        for lane in 0..2 {
            // dst_val → L_A, src_val → L_B, res → L_R → store
            self.push_xmm_lane(e, dst, lane);
            e.b(op::LOCAL_SET);
            e.u(L_A);
            self.push_xmm_lane(e, src, lane);
            e.b(op::LOCAL_SET);
            e.u(L_B);
            e.lget(L_A);
            e.lget(L_B);
            e.b(wop);
            e.b(op::LOCAL_SET);
            e.u(L_R);
            self.store_xmm_lane(e, dst, lane, L_R);
        }
        Ok(())
    }
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_mov_add_ret() {
        // mov rax,1 ; add rax,2 ; ret  (48 C7 C0 01 00 00 00 / 48 83 C0 02 / C3)
        let code = [
            0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, 0x48, 0x83, 0xC0, 0x02, 0xC3,
        ];
        let mut c = JitCompiler::new();
        let m = c.compile(&code).unwrap();
        assert_eq!(&m.wasm[0..4], &[0, 0x61, 0x73, 0x6D]);
        assert!(!m.blocks.is_empty());
        assert!(m.block_map.contains_key(&0));
    }

    #[test]
    fn compiles_branch() {
        // cmp rax,rbx ; je +2 ; nop ; ret-ish — build minimal:
        // 48 39 D8 (cmp rax,rbx) 74 02 (je +2) 90 (nop) C3 (ret)
        let code = [0x48, 0x39, 0xD8, 0x74, 0x02, 0x90, 0xC3];
        let mut c = JitCompiler::new();
        let m = c.compile(&code).unwrap();
        assert!(m.blocks.len() >= 2, "branch should split blocks");
    }

    #[test]
    fn rejects_empty() {
        let mut c = JitCompiler::new();
        assert!(c.compile(&[]).is_err());
    }
}
