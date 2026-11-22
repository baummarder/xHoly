//! WASM 1.0 binary writer — Phase 13.
//!
//! Emits real, spec-shaped WASM binaries (magic + version + LEB128
//! sections) for the subset the JIT compiler produces: functions with
//! i32/i64 locals and globals, imports for host calls, and a flat
//! memory. The runtime in [`super::runtime`] interprets this exact
//! subset; the bytes are also consumable by any spec WASM engine.

/// WASM value types used by the JIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
}

impl ValType {
    /// Binary encoding.
    pub fn byte(self) -> u8 {
        match self {
            Self::I32 => 0x7F,
            Self::I64 => 0x7E,
        }
    }
}

/// Section ids (WASM spec).
mod sec {
    pub const TYPE: u8 = 1;
    pub const IMPORT: u8 = 2;
    pub const FUNCTION: u8 = 3;
    pub const MEMORY: u8 = 5;
    pub const GLOBAL: u8 = 6;
    pub const EXPORT: u8 = 7;
    pub const CODE: u8 = 10;
}

/// One function type: params → results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

/// An imported host function.
#[derive(Debug, Clone)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub ty: FuncType,
}

/// A global variable (mutable, i64/i32 init).
#[derive(Debug, Clone)]
pub struct Global {
    pub ty: ValType,
    pub init: i64,
}

/// A memory declaration (min pages).
#[derive(Debug, Clone, Copy)]
pub struct MemoryDecl {
    pub min_pages: u32,
}

/// One code entry: grouped locals + raw expression bytes.
#[derive(Debug, Clone)]
pub struct CodeEntry {
    /// (count, type) pairs — WASM locals are declared in runs.
    pub locals: Vec<(u32, ValType)>,
    /// Function body WITHOUT the trailing `end` opcode.
    pub expr: Vec<u8>,
}

/// A complete WASM module description.
#[derive(Debug, Default)]
pub struct WasmModule {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    /// Type index per defined function.
    pub funcs: Vec<u32>,
    pub memories: Vec<MemoryDecl>,
    pub globals: Vec<Global>,
    /// (name, kind, index) exports; kind 0 = func, 2 = memory, 3 = global.
    pub exports: Vec<(String, u8, u32)>,
    pub code: Vec<CodeEntry>,
}

/// Write an unsigned LEB128.
pub fn uleb(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        buf.push(b);
        if v == 0 {
            break;
        }
    }
}

/// Write a signed LEB128 (33/64-bit).
pub fn sleb(buf: &mut Vec<u8>, mut v: i64) {
    loop {
        let mut b = (v & 0x7F) as u8;
        v >>= 7;
        let done = (v == 0 && (b & 0x40) == 0) || (v == -1 && (b & 0x40) != 0);
        if !done {
            b |= 0x80;
        }
        buf.push(b);
        if done {
            break;
        }
    }
}

fn name(buf: &mut Vec<u8>, s: &str) {
    uleb(buf, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

fn section(id: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    uleb(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

impl WasmModule {
    /// Serialize to a WASM 1.0 binary.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

        // Type section.
        let mut p = Vec::new();
        uleb(&mut p, self.types.len() as u64);
        for t in &self.types {
            p.push(0x60);
            uleb(&mut p, t.params.len() as u64);
            for v in &t.params {
                p.push(v.byte());
            }
            uleb(&mut p, t.results.len() as u64);
            for v in &t.results {
                p.push(v.byte());
            }
        }
        if !p.is_empty() || self.types.is_empty() {
            // Always emit type section when types exist or funcs/imports need it.
        }
        if !self.types.is_empty() {
            section(sec::TYPE, &p, &mut out);
        }

        // Import section.
        if !self.imports.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.imports.len() as u64);
            for i in &self.imports {
                name(&mut p, &i.module);
                name(&mut p, &i.name);
                p.push(0x00); // func import
                let ty_idx = self
                    .types
                    .iter()
                    .position(|t| t == &i.ty)
                    .expect("import type must exist in types");
                uleb(&mut p, ty_idx as u64);
            }
            section(sec::IMPORT, &p, &mut out);
        }

        // Function section.
        if !self.funcs.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.funcs.len() as u64);
            for &t in &self.funcs {
                uleb(&mut p, t as u64);
            }
            section(sec::FUNCTION, &p, &mut out);
        }

        // Memory section.
        if !self.memories.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.memories.len() as u64);
            for m in &self.memories {
                p.push(0x00); // limits flag: min only
                uleb(&mut p, m.min_pages as u64);
            }
            section(sec::MEMORY, &p, &mut out);
        }

        // Global section.
        if !self.globals.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.globals.len() as u64);
            for g in &self.globals {
                p.push(g.ty.byte());
                p.push(0x01); // mutable
                match g.ty {
                    ValType::I32 => {
                        p.push(0x41); // i32.const
                        sleb(&mut p, g.init);
                    }
                    ValType::I64 => {
                        p.push(0x42); // i64.const
                        sleb(&mut p, g.init);
                    }
                }
                p.push(0x0B); // end init expr
            }
            section(sec::GLOBAL, &p, &mut out);
        }

        // Export section.
        if !self.exports.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.exports.len() as u64);
            for (n, k, i) in &self.exports {
                name(&mut p, n);
                p.push(*k);
                uleb(&mut p, *i as u64);
            }
            section(sec::EXPORT, &p, &mut out);
        }

        // Code section.
        if !self.code.is_empty() {
            let mut p = Vec::new();
            uleb(&mut p, self.code.len() as u64);
            for c in &self.code {
                let mut body = Vec::new();
                uleb(&mut body, c.locals.len() as u64);
                for (n, t) in &c.locals {
                    uleb(&mut body, *n as u64);
                    body.push(t.byte());
                }
                body.extend_from_slice(&c.expr);
                body.push(0x0B); // end
                uleb(&mut p, body.len() as u64);
                p.extend_from_slice(&body);
            }
            section(sec::CODE, &p, &mut out);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_and_version() {
        let m = WasmModule::default();
        let b = m.to_bytes();
        assert_eq!(&b[0..8], &[0, 0x61, 0x73, 0x6D, 1, 0, 0, 0]);
    }

    #[test]
    fn leb_roundtrip() {
        let mut b = Vec::new();
        uleb(&mut b, 0x80);
        assert_eq!(b, vec![0x80, 0x01]);
        let mut b = Vec::new();
        sleb(&mut b, -1);
        assert_eq!(b, vec![0x7F]);
        let mut b = Vec::new();
        sleb(&mut b, 64);
        assert_eq!(b, vec![0xC0, 0x00]);
    }

    #[test]
    fn emits_code_section() {
        let mut m = WasmModule {
            types: vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            funcs: vec![0],
            memories: vec![MemoryDecl { min_pages: 1 }],
            globals: vec![Global {
                ty: ValType::I64,
                init: 0,
            }],
            ..Default::default()
        };
        m.code.push(CodeEntry {
            locals: vec![],
            expr: vec![0x42, 0x2A, 0x0B], // i64.const 42, end
        });
        let b = m.to_bytes();
        // Walk sections properly: id + u32-LEB size.
        let mut ids = Vec::new();
        let mut p = 8usize;
        while p < b.len() {
            let id = b[p];
            p += 1;
            let (size, n) = {
                let mut v = 0u64;
                let mut sh = 0u32;
                let mut i = 0;
                loop {
                    let byte = b[p + i];
                    v |= ((byte & 0x7F) as u64) << sh;
                    i += 1;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    sh += 7;
                }
                (v as usize, i)
            };
            p += n;
            ids.push(id);
            p += size;
        }
        assert!(
            ids.contains(&1)
                && ids.contains(&3)
                && ids.contains(&5)
                && ids.contains(&6)
                && ids.contains(&10)
        );
        assert!(b.len() > 20);
    }
}
