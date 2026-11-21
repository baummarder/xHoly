//! Screenshot capture: framebuffer readback → image file.
//!
//! Writes uncompressed BMP (no external deps) for the scaffold; PNG
//! encoding can be added behind a feature flag.

use std::io::Write;
use std::path::Path;

/// Write an RGBA8 framebuffer to a BMP file at `path`.
pub fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    // Despite the name (kept for API stability), this writes BMP v3.
    let mut f = std::fs::File::create(path)?;
    let row_size = (width * 3 + 3) & !3; // 24-bit rows padded to 4 bytes
    let pixel_data_size = row_size * height;
    let file_size = 54 + pixel_data_size;

    f.write_all(b"BM")?;
    f.write_all(&file_size.to_le_bytes())?;
    f.write_all(&[0; 4])?; // reserved
    f.write_all(&54u32.to_le_bytes())?; // pixel data offset
    f.write_all(&40u32.to_le_bytes())?; // DIB header size
    f.write_all(&width.to_le_bytes())?;
    f.write_all(&height.to_le_bytes())?; // positive = bottom-up
    f.write_all(&1u16.to_le_bytes())?; // planes
    f.write_all(&24u16.to_le_bytes())?; // bpp
    f.write_all(&0u32.to_le_bytes())?; // no compression
    f.write_all(&pixel_data_size.to_le_bytes())?;
    f.write_all(&2835i32.to_le_bytes())?; // x ppm
    f.write_all(&2835i32.to_le_bytes())?; // y ppm
    f.write_all(&0u32.to_le_bytes())?; // colors used
    f.write_all(&0u32.to_le_bytes())?; // important colors

    let mut row = vec![0u8; row_size as usize];
    for y in (0..height as usize).rev() {
        for x in 0..width as usize {
            let src = (y * width as usize + x) * 4;
            let dst = x * 3;
            row[dst] = rgba[src + 2]; // B
            row[dst + 1] = rgba[src + 1]; // G
            row[dst + 2] = rgba[src]; // R
        }
        f.write_all(&row)?;
    }
    Ok(())
}
