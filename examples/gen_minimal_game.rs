//! Regenerates the bundled minimal guest game (`games/minimal/`).
//!
//! Produces a real on-disk XEX2 image: a little-endian `XEX2` header
//! (Xbox-One-style scaffold layout — see `core::loader::xex_parser`)
//! followed by an AArch64 payload that writes `Hello, xHoly!` through
//! the kernel stub's SYS_WRITE and exits via SYS_EXIT.
//!
//! Run: `cargo run --example gen_minimal_game`

use std::fs;
use std::path::Path;

/// XEX2 fixed header size used by our minimal format (base fields +
/// room for the title-id quad at 0x184).
const HEADER_SIZE: usize = 0x1A0;
/// Guest VA the image is linked at (mapped to USER_BASE by the loader).
const IMAGE_BASE: u32 = 0x8200_0000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new("games/minimal");
    fs::create_dir_all(dir)?;

    // ARM64 payload: SYS_WRITE("Hello, xHoly!\n") then SYS_EXIT.
    let payload = xholy::core::execution::hello_world_program(b"Hello, xHoly!\n");

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
                                                  // Title id quad at 0x184: "XHOL" + "00001" style, little-endian u32s.
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
        "wrote {} ({} bytes header+payload) and xboxconfig.xml",
        dir.join("default.xex").display(),
        xex.len()
    );
    Ok(())
}

fn put32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
