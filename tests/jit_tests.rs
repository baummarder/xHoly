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

/// Memory operands are now supported (Phase 14): absolute disp32
/// store/load roundtrip through WASM linear memory.
#[test]
fn jit_memory_operand_absolute() {
    // mov rax,42 ; mov [0x1000],rax ; mov rbx,[0x1000] ; mov rax,rbx ; ret
    // 48 89 04 25 <disp32> = mov [disp32], rax  (SIB base=101 → absolute)
    let mut code = vec![
        0x48, 0xC7, 0xC0, 42, 0, 0, 0, // mov rax,42
        0x48, 0x89, 0x04, 0x25, 0x00, 0x10, 0x00, 0x00, // mov [0x1000],rax
        0x48, 0x8B, 0x1C, 0x25, 0x00, 0x10, 0x00, 0x00, // mov rbx,[0x1000]
        0x48, 0x89, 0xD8, // mov rax,rbx
        0xC3,
    ];
    code.shrink_to_fit();
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), 42);
}

/// Register-based memory operand: mov [rbx],rax / mov rcx,[rbx].
#[test]
fn jit_memory_operand_reg_base() {
    // mov rax,7 ; mov rbx,0x2000 ; mov [rbx],rax ; mov rcx,[rbx] ;
    // mov rax,rcx ; ret
    let code = [
        0x48, 0xC7, 0xC0, 0x07, 0x00, 0x00, 0x00, // mov rax,7
        0x48, 0xC7, 0xC3, 0x00, 0x20, 0x00, 0x00, // mov rbx,0x2000
        0x48, 0x89, 0x03, // mov [rbx],rax
        0x48, 0x8B, 0x0B, // mov rcx,[rbx]
        0x48, 0x89, 0xC8, // mov rax,rcx
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), 7);
}

/// SIB + displacement: mov rax,[rbx+rcx*4+0x10].
#[test]
fn jit_memory_operand_sib() {
    // mov rbx,0x100 ; mov rcx,2 ; mov rax,[rbx+rcx*4+0x10]
    //   → addr = 0x100 + 8 + 0x10 = 0x118 ; pre-store 0x55 there first.
    // Setup: mov rdx,0x55 ; mov [0x118],rdx — reuse absolute store.
    let code = [
        // mov rdx,0x55
        0x48, 0xC7, 0xC2, 0x55, 0x00, 0x00, 0x00,
        // mov [0x118],rdx  (48 89 14 25 18 01 00 00)
        0x48, 0x89, 0x14, 0x25, 0x18, 0x01, 0x00, 0x00, // mov rbx,0x100
        0x48, 0xC7, 0xC3, 0x00, 0x01, 0x00, 0x00, // mov rcx,2
        0x48, 0xC7, 0xC1, 0x02, 0x00, 0x00, 0x00,
        // mov rax,[rbx+rcx*4+0x10]  → mod=01? use mod=2 disp32:
        // 48 8B 84 8B 10 00 00 00  (modrm 84: mod=2 reg=0 rm=4→SIB;
        //   sib 8B: scale=2(×4) index=1(rcx) base=3(rbx))
        0x48, 0x8B, 0x84, 0x8B, 0x10, 0x00, 0x00, 0x00, 0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), 0x55);
}

/// PF tracking: `or rax,0` on rax=3 → even parity → PF=1 → jp taken.
#[test]
fn jit_pf_parity_flag() {
    // mov rax,3 ; or rax,0 ; jp +7 ; mov rax,2 ; ret
    let code = [
        0x48, 0xC7, 0xC0, 0x03, 0x00, 0x00, 0x00, // mov rax,3
        0x48, 0x83, 0xC8, 0x00, // or rax,0 (83 /1)
        0x7A, 0x07, // jp +7 → ret
        0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00, // mov rax,2
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), 3); // jp taken → rax stays 3
}

/// CF tracking: sub borrow → jb taken.
#[test]
fn jit_cf_borrow_flag() {
    // mov rax,1 ; sub rax,2 ; jb +7 ; mov rax,5 ; ret
    // rax = 1-2 = u64::MAX, CF=1 → jb taken → skip mov rax,5.
    let code = [
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
        0x48, 0x83, 0xE8, 0x02, // sub rax,2
        0x72, 0x07, // jb +7 → ret
        0x48, 0xC7, 0xC0, 0x05, 0x00, 0x00, 0x00, // mov rax,5
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), -1); // u64::MAX as i64
}

/// SIMD: movd + movdqa + paddq + movd roundtrip.
#[test]
fn jit_simd_movd_paddq() {
    // mov rax,0x55667788 ; movd xmm0,eax ; movdqa xmm1,xmm0 ;
    // paddq xmm0,xmm1 ; movd eax,xmm0 ; ret
    // xmm0.lo = 0x55667788*2 = 0xAACCEF10 → rax low32.
    let code = [
        0x48, 0xC7, 0xC0, 0x88, 0x77, 0x66, 0x55, // mov rax,0x55667788
        0x66, 0x0F, 0x6E, 0xC0, // movd xmm0,eax
        0x66, 0x0F, 0x6F, 0xC8, // movdqa xmm1,xmm0
        0x66, 0x0F, 0xD4, 0xC1, // paddq xmm0,xmm1
        0x66, 0x0F, 0x7E, 0xC0, // movd eax,xmm0
        0xC3,
    ];
    let mut c = JitCompiler::new();
    let m = c.compile(&code).unwrap();
    let mut rt = JitRuntime::new();
    rt.load(&m.wasm, m.block_map.clone(), vec![Box::new(|_| Ok(0))])
        .unwrap();
    assert_eq!(rt.run(0).unwrap(), 0xAACCEF10);
}

/// Emulator loop: JIT path and interpreter fallback produce the same
/// result for the supported subset.
#[test]
fn jit_emulator_run_program() {
    use xholy::jit::{JitConfig, WasmJitEmulator};
    let code = [
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00, // mov rax,1
        0x48, 0x83, 0xC0, 0x02, // add rax,2
        0xC3,
    ];
    // JIT enabled.
    let mut emu = WasmJitEmulator::new();
    let cfg = JitConfig {
        enabled: true,
        fallback_to_interpreter: true,
        ..Default::default()
    };
    assert_eq!(emu.run_program(&code, &cfg).unwrap(), 3);
    // Interpreter fallback (JIT disabled).
    let cfg2 = JitConfig {
        enabled: false,
        fallback_to_interpreter: true,
        ..Default::default()
    };
    assert_eq!(emu.run_program(&code, &cfg2).unwrap(), 3);
    // JIT disabled + fallback off → error.
    let cfg3 = JitConfig {
        enabled: false,
        fallback_to_interpreter: false,
        ..Default::default()
    };
    assert!(emu.run_program(&code, &cfg3).is_err());
}

/// Interpreter executes memory operands independently of the JIT.
#[test]
fn jit_interpreter_memory_ops() {
    use xholy::jit::X86Interpreter;
    let code = [
        0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, // mov rax,42
        0x48, 0x89, 0x04, 0x25, 0x00, 0x10, 0x00, 0x00, // mov [0x1000],rax
        0x48, 0x8B, 0x1C, 0x25, 0x00, 0x10, 0x00, 0x00, // mov rbx,[0x1000]
        0x48, 0x89, 0xD8, // mov rax,rbx
        0xC3,
    ];
    let mut it = X86Interpreter::new();
    assert_eq!(it.run(&code, 10_000).unwrap(), 42);
}

/// Interpreter SIMD path matches the JIT result.
#[test]
fn jit_interpreter_simd() {
    use xholy::jit::X86Interpreter;
    let code = [
        0x48, 0xC7, 0xC0, 0x88, 0x77, 0x66, 0x55, // mov rax,0x55667788
        0x66, 0x0F, 0x6E, 0xC0, // movd xmm0,eax
        0x66, 0x0F, 0x6F, 0xC8, // movdqa xmm1,xmm0
        0x66, 0x0F, 0xD4, 0xC1, // paddq xmm0,xmm1
        0x66, 0x0F, 0x7E, 0xC0, // movd eax,xmm0
        0xC3,
    ];
    let mut it = X86Interpreter::new();
    assert_eq!(it.run(&code, 10_000).unwrap(), 0xAACCEF10);
}

/// EmulationCore::compile_and_run dispatches through the JIT branch.
#[test]
fn emulation_core_compile_and_run() {
    use xholy::core::EmulationCore;
    use xholy::jit::JitConfig;
    // Write a tiny x86 program to a temp file for EmulationCore::load.
    let code = [
        0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, // mov rax,42
        0xC3,
    ];
    let path = std::env::temp_dir().join("xholy_jit_test.bin");
    std::fs::write(&path, code).unwrap();
    let mut core = EmulationCore::load(&path);
    core.enable_jit(JitConfig {
        enabled: true,
        fallback_to_interpreter: true,
        ..Default::default()
    });
    assert!(core.jit_enabled());
    assert_eq!(core.compile_and_run(&code).unwrap(), 42);
    let _ = std::fs::remove_file(&path);
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
