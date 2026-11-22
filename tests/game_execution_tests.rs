//! Phase-8 integration tests: real AArch64 execution + kernel + loader.

use std::path::PathBuf;
use xholy::core::cpu::interpreter::Arm64Cpu;
use xholy::core::execution::{execute_game, hello_world_program, GUEST_MEM_SIZE};
use xholy::core::kernel::{self, Kernel, SYS_EXIT, SYS_SLEEP, SYS_WRITE, SYS_YIELD};
use xholy::core::loader::{load_game, parse_xboxconfig, GameImage, GameMetadata};

fn image_from(bytes: Vec<u8>) -> GameImage {
    GameImage {
        title: "test".into(),
        title_id: None,
        load_base: kernel::USER_BASE,
        entry: kernel::USER_BASE,
        image: bytes,
        source: PathBuf::from("mem"),
        metadata: GameMetadata::default(),
    }
}

#[test]
fn arm64_interpreter_executes_real_encodings() {
    // MOVZ x0,#42 ; MOVK x0,#1,lsl16 ; BRK → x0 = 0x0001_0000_002A
    let code = [
        (0xD2800000u32 | (42u32 << 5)),         // MOVZ x0, #42
        (0xF2800000 | (1 << 21) | (1u32 << 5)), // MOVK x0, #1, lsl #16
        0xD4200000,                             // BRK
    ];
    let mut bytes = Vec::new();
    for w in code {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    let img = image_from(bytes);
    let report = execute_game(&img, 100).expect("execute");
    assert!(report.steps >= 3);
}

#[test]
fn kernel_write_syscall_produces_console_output() {
    let bytes = hello_world_program(b"Phase8\n");
    let report = execute_game(&image_from(bytes), 10_000).expect("run");
    assert_eq!(report.output, b"Phase8\n");
}

#[test]
fn kernel_scheduler_tracks_threads() {
    let cpu = Arm64Cpu::new(64 * 1024);
    let mut k = Kernel::new(&cpu);
    k.threads
        .push(xholy::core::kernel::Thread::from_cpu(1, &cpu));
    k.current = 0;
    k.schedule_next();
    assert_eq!(k.current, 1);
}

#[test]
fn loader_parses_xboxconfig_and_flat_images() {
    // xboxconfig parsing
    let dir = std::env::temp_dir().join("xholy_p8_cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let xml = "<GameInfo><TitleID>XYZ</TitleID><Name>Cool Game</Name></GameInfo>";
    let cfg = dir.join("xboxconfig.xml");
    std::fs::write(&cfg, xml).unwrap();
    let meta = parse_xboxconfig(&cfg).unwrap();
    assert_eq!(meta.title_id.as_deref(), Some("XYZ"));
    assert_eq!(meta.name.as_deref(), Some("Cool Game"));

    // flat binary load
    let bin = dir.join("flat.bin");
    std::fs::write(&bin, vec![0x11; 128]).unwrap();
    let img = load_game(&bin).unwrap();
    assert_eq!(img.image.len(), 128);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn game_image_roundtrip_into_guest_memory() {
    let img = image_from(vec![0x5A; 256]);
    let mut cpu = Arm64Cpu::new(GUEST_MEM_SIZE);
    cpu.va_base = kernel::KERNEL_BASE;
    img.load_into(&mut cpu).unwrap();
    let off = (kernel::USER_BASE - cpu.va_base) as usize;
    assert_eq!(cpu.memory[off], 0x5A);
    assert_eq!(cpu.pc, kernel::USER_BASE);
}

#[test]
fn kernel_syscall_table_covers_prompt_set() {
    // Prompt required svc#0..#6 (+exit/yield/ticks we added).
    assert_eq!(SYS_WRITE, 0);
    assert_eq!(1, xholy::core::kernel::SYS_CREATE_THREAD);
    assert_eq!(2, xholy::core::kernel::SYS_CREATE_PROCESS);
    assert_eq!(SYS_SLEEP, 3);
    assert_eq!(4, xholy::core::kernel::SYS_WRITE_FILE);
    assert_eq!(5, xholy::core::kernel::SYS_READ_FILE);
    assert_eq!(SYS_YIELD, 6);
    assert_eq!(SYS_EXIT, 7);
    assert_eq!(8, xholy::core::kernel::SYS_SUSPEND);
    assert_eq!(16, xholy::core::kernel::SYS_SET_CONTEXT);
    // Ticks moved to svc#17 (svc#8 is Suspend in the Phase-10 table).
    assert_eq!(17, xholy::core::kernel::SYS_TICKS);
}
