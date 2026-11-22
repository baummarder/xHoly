//! Phase 13 JIT tests — x86-64 → WASM compile + execute.

use xholy::jit::{decode_all, disassemble, JitCompiler, JitRuntime, RuntimeConfig};

/// mov rax,1 ; add rax,2 ; ret → rax == 3.
#[test]
fn jit_mov_add_ret() {
    let code = [
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
        0x48, 0x83, 0xC0, 0x02, // add rax,2
        0xC3, // ret
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let wasm = &m.wasm;
    let map = m.block_map.clone();
    assert_eq!(&wasm[0..4], &[0, 0x61, 0x73, 0x6D]);

    let mut rt = JitRuntime::with_config(RuntimeConfig {
        memory_pages: 4,
        ..Default::default()
    });
    rt.load(wasm, map, vec![Box::new(|_| Ok(0))]).unwrap();
    let rax = rt.run(0).unwrap();
    assert_eq!(rax, 3);
}

/// cmp + je: equal operands take the branch (skip mov rax,2).
#[test]
fn jit_conditional_branch() {
    let code = [
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
        0x48, 0xC7, 0xC3, 0x01, 0x00, 0x00, 0x00, // mov rbx,1
        0x48, 0x39, 0xD8, // cmp rax,rbx
        0x74, 0x07, // je +7 → ret
        0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00, // mov rax,2
        0xC3, // ret
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let wasm = &m.wasm;
    let map = m.block_map.clone();
    assert!(m.blocks.len() >= 2);
    let mut rt = JitRuntime::new();
    rt.load(wasm, map, vec![Box::new(|_| Ok(0))]).unwrap();
    assert_eq!(rt.run(0).unwrap(), 1); // branch taken
}

/// jne path: not-equal falls through to mov rax,2.
#[test]
fn jit_branch_not_taken() {
    let code = [
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
        0x48, 0xC7, 0xC3, 0x02, 0x00, 0x00, 0x00, // mov rbx,2
        0x48, 0x39, 0xD8, // cmp rax,rbx
        0x74, 0x07, // je +7 (not taken)
        0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00, // mov rax,2
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let wasm = &m.wasm;
    let map = m.block_map.clone();
    let mut rt = JitRuntime::new();
    rt.load(wasm, map, vec![Box::new(|_| Ok(0))]).unwrap();
    assert_eq!(rt.run(0).unwrap(), 2);
}

/// syscall → host import result lands in rax.
#[test]
fn jit_syscall_host_call() {
    let code = [0x0F, 0x05, 0xC3]; // syscall ; ret
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let wasm = &m.wasm;
    let map = m.block_map.clone();
    let mut rt = JitRuntime::new();
    rt.load(wasm, map, vec![Box::new(|_| Ok(42))]).unwrap();
    assert_eq!(rt.run(0).unwrap(), 42);
}

/// Disassembler produces AT&T text.
#[test]
fn disassembler_output() {
    let code = [0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, 0xC3];
    let asm = disassemble(&code).unwrap();
    assert!(asm.contains("mov") && asm.contains("%rax"));
    assert!(asm.contains("ret"));
    let instrs = decode_all(&code).unwrap();
    assert_eq!(instrs.len(), 2);
}

/// Unsupported opcodes rejected honestly.
#[test]
fn decoder_rejects_unsupported() {
    // ud2 = 0F 0B
    let mut c = JitCompiler::new();
    assert!(c.compile(&[0x0F, 0x0B]).is_err());
    // SSE movaps = 0F 28 /r
    assert!(c.compile(&[0x0F, 0x28, 0xC0]).is_err());
}

/// push/pop roundtrip through guest stack.
#[test]
fn jit_push_pop() {
    // mov rax,7 ; push rax ; pop rbx ; ret
    let code = [
        0x48, 0xC7, 0xC0, 0x07, 0x00, 0x00, 0x00, // mov rax,7
        0x50, // push rax
        0x5B, // pop rbx
        0x48, 0x89,
        0xD8, // mov rax,rbx (48 89 /r: mov r/m64,r64; modrm D8 = rax←rbx? 89 D8: mod3 reg=3(rbx) rm=0(rax) → mov rax,rbx)
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let wasm = &m.wasm;
    let map = m.block_map.clone();
    let mut rt = JitRuntime::new();
    rt.load(wasm, map, vec![Box::new(|_| Ok(0))]).unwrap();
    assert_eq!(rt.run(0).unwrap(), 7);
}

/// Memory operand rejection.
#[test]
fn decoder_rejects_memory_operands() {
    // mov rax,[rbx] = 48 8B 03 (mod=0)
    let mut c = JitCompiler::new();
    assert!(c.compile(&[0x48, 0x8B, 0x03]).is_err());
}

/// Performance: compile 100 instructions < 50 ms.
#[test]
fn jit_compile_perf() {
    // 100 × (mov + add) pairs.
    let mut code = Vec::new();
    for i in 0..50u8 {
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, i, 0, 0, 0]); // mov rax,i
        code.extend_from_slice(&[0x48, 0x83, 0xC0, 0x01]); // add rax,1
    }
    code.push(0xC3);
    let start = std::time::Instant::now();
    let mut c = JitCompiler::new();
    let _m = c.compile(&code).unwrap();
    assert!(start.elapsed().as_millis() < 50);
}
