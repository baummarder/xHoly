//! Regenerates the bundled minimal guest game (`games/minimal/`).
//!
//! Produces a real on-disk XEX2 image: a little-endian `XEX2` header
//! (Xbox-One-style scaffold layout — see `core::loader::xex_parser`)
//! followed by an AArch64 payload that:
//!
//! 1. Writes a framebuffer header + a two-tone pattern ("x"-colored
//!    top half, "y"-colored bottom half) into the guest FB aperture
//!    ([`kernel::GUEST_FB_BASE`]) — this is what the GUI blits.
//! 2. Prints `Hello, xHoly!` via SYS_WRITE.
//! 3. Exits via SYS_EXIT.
//!
//! Run: `cargo run --example gen_minimal_game`

use std::fs;
use std::path::Path;
use xholy::core::kernel::USER_BASE;
use xholy::core::kernel::{GUEST_FB_BASE, GUEST_FB_H, GUEST_FB_MAGIC, GUEST_FB_W};

/// XEX2 fixed header size used by our minimal format (base fields +
/// room for the title-id quad at 0x184).
const HEADER_SIZE: usize = 0x1A0;
/// Guest VA the image is linked at (mapped to USER_BASE by the loader).
const IMAGE_BASE: u32 = 0x8200_0000;

// ---------------------------------------------------------------------------
// Tiny AArch64 assembler subset (matches the interpreter's coverage)
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
/// STR Wt, [Xn, #imm12*4]
fn str_w(rt: u8, rn: u8, imm12: u16) -> u32 {
    0xB9000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rt as u32
}
/// ADD Xd, Xn, #imm12
fn add_x(rd: u8, rn: u8, imm12: u16) -> u32 {
    0x91000000 | ((imm12 as u32) << 10) | ((rn as u32) << 5) | rd as u32
}
/// CMP Xn, Xm  (SUBS XZR, Xn, Xm)
fn cmp_x(rn: u8, rm: u8) -> u32 {
    0xEB000000 | ((rm as u32) << 16) | ((rn as u32) << 5) | 31
}
/// B.cond — `off_words` is the signed instruction offset.
fn b_cond(cond: u8, off_words: i32) -> u32 {
    0x54000000 | (((off_words as u32) & 0x7FFFF) << 5) | cond as u32
}
fn svc() -> u32 {
    0xD4000001
}
fn brk() -> u32 {
    0xD4200000
}

const COND_LT: u8 = 0xB;

/// 64-bit constant into a register (MOVZ + MOVK chain).
fn load_const(code: &mut Vec<u32>, rd: u8, val: u64) {
    code.push(movz_x(rd, (val & 0xFFFF) as u16, 0));
    for shift in [16u32, 32, 48] {
        let part = ((val >> shift) & 0xFFFF) as u16;
        if part != 0 {
            code.push(movk_x(rd, part, shift));
        }
    }
}

/// Build the guest payload: framebuffer pattern + hello + exit.
fn guest_payload(msg: &[u8]) -> Vec<u8> {
    let mut code: Vec<u32> = Vec::new();

    // x19 = GUEST_FB_BASE (guest VA, absolute).
    load_const(&mut code, 19, GUEST_FB_BASE);

    // --- framebuffer header -------------------------------------------
    // magic
    code.push(movz_w(8, (GUEST_FB_MAGIC & 0xFFFF) as u16));
    code.push(movk_w(8, (GUEST_FB_MAGIC >> 16) as u16, 16));
    code.push(str_w(8, 19, 0)); // [x19 + 0]
                                // width
    code.push(movz_w(8, GUEST_FB_W as u16));
    code.push(str_w(8, 19, 1)); // [x19 + 4]
                                // height
    code.push(movz_w(8, GUEST_FB_H as u16));
    code.push(str_w(8, 19, 2)); // [x19 + 8]
                                // seq = 1
    code.push(movz_w(8, 1));
    code.push(str_w(8, 19, 3)); // [x19 + 12]

    // --- colors ---------------------------------------------------------
    // "x"-half: warm red  RGBA(200, 40, 40, 255) = 0xFF2828C8
    code.push(movz_w(9, 0x28C8));
    code.push(movk_w(9, 0xFF28, 16));
    // "y"-half: cyan       RGBA(30, 226, 226, 255) = 0xFFE2E21E
    code.push(movz_w(10, 0xE21E));
    code.push(movk_w(10, 0xFFE2, 16));

    // --- fill loops -----------------------------------------------------
    // x20 = pixel index, x21 = pixel addr cursor (FB+16),
    // x22 = total pixels, x23 = half (top/bottom split).
    code.push(movz_x(20, 0, 0));
    code.push(add_x(21, 19, 16)); // x21 = FB + header(16) = first pixel
    let total = (GUEST_FB_W * GUEST_FB_H) as u16;
    let half = total / 2;
    code.push(movz_x(22, total, 0));
    code.push(movz_x(23, half, 0));

    // Top half: colorX
    let loop_top = code.len() as i32;
    code.push(str_w(9, 21, 0));
    code.push(add_x(21, 21, 4));
    code.push(add_x(20, 20, 1));
    code.push(cmp_x(20, 23));
    code.push(b_cond(COND_LT, loop_top - code.len() as i32));

    // Bottom half: colorY
    let loop_bot = code.len() as i32;
    code.push(str_w(10, 21, 0));
    code.push(add_x(21, 21, 4));
    code.push(add_x(20, 20, 1));
    code.push(cmp_x(20, 22));
    code.push(b_cond(COND_LT, loop_bot - code.len() as i32));

    // --- console output -------------------------------------------------
    // SYS_WRITE(msg) then SYS_EXIT. The message address is patched in
    // after the whole code vector is known (msg follows the code).
    code.push(movz_x(0, 0, 0)); // x0 = SYS_WRITE
    let x1_pos = code.len();
    // Placeholder: USER_BASE-derived addresses always lower to
    // MOVZ + 2×MOVK (3 words); patched below once msg_addr is known.
    for _ in 0..3 {
        code.push(movz_x(1, 0, 0));
    }
    code.push(movz_x(2, msg.len() as u16, 0)); // x2 = len
    code.push(svc());
    code.push(movz_x(0, 7, 0)); // x0 = SYS_EXIT
    code.push(svc());
    code.push(brk());

    let msg_addr = USER_BASE + code.len() as u64 * 4;
    let mut patch = Vec::new();
    load_const(&mut patch, 1, msg_addr);
    debug_assert_eq!(patch.len(), 3);
    code.splice(x1_pos..x1_pos + 3, patch);

    // Message bytes after code, 4-byte aligned.
    let mut bytes: Vec<u8> = Vec::new();
    for w in &code {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes.extend_from_slice(msg);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new("games/minimal");
    fs::create_dir_all(dir)?;

    let payload = guest_payload(b"Hello, xHoly!\n");

    // XEX2 little-endian header (scaffold layout documented in
    // loader/xex_parser.rs — retail Xbox One XEX has more optional
    // headers; this is the minimal subset our loader understands).
    let mut xex = vec![0u8; HEADER_SIZE];
    xex[0..4].copy_from_slice(b"XEX2");
    put32(&mut xex, 0x04, 0x0000_0001); // module_flags: executable
    put32(&mut xex, 0x08, HEADER_SIZE as u32);
    put32(&mut xex, 0x0C, 0); // reserved
    put32(&mut xex, 0x10, 0x6468_0000); // timestamp (fixed, reproducible)
    put32(&mut xex, 0x14, 0); // media id
                              // Entry point = image base (code starts at payload offset 0).
    put32(&mut xex, 0x104, IMAGE_BASE);
    put32(&mut xex, 0x108, IMAGE_BASE);
    put32(&mut xex, 0x10C, payload.len() as u32); // image size
                                                  // Title id quad at 0x184.
    put32(&mut xex, 0x184, u32::from_le_bytes(*b"XHOL"));
    put32(&mut xex, 0x188, u32::from_le_bytes(*b"YMIN"));
    put32(&mut xex, 0x18C, u32::from_le_bytes(*b"IMAL"));
    put32(&mut xex, 0x190, 1);

    xex.extend_from_slice(&payload);
    fs::write(dir.join("default.xex"), &xex)?;

    let config = r#"<?xml version="1.0" encoding="utf-8"?>
<GameInfo>
  <TitleID>XHOLYMINIMAL01</TitleID>
  <Name>xHoly Minimal Guest</Name>
  <Studio>xHoly Research</Studio>
  <Path>/default.xex</Path>
  <ReleaseDate>2026-01-01</ReleaseDate>
</GameInfo>
"#;
    fs::write(dir.join("xboxconfig.xml"), config)?;

    println!(
        "wrote {} ({} bytes header+payload, {} payload bytes)",
        dir.join("default.xex").display(),
        xex.len(),
        payload.len()
    );
    Ok(())
}

fn put32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
