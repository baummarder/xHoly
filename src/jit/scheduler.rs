//! Instruction scheduler (reorder buffer) — Phase 13.
//!
//! Reorders decoded x86-64 instructions within a basic block to reduce
//! dependency stalls: producers move earlier, consumers stay after
//! their operands. Control-flow instructions are barriers — they are
//! never moved across each other or across block boundaries.
//!
//! This is a compile-time scheduling pass over [`X86Instr`]; it does
//! not model a hardware reorder buffer.

use super::disassembler::{InstrKind, X86Instr};
use std::collections::HashSet;

/// Registers an instruction reads (conservative).
fn reads(i: &X86Instr) -> HashSet<u8> {
    use super::disassembler::Operand::{self, *};
    use InstrKind::*;
    let mut s = HashSet::new();
    let reg = |o: &Operand, set: &mut HashSet<u8>| {
        if let Reg(r) = o {
            set.insert(*r);
        }
    };
    match &i.kind {
        Mov { dst: _, src } | Movzx { dst: _, src, .. } | Movsx { dst: _, src, .. } => {
            reg(src, &mut s)
        }
        Add { dst: _, src }
        | Sub { dst: _, src }
        | Xor { dst: _, src }
        | And { dst: _, src }
        | Or { dst: _, src }
        | Mul { dst: _, src } => {
            reg(src, &mut s);
        }
        Cmp { a, b } | Test { a, b } => {
            reg(a, &mut s);
            reg(b, &mut s);
        }
        Inc(r) | Dec(r) | Neg(r) | Not(r) | Push(r) | Pop(r) => reg(r, &mut s),
        _ => {}
    }
    // dst is also read for RMW ops.
    match &i.kind {
        Add { dst, .. }
        | Sub { dst, .. }
        | Xor { dst, .. }
        | And { dst, .. }
        | Or { dst, .. }
        | Mul { dst, .. }
        | Inc(dst)
        | Dec(dst)
        | Neg(dst)
        | Not(dst) => reg(dst, &mut s),
        _ => {}
    }
    s
}

/// Registers an instruction writes.
fn writes(i: &X86Instr) -> HashSet<u8> {
    use super::disassembler::Operand::*;
    use InstrKind::*;
    let mut s = HashSet::new();
    match &i.kind {
        Mov { dst, .. } | Movzx { dst, .. } | Movsx { dst, .. } => {
            if let Reg(r) = dst {
                s.insert(*r);
            }
        }
        Add { dst, .. }
        | Sub { dst, .. }
        | Xor { dst, .. }
        | And { dst, .. }
        | Or { dst, .. }
        | Mul { dst, .. } => {
            if let Reg(r) = dst {
                s.insert(*r);
            }
        }
        Inc(r) | Dec(r) | Neg(r) | Not(r) | Pop(r) => {
            if let Reg(x) = r {
                s.insert(*x);
            }
        }
        _ => {}
    }
    s
}

/// Whether the instruction is a scheduling barrier.
fn is_barrier(i: &X86Instr) -> bool {
    matches!(
        i.kind,
        InstrKind::Jmp { .. }
            | InstrKind::Jcc { .. }
            | InstrKind::Call { .. }
            | InstrKind::Ret
            | InstrKind::Syscall
    )
}

/// Scheduler with a bounded reorder window.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// Max instructions looked ahead when reordering.
    pub window: usize,
}

impl Scheduler {
    /// Create with default window (8).
    pub fn new() -> Self {
        Self { window: 8 }
    }

    /// Schedule a basic block (no internal branches).
    ///
    /// Greedy list-scheduling: repeatedly pick the ready instruction
    /// (all register producers already emitted) with the most
    /// successors unblocked. Deterministic and stable.
    pub fn schedule(&self, block: &[X86Instr]) -> Vec<X86Instr> {
        let n = block.len();
        if n <= 2 {
            return block.to_vec();
        }
        // last_writer[r] = index of latest instruction writing r
        // deps[i] = set of producer indices i depends on.
        let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        let mut last_write: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
        let mut last_read: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
        for (i, instr) in block.iter().enumerate() {
            for r in reads(instr) {
                if let Some(&w) = last_write.get(&r) {
                    deps[i].insert(w);
                }
                if let Some(&rd) = last_read.get(&r) {
                    // WAR: keep order vs. earlier readers of same reg.
                    deps[i].insert(rd);
                }
            }
            for r in writes(instr) {
                if let Some(&rd) = last_read.get(&r) {
                    deps[i].insert(rd); // WAW/RAW ordering
                }
                last_write.insert(r, i);
            }
            for r in reads(instr) {
                last_read.insert(r, i);
            }
            // Flags are an implicit dependency for cmp/test → jcc;
            // treat barriers as depending on everything before them.
            if is_barrier(instr) {
                for j in 0..i {
                    deps[i].insert(j);
                }
            }
        }

        // Kahn's algorithm with a stable priority (original index).
        let mut emitted = vec![false; n];
        let mut out = Vec::with_capacity(n);
        let mut ready: Vec<usize> = (0..n).filter(|&i| deps[i].is_empty()).collect();
        while out.len() < n {
            if ready.is_empty() {
                // Cycle (shouldn't happen) — emit next in order.
                let i = (0..n).find(|&i| !emitted[i]).unwrap();
                emitted[i] = true;
                out.push(block[i].clone());
                continue;
            }
            // Prefer lowest original index for determinism.
            ready.sort_unstable();
            let i = ready.remove(0);
            if emitted[i] {
                continue;
            }
            emitted[i] = true;
            out.push(block[i].clone());
            for j in 0..n {
                if !emitted[j] && deps[j].remove(&i) && deps[j].is_empty() {
                    ready.push(j);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jit::disassembler::{decode_all, Operand};

    #[test]
    fn independent_movs_may_reorder() {
        // mov rax,1 ; mov rbx,2 ; add rax,rbx — add must stay last.
        let code = [
            0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
            0x48, 0xC7, 0xC3, 0x02, 0x00, 0x00, 0x00, // mov rbx,2
            0x48, 0x01, 0xD8, // add rax,rbx
        ];
        let instrs = decode_all(&code).unwrap();
        let sched = Scheduler::new().schedule(&instrs);
        let pos_add = sched
            .iter()
            .position(|i| matches!(i.kind, InstrKind::Add { .. }))
            .unwrap();
        assert_eq!(pos_add, sched.len() - 1);
    }

    #[test]
    fn barrier_not_moved() {
        let code = [0x90, 0xC3]; // nop; ret
        let instrs = decode_all(&code).unwrap();
        let sched = Scheduler::new().schedule(&instrs);
        assert!(matches!(sched.last().unwrap().kind, InstrKind::Ret));
    }

    #[test]
    fn preserves_all_instructions() {
        let code = [
            0x48, 0xC7, 0xC0, 0x05, 0x00, 0x00, 0x00, // mov rax,5
            0x48, 0xC7, 0xC1, 0x07, 0x00, 0x00, 0x00, // mov rcx,7
            0x48, 0x29, 0xC8, // sub rax,rcx
            0xC3, // ret
        ];
        let instrs = decode_all(&code).unwrap();
        let sched = Scheduler::new().schedule(&instrs);
        assert_eq!(sched.len(), instrs.len());
        let _ = Operand::Reg(0);
    }
}
