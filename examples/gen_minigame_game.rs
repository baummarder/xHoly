//! Regenerates `games/minigame/` — a real, bootable XEX2 guest game.
//!
//! Unlike `gen_minimal_game` (a one-shot "Hello, xHoly!" demo), this
//! image contains a persistent **game loop** written in real AArch64:
//!
//! 1. reads the kernel tick counter (`SYS_TICKS`),
//! 2. reads host input from the guest input aperture,
//! 3. bumps the framebuffer sequence number,
//! 4. paints a moving 160px row whose colour follows the input,
//! 5. yields to the host with `SYS_FRAME_END` and resumes next frame.
//!
//! The file uses the **real big-endian XEX2 layout** (Phase 19) with an
//! optional-header directory, so it goes through the same parse/decode
//! path as a retail header. Run: `cargo run --example gen_minigame_game`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use xholy::core::kernel::{
    GUEST_FB_BASE, GUEST_FB_H, GUEST_FB_MAGIC, GUEST_FB_W, GUEST_INPUT_BASE,
};
use xholy::core::loader::xex_parser::{XEX_HEADER_ENTRY_POINT, XEX_HEADER_IMAGE_BASE_ADDRESS};

const HEADER_SIZE: usize = 0x200;
const IMAGE_BASE: u32 = 0x8200_0000;

const SYS_TICKS: u16 = 17;
const SYS_FRAME_END: u16 = 18;

// ---------------------------------------------------------------------------
// Minimal AArch64 assembler (subset implemented by the interpreter)
// ---------------------------------------------------------------------------

fn movz_x(rd: u8, imm16: u16, shift: u32) -> u32 {
    0xD2800000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | rd as u32
}
fn movk_x(rd: u8, imm16: u16, shift: u32) -> u32 {
    0xF2800000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | rd as u32
}
fn movz_w(rd: u8, imm16: u16) -> u32 {
    0x52800000 | ((imm16 as u32) << 5) | rd as u32
}
fn movk_w(rd: u8, imm16: u16, shift: u32) -> u32 {
    0x72800000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | rd as u32
}
fn ldr_w(rt: u8, rn: u8, imm12: u16) -> u32 {
    0xB9400000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rt as u32
}
fn str_w(rt: u8, rn: u8, imm12: u16) -> u32 {
    0xB9000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rt as u32
}
fn add_imm(rd: u8, rn: u8, imm12: u16) -> u32 {
    0x91000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rd as u32
}
fn sub_imm(rd: u8, rn: u8, imm12: u16) -> u32 {
    0xD1000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rd as u32
}
fn add_shift(rd: u8, rn: u8, rm: u8, lsl: u32) -> u32 {
    0x8B000000 | ((rm as u32) << 16) | ((lsl & 0x3F) << 10) | ((rn as u32) << 5) | rd as u32
}
fn and_reg(rd: u8, rn: u8, rm: u8) -> u32 {
    0x8A000000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32
}
fn orr_reg(rd: u8, rn: u8, rm: u8) -> u32 {
    0xAA000000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32
}
fn svc(n: u16) -> u32 {
    0xD4000001 | ((n as u32) << 5)
}
fn b_placeholder() -> u32 {
    0x1400_0000
}
fn cbnz_placeholder(rt: u8) -> u32 {
    // sf=1, op=1 (CBNZ); 0xB7... would be TBZ/TBNZ.
    0xB5000000 | rt as u32
}

/// Tiny labelled assembler: branches are patched once all labels exist.
struct Asm {
    code: Vec<u32>,
    labels: HashMap<String, usize>,
    /// (word index, target label, instruction kind: 0=B, 1=CBNZ)
    fixups: Vec<(usize, String, u8)>,
}

impl Asm {
    fn new() -> Self {
        Self {
            code: Vec::new(),
            labels: HashMap::new(),
            fixups: Vec::new(),
        }
    }
    fn emit(&mut self, word: u32) -> usize {
        let at = self.code.len();
        self.code.push(word);
        at
    }
    fn label(&mut self, name: &str) {
        self.labels.insert(name.into(), self.code.len());
    }
    fn b(&mut self, target: &str) {
        let at = self.emit(b_placeholder());
        self.fixups.push((at, target.into(), 0));
    }
    fn cbnz(&mut self, rt: u8, target: &str) {
        let at = self.emit(cbnz_placeholder(rt));
        self.fixups.push((at, target.into(), 1));
    }
    fn load_const(&mut self, rd: u8, value: u64) {
        self.emit(movz_x(rd, (value & 0xFFFF) as u16, 0));
        for shift in [16u32, 32, 48] {
            let part = ((value >> shift) & 0xFFFF) as u16;
            if part != 0 {
                self.emit(movk_x(rd, part, shift));
            }
        }
    }
    fn resolve(mut self) -> Vec<u32> {
        for (at, target, kind) in self.fixups {
            let dest = *self
                .labels
                .get(&target)
                .unwrap_or_else(|| panic!("unknown label {target}"));
            // AArch64 branch immediates are relative to the branch's own
            // address (the interpreter adds imm to `pc`), so no -1 here.
            let rel = dest as i64 - at as i64;
            self.code[at] = match kind {
                0 => 0x1400_0000 | ((rel as u32) & 0x03FF_FFFF),
                _ => (self.code[at] & 0xFFE0_001F) | (((rel as u32) & 0x7FFFF) << 5),
            };
        }
        self.code
    }
}

/// Build the game loop payload.
fn guest_payload() -> Vec<u8> {
    let mut a = Asm::new();

    // x19 = framebuffer base, x20 = input aperture.
    a.load_const(19, GUEST_FB_BASE);
    a.load_const(20, GUEST_INPUT_BASE);
    // x25 = row mask 0x3F, x28 = button bit 0, x29 = explicit zero
    // (the interpreter exposes r31 as SP, so MOV-from-ZR uses x29 here).
    a.emit(movz_x(25, 0x3F, 0));
    a.emit(movz_x(28, 1, 0));
    a.emit(movz_x(29, 0, 0));

    // Framebuffer header (written once).
    a.emit(movz_w(8, (GUEST_FB_MAGIC & 0xFFFF) as u16));
    a.emit(movk_w(8, (GUEST_FB_MAGIC >> 16) as u16, 16));
    a.emit(str_w(8, 19, 0));
    a.emit(movz_w(8, GUEST_FB_W as u16));
    a.emit(str_w(8, 19, 1));
    a.emit(movz_w(8, GUEST_FB_H as u16));
    a.emit(str_w(8, 19, 2));

    // ---- frame loop ----------------------------------------------------
    a.label("frame");
    // Tick counter (timer).
    a.emit(movz_x(0, SYS_TICKS, 0));
    a.emit(svc(SYS_TICKS));
    a.emit(orr_reg(21, 29, 0)); // x21 = ticks
                                // Host input.
    a.emit(ldr_w(22, 20, 0)); // x22 = buttons
                              // y = ticks & 0x3F (stays inside the 120-row framebuffer).
    a.emit(and_reg(26, 21, 25));
    // row offset = y * 640 = (y << 9) + (y << 7).
    a.emit(add_shift(27, 29, 26, 9));
    a.emit(add_shift(27, 27, 26, 7));
    a.emit(add_imm(23, 19, 16)); // pixels start after the 16-byte header
    a.emit(add_shift(23, 23, 27, 0)); // x23 = pixel row address
                                      // seq++ so the host sees a fresh frame.
    a.emit(ldr_w(0, 19, 3));
    a.emit(add_imm(0, 0, 1));
    a.emit(str_w(0, 19, 3));
    // Colour from input bit 0: cyan when pressed, red otherwise.
    a.emit(and_reg(24, 22, 28));
    a.cbnz(24, "cyan");
    a.emit(movz_w(9, 0x28C8)); // red RGBA(200,40,40,255)
    a.emit(movk_w(9, 0xFF28, 16));
    a.b("fill");
    a.label("cyan");
    a.emit(movz_w(9, 0xE21E)); // cyan RGBA(30,226,226,255)
    a.emit(movk_w(9, 0xFFE2, 16));
    // Fill one row: 160 pixels x 4 bytes.
    a.label("fill");
    a.emit(movz_x(24, 160, 0));
    a.label("fill_loop");
    a.emit(str_w(9, 23, 0));
    a.emit(add_imm(23, 23, 4));
    a.emit(sub_imm(24, 24, 1));
    a.cbnz(24, "fill_loop");
    // Yield to host, then resume the loop next frame.
    a.emit(movz_x(0, SYS_FRAME_END, 0));
    a.emit(svc(SYS_FRAME_END));
    a.b("frame");

    let code = a.resolve();
    let mut bytes = Vec::with_capacity(code.len() * 4);
    for word in code {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn put_be32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new("games/minigame");
    fs::create_dir_all(dir)?;

    let payload = guest_payload();

    // Real XEX2: big-endian 0x18-byte fixed header + optional directory.
    let mut xex = vec![0u8; HEADER_SIZE];
    xex[0..4].copy_from_slice(b"XEX2");
    put_be32(&mut xex, 0x04, 0x0000_0001); // module flags: title
    put_be32(&mut xex, 0x08, HEADER_SIZE as u32); // header size / PE offset
    put_be32(&mut xex, 0x0C, 0); // reserved
    put_be32(&mut xex, 0x10, 0); // no security block
    put_be32(&mut xex, 0x14, 2); // two optional headers
                                 // Directory at 0x18: entry point (inline) + image base (inline ref).
    put_be32(&mut xex, 0x18, XEX_HEADER_ENTRY_POINT);
    put_be32(&mut xex, 0x1C, IMAGE_BASE);
    put_be32(&mut xex, 0x20, XEX_HEADER_IMAGE_BASE_ADDRESS);
    put_be32(&mut xex, 0x24, IMAGE_BASE);

    xex.extend_from_slice(&payload);
    fs::write(dir.join("default.xex"), &xex)?;

    let config = r#"<?xml version="1.0" encoding="utf-8"?>
<GameInfo>
  <TitleID>XHOLYMINIGAME1</TitleID>
  <Name>xHoly Minigame</Name>
  <Studio>xHoly Research</Studio>
  <Path>/default.xex</Path>
  <ReleaseDate>2026-11-23</ReleaseDate>
</GameInfo>
"#;
    fs::write(dir.join("xboxconfig.xml"), config)?;

    println!(
        "wrote {} ({} bytes header + {} payload bytes)",
        dir.join("default.xex").display(),
        HEADER_SIZE,
        payload.len()
    );
    Ok(())
}
