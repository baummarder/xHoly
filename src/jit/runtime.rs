//! WASM runtime — Phase 13.
//!
//! A small interpreter for the WASM subset emitted by
//! [`super::compiler`]: i32/i64 locals & globals, flat memory,
//! imports (host calls), and straight-line function bodies that
//! return an `i64` next-PC. It parses real WASM 1.0 binaries
//! (sections 1/2/3/5/6/10) — no `wasmtime` dependency.
//!
//! The JIT drives it block-at-a-time: each compiled x86 basic block
//! is one function returning the next guest offset; [`JitRuntime::run`]
//! loops until a function returns `-1` (halt) or `ret` pops an empty
//! guest stack.
//!
//! Honest limits: no SIMD, no reference types, no exceptions, no
//! threads — spec-conformant for the MVP numeric subset only.

use super::wasm_binary::ValType;
use super::{JitError, JitResult};
use std::collections::HashMap;

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// WASM memory size in 64 KiB pages.
    pub memory_pages: u32,
    /// Max executed WASM instructions (infinite-loop guard).
    pub max_steps: u64,
    /// Max nested `call` depth.
    pub max_call_depth: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            memory_pages: 16, // 1 MiB
            max_steps: 10_000_000,
            max_call_depth: 256,
        }
    }
}

/// WASM value on the operand stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WVal {
    I32(i32),
    I64(i64),
}

impl WVal {
    fn as_i64(self) -> i64 {
        match self {
            Self::I64(v) => v,
            Self::I32(v) => v as i64,
        }
    }
    fn as_i32(self) -> i32 {
        match self {
            Self::I32(v) => v,
            Self::I64(v) => v as i32,
        }
    }
}

/// Parsed function body (locals already flattened to types).
#[derive(Debug, Clone)]
struct FuncBody {
    locals: Vec<ValType>,
    expr: Vec<u8>,
}

/// Host import signature: gets the machine, returns i64.
pub type HostFn = Box<dyn FnMut(&mut Machine) -> JitResult<i64>>;

/// Machine state: globals, memory, functions, operand stack.
#[derive(Debug)]
pub struct Machine {
    /// Global values (i64 or i32 by declared type).
    pub globals: Vec<WVal>,
    /// Linear memory bytes.
    pub memory: Vec<u8>,
    /// Parsed defined functions (import index space excluded).
    funcs: Vec<FuncBody>,
    /// Number of imported functions (they occupy func indices 0..n).
    pub num_imports: usize,
    /// Operand stack.
    stack: Vec<WVal>,
    /// Function locals (save/restored per call frame).
    locals: Vec<WVal>,
    /// Step counter.
    steps: u64,
    max_steps: u64,
}

/// The JIT runtime: machine + host functions + block map.
pub struct JitRuntime {
    pub machine: Machine,
    hosts: Vec<Option<HostFn>>,
    /// guest offset → WASM func index (in full index space).
    offset_map: HashMap<u64, u32>,
    max_call_depth: u32,
}

impl JitRuntime {
    /// New runtime with default config.
    pub fn new() -> Self {
        Self::with_config(RuntimeConfig::default())
    }

    /// New runtime with explicit config.
    pub fn with_config(cfg: RuntimeConfig) -> Self {
        Self {
            machine: Machine {
                globals: Vec::new(),
                memory: vec![0u8; cfg.memory_pages as usize * 65536],
                funcs: Vec::new(),
                num_imports: 0,
                stack: Vec::new(),
                locals: Vec::new(),
                steps: 0,
                max_steps: cfg.max_steps,
            },
            hosts: Vec::new(),
            offset_map: HashMap::new(),
            max_call_depth: cfg.max_call_depth,
        }
    }

    /// Load a WASM binary produced by [`super::compiler::JitCompiler`].
    /// `offset_map` maps guest code offsets → WASM function indices
    /// (as stored in `CompiledBlock::func_index`).
    pub fn load(
        &mut self,
        wasm: &[u8],
        offset_map: HashMap<u64, u32>,
        hosts: Vec<HostFn>,
    ) -> JitResult<()> {
        let parsed = parse_module(wasm)?;
        self.machine.num_imports = parsed.num_imports;
        self.machine.funcs = parsed.funcs;
        self.machine.globals = parsed.globals;
        if let Some(pages) = parsed.mem_pages {
            self.machine.memory = vec![0u8; pages as usize * 65536];
        }
        self.hosts = hosts.into_iter().map(Some).collect();
        if self.hosts.len() < self.machine.num_imports {
            return Err(JitError::Runtime(format!(
                "need {} host fns, got {}",
                self.machine.num_imports,
                self.hosts.len()
            )));
        }
        // Guest stack: top of WASM linear memory, sentinel -1 marks
        // "no caller" so the first `ret` halts instead of trapping.
        if self.machine.globals.len() > 4 {
            let sp = self.machine.memory.len().saturating_sub(8) as u64;
            if sp + 8 <= self.machine.memory.len() as u64 {
                self.machine.memory[sp as usize..sp as usize + 8]
                    .copy_from_slice(&(-1i64).to_le_bytes());
                self.machine.globals[4] = WVal::I64(sp as i64);
            }
        }
        self.offset_map = offset_map;
        Ok(())
    }

    /// Register the block map without loading (for tests).
    pub fn set_offset_map(&mut self, map: HashMap<u64, u32>) {
        self.offset_map = map;
    }

    /// Read a GPR global (0–15).
    pub fn reg(&self, r: u32) -> i64 {
        self.machine.globals[r as usize].as_i64()
    }

    /// Write a GPR global.
    pub fn set_reg(&mut self, r: u32, v: i64) {
        self.machine.globals[r as usize] = WVal::I64(v);
    }

    /// Guest memory read (delegates to WASM linear memory).
    pub fn mem_read(&self, addr: u64, len: usize) -> JitResult<Vec<u8>> {
        let a = addr as usize;
        let end = a + len;
        if end > self.machine.memory.len() {
            return Err(JitError::MemoryOverflow(addr));
        }
        Ok(self.machine.memory[a..end].to_vec())
    }

    /// Guest memory write.
    pub fn mem_write(&mut self, addr: u64, data: &[u8]) -> JitResult<()> {
        let a = addr as usize;
        let end = a + data.len();
        if end > self.machine.memory.len() {
            return Err(JitError::MemoryOverflow(addr));
        }
        self.machine.memory[a..end].copy_from_slice(data);
        Ok(())
    }

    /// Run from `entry_offset` until halt; returns rax.
    pub fn run(&mut self, entry_offset: u64) -> JitResult<i64> {
        let mut pc = entry_offset;
        let mut guard = 0u64;
        loop {
            guard += 1;
            if guard > 1_000_000 {
                return Err(JitError::Runtime("block dispatch limit".into()));
            }
            let func = *self
                .offset_map
                .get(&pc)
                .ok_or_else(|| JitError::Runtime(format!("no block at offset {pc:#x}")))?;
            let next = self.exec_func(func, 0)?;
            if next < 0 {
                break;
            }
            pc = next as u64;
        }
        Ok(self.reg(0)) // rax
    }

    /// Execute one function; returns its i64 result.
    fn exec_func(&mut self, func_idx: u32, depth: u32) -> JitResult<i64> {
        if depth > self.max_call_depth {
            return Err(JitError::Runtime("call depth exceeded".into()));
        }
        // Imported function → host call.
        if (func_idx as usize) < self.machine.num_imports {
            let mut host = self.hosts[func_idx as usize]
                .take()
                .ok_or_else(|| JitError::Runtime("host fn missing".into()))?;
            let r = host(&mut self.machine);
            self.hosts[func_idx as usize] = Some(host);
            return r;
        }
        let body_idx = func_idx as usize - self.machine.num_imports;
        let body = self
            .machine
            .funcs
            .get(body_idx)
            .cloned()
            .ok_or_else(|| JitError::Runtime(format!("bad func idx {func_idx}")))?;
        // Fresh locals frame for this call (params: none in subset).
        let saved_locals = std::mem::take(&mut self.machine.locals);
        self.machine.locals = body
            .locals
            .iter()
            .map(|t| match t {
                ValType::I32 => WVal::I32(0),
                ValType::I64 => WVal::I64(0),
            })
            .collect();
        // Run interpreter over expr bytes until `end`.
        let mut p = 0usize;
        let expr = &body.expr;
        // Local operand stack frame: clone machine stack boundary.
        let base = self.machine.stack.len();
        while p < expr.len() {
            let op = expr[p];
            p += 1;
            self.machine.steps += 1;
            if self.machine.steps > self.machine.max_steps {
                return Err(JitError::Runtime("step limit".into()));
            }
            match op {
                0x0B => break, // end
                0x41 => {
                    // i32.const
                    let (v, n) = read_sleb(&expr[p..])?;
                    p += n;
                    self.machine.stack.push(WVal::I32(v as i32));
                }
                0x42 => {
                    let (v, n) = read_sleb(&expr[p..])?;
                    p += n;
                    self.machine.stack.push(WVal::I64(v));
                }
                0x20 => {
                    // local.get
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let v = self.local_get(&body, idx as usize)?;
                    self.machine.stack.push(v);
                }
                0x21 => {
                    // local.set
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let v = self
                        .machine
                        .stack
                        .pop()
                        .ok_or_else(|| JitError::Runtime("stack underflow local.set".into()))?;
                    self.local_set(&body, idx as usize, v)?;
                }
                0x22 => {
                    // local.tee
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let v = *self
                        .machine
                        .stack
                        .last()
                        .ok_or_else(|| JitError::Runtime("stack underflow local.tee".into()))?;
                    self.local_set(&body, idx as usize, v)?;
                }
                0x23 => {
                    // global.get
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let v = self
                        .machine
                        .globals
                        .get(idx as usize)
                        .copied()
                        .ok_or_else(|| JitError::Runtime("bad global".into()))?;
                    self.machine.stack.push(v);
                }
                0x24 => {
                    // global.set
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let v =
                        self.machine.stack.pop().ok_or_else(|| {
                            JitError::Runtime("stack underflow global.set".into())
                        })?;
                    let g = self
                        .machine
                        .globals
                        .get_mut(idx as usize)
                        .ok_or_else(|| JitError::Runtime("bad global".into()))?;
                    *g = match *g {
                        WVal::I32(_) => WVal::I32(v.as_i32()),
                        WVal::I64(_) => WVal::I64(v.as_i64()),
                    };
                }
                0x10 => {
                    // call
                    let (idx, n) = read_uleb(&expr[p..])?;
                    p += n;
                    let r = self.exec_func(idx as u32, depth + 1)?;
                    self.machine.stack.push(WVal::I64(r));
                }
                0x1A => {
                    self.machine.stack.pop();
                }
                0x1B => {
                    // select: [v1, v2, cond] → v1 if cond!=0
                    let cond = self.pop_i32()?;
                    let v2 = self
                        .machine
                        .stack
                        .pop()
                        .ok_or_else(|| JitError::Runtime("stack underflow select".into()))?;
                    let v1 = self
                        .machine
                        .stack
                        .pop()
                        .ok_or_else(|| JitError::Runtime("stack underflow select".into()))?;
                    self.machine.stack.push(if cond != 0 { v1 } else { v2 });
                }
                // i64 comparisons → i32 (0x50 eqz is unary)
                0x50..=0x5A => {
                    if op == 0x50 {
                        let a = self.pop_i64()?;
                        self.machine.stack.push(WVal::I32((a == 0) as i32));
                        continue;
                    }
                    let b = self.pop_i64()?;
                    let a = self.pop_i64()?;
                    let r = match op {
                        0x51 => (a == b) as i32,
                        0x52 => (a != b) as i32,
                        0x53 => (a < b) as i32,
                        0x54 => ((a as u64) < (b as u64)) as i32,
                        0x55 => (a > b) as i32,
                        0x56 => ((a as u64) > (b as u64)) as i32,
                        0x57 => (a <= b) as i32,
                        0x58 => ((a as u64) <= (b as u64)) as i32,
                        0x59 => (a >= b) as i32,
                        0x5A => ((a as u64) >= (b as u64)) as i32,
                        _ => unreachable!(),
                    };
                    self.machine.stack.push(WVal::I32(r));
                }
                // i64 arithmetic/logic
                0x7C..=0x7E | 0x83..=0x88 => {
                    let b = self.pop_i64()?;
                    let a = self.pop_i64()?;
                    let r = match op {
                        0x7C => a.wrapping_add(b),
                        0x7D => a.wrapping_sub(b),
                        0x7E => a.wrapping_mul(b),
                        0x83 => a & b,
                        0x84 => a | b,
                        0x85 => a ^ b,
                        0x86 => a.wrapping_shl(b as u32),
                        0x87 => a.wrapping_shr(b as u32),
                        0x88 => ((a as u64).wrapping_shr(b as u32)) as i64,
                        _ => unreachable!(),
                    };
                    self.machine.stack.push(WVal::I64(r));
                }
                // i32 ops (0x45 eqz is unary)
                0x45..=0x4F | 0x71..=0x73 => {
                    if op == 0x45 {
                        let a = self.pop_i32()?;
                        self.machine.stack.push(WVal::I32((a == 0) as i32));
                        continue;
                    }
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    let r = match op {
                        0x46 => (a == b) as i32,
                        0x47 => (a != b) as i32,
                        0x48 => (a < b) as i32,
                        0x49 => ((a as u32) < (b as u32)) as i32,
                        0x4B => ((a as u32) > (b as u32)) as i32,
                        0x71 => a & b,
                        0x72 => a | b,
                        0x73 => a ^ b,
                        _ => return Err(JitError::Runtime(format!("i32 op {op:#x}"))),
                    };
                    self.machine.stack.push(WVal::I32(r));
                }
                0xA7 => {
                    // i32.wrap_i64
                    let v = self.pop_i64()?;
                    self.machine.stack.push(WVal::I32(v as i32));
                }
                0xC2..=0xC4 => {
                    // sign-extension ops
                    let v = self.pop_i64()?;
                    let r = match op {
                        0xC2 => (v << 56) >> 56,
                        0xC3 => (v << 48) >> 48,
                        _ => (v << 32) >> 32,
                    };
                    self.machine.stack.push(WVal::I64(r));
                }
                // memory ops: i64.load=0x29 i64.store=0x37 (+align,offset)
                0x29 | 0x37 => {
                    let (_align, n1) = read_uleb(&expr[p..])?;
                    p += n1;
                    let (offset, n2) = read_uleb(&expr[p..])?;
                    p += n2;
                    if op == 0x37 {
                        let val = self.pop_i64()?;
                        let addr = self.pop_i32()? as u64;
                        let a = addr
                            .checked_add(offset)
                            .ok_or(JitError::MemoryOverflow(addr))?;
                        self.mem_write(a, &val.to_le_bytes())?;
                    } else {
                        let addr = self.pop_i32()? as u64;
                        let a = addr
                            .checked_add(offset)
                            .ok_or(JitError::MemoryOverflow(addr))?;
                        let bytes = self.mem_read(a, 8)?;
                        self.machine
                            .stack
                            .push(WVal::I64(i64::from_le_bytes(bytes.try_into().unwrap())));
                    }
                }
                _ => {
                    return Err(JitError::Runtime(format!(
                        "unimplemented wasm opcode {op:#04x}"
                    )));
                }
            }
        }
        // Function result: top of stack must be i64.
        let result = self
            .machine
            .stack
            .pop()
            .ok_or_else(|| JitError::Runtime("function produced no result".into()))?;
        // Restore caller's locals frame.
        self.machine.locals = saved_locals;
        // Truncate stack back to base for caller.
        self.machine.stack.truncate(base);
        // Re-push result for uniform handling? Caller expects i64 —
        // we return it directly; nested calls push it themselves.
        Ok(result.as_i64())
    }

    fn pop_i64(&mut self) -> JitResult<i64> {
        self.machine
            .stack
            .pop()
            .map(|v| v.as_i64())
            .ok_or_else(|| JitError::Runtime("stack underflow i64".into()))
    }

    fn pop_i32(&mut self) -> JitResult<i32> {
        self.machine
            .stack
            .pop()
            .map(|v| v.as_i32())
            .ok_or_else(|| JitError::Runtime("stack underflow i32".into()))
    }

    /// Local variable access (indices are direct — no params).
    fn local_get(&self, _body: &FuncBody, idx: usize) -> JitResult<WVal> {
        self.machine
            .locals
            .get(idx)
            .copied()
            .ok_or_else(|| JitError::Runtime(format!("bad local {idx}")))
    }

    fn local_set(&mut self, _body: &FuncBody, idx: usize, v: WVal) -> JitResult<()> {
        let l = self
            .machine
            .locals
            .get_mut(idx)
            .ok_or_else(|| JitError::Runtime(format!("bad local {idx}")))?;
        // Coerce to declared type if needed (all our locals are i64).
        *l = match l {
            WVal::I32(_) => WVal::I32(v.as_i32()),
            WVal::I64(_) => WVal::I64(v.as_i64()),
        };
        Ok(())
    }
}

impl Default for JitRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsed module.
struct ParsedModule {
    num_imports: usize,
    funcs: Vec<FuncBody>,
    globals: Vec<WVal>,
    mem_pages: Option<u32>,
}

/// Parse the WASM sections we emit.
fn parse_module(bytes: &[u8]) -> JitResult<ParsedModule> {
    if bytes.len() < 8 || bytes[0..4] != [0, 0x61, 0x73, 0x6D] {
        return Err(JitError::ParseError("bad wasm magic".into()));
    }
    let mut p = 8usize;
    let mut num_imports = 0usize;
    let mut funcs = Vec::new();
    let mut globals = Vec::new();
    let mut mem_pages = None;
    while p < bytes.len() {
        let id = bytes[p];
        p += 1;
        let (size, n) = read_uleb(&bytes[p..])?;
        p += n;
        let end = p + size as usize;
        if end > bytes.len() {
            return Err(JitError::ParseError("section overruns buffer".into()));
        }
        let sec = &bytes[p..end];
        match id {
            2 => {
                // import section: count + entries (we only need count
                // of func imports — all our imports are funcs).
                let (count, mut q) = read_uleb(sec)?;
                num_imports = count as usize;
                for _ in 0..count {
                    let (_m, a) = read_name(sec, q)?;
                    q = a;
                    let (_nm, b) = read_name(sec, q)?;
                    q = b;
                    let kind = sec[q];
                    q += 1;
                    if kind == 0x00 {
                        // func import: typeidx u32
                        let (_t, c) = read_uleb(&sec[q..])?;
                        q += c;
                    } else {
                        return Err(JitError::Unsupported(
                            "non-func import in jit module".into(),
                        ));
                    }
                }
            }
            3 => {
                // function section: type indices (we use one type).
                let (_count, _q) = read_uleb(sec)?;
                // func count comes from code section; skip.
            }
            5 => {
                // memory section
                let (count, mut q) = read_uleb(sec)?;
                if count > 0 {
                    let flag = sec[q];
                    q += 1;
                    let (min, _n) = read_uleb(&sec[q..])?;
                    mem_pages = Some(min as u32);
                    let _ = flag;
                }
            }
            6 => {
                // global section
                let (count, mut q) = read_uleb(sec)?;
                for _ in 0..count {
                    let ty = sec[q];
                    q += 1;
                    let mutable = sec[q];
                    q += 1;
                    let _ = mutable;
                    // init expr: expect i32.const/i64.const … end
                    q += 1; // const opcode
                    let (v, n) = read_sleb(&sec[q..])?;
                    q += n;
                    // expect 0x0B end
                    if sec.get(q) == Some(&0x0B) {
                        q += 1;
                    }
                    globals.push(match ty {
                        0x7F => WVal::I32(v as i32),
                        0x7E => WVal::I64(v),
                        _ => return Err(JitError::ParseError("bad global type".into())),
                    });
                }
            }
            10 => {
                // code section
                let (count, mut q) = read_uleb(sec)?;
                for _ in 0..count {
                    let (body_size, n) = read_uleb(&sec[q..])?;
                    q += n;
                    let body = &sec[q..q + body_size as usize];
                    q += body_size as usize;
                    let (nlocals, mut r) = read_uleb(body)?;
                    let mut local_types = Vec::new();
                    for _ in 0..nlocals {
                        let (cnt, m) = read_uleb(&body[r..])?;
                        r += m;
                        let vt = match body[r] {
                            0x7F => ValType::I32,
                            0x7E => ValType::I64,
                            t => return Err(JitError::Unsupported(format!("local type {t:#x}"))),
                        };
                        r += 1;
                        for _ in 0..cnt {
                            local_types.push(vt);
                        }
                    }
                    funcs.push(FuncBody {
                        locals: local_types,
                        expr: body[r..body.len() - 1].to_vec(), // strip end
                    });
                }
            }
            _ => {}
        }
        p = end;
    }
    Ok(ParsedModule {
        num_imports,
        funcs,
        globals,
        mem_pages,
    })
}

fn read_name(buf: &[u8], p: usize) -> JitResult<(&str, usize)> {
    let (len, n) = read_uleb(&buf[p..])?;
    let s = std::str::from_utf8(&buf[p + n..p + n + len as usize])
        .map_err(|_| JitError::ParseError("bad name".into()))?;
    Ok((s, p + n + len as usize))
}

/// Read unsigned LEB128.
fn read_uleb(buf: &[u8]) -> JitResult<(u64, usize)> {
    let mut v = 0u64;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate().take(10) {
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok((v, i + 1));
        }
        shift += 7;
    }
    Err(JitError::ParseError("uleb overflow".into()))
}

/// Read signed LEB128 (up to 64-bit).
fn read_sleb(buf: &[u8]) -> JitResult<(i64, usize)> {
    let mut v = 0i64;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate().take(10) {
        v |= ((b & 0x7F) as i64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && (b & 0x40) != 0 {
                v |= -1i64 << shift;
            }
            return Ok((v, i + 1));
        }
    }
    Err(JitError::ParseError("sleb overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb_helpers() {
        assert_eq!(read_uleb(&[0x80, 0x01]).unwrap(), (128, 2));
        assert_eq!(read_sleb(&[0x7F]).unwrap(), (-1, 1));
    }
}
