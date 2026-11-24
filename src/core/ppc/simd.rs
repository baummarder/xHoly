//! SIMD pixel helpers (Phase 24).
//!
//! Xbox 360 uses VMX/AltiVec (`vector unsigned char`, 16 bytes), while
//! Series S|X uses ARM NEON. Both are 128-bit integer vector units for
//! the pixel work we model here, so a single little-endian `u8x16`
//! representation is shared. This is a **real** implementation of the
//! arithmetic — not a stub — but it is a host-side model, not guest
//! VMX/NEON instruction decoding.

/// 128-bit unsigned byte vector (VMX / NEON `uint8x16_t`).
pub type U8x16 = [u8; 16];

/// Packed 16-bit lanes (VMX `vector unsigned short`).
pub type U16x8 = [u16; 8];

/// Saturating unsigned byte add (`vaddubs`/`vqadd_u8`).
pub fn add_sat_u8(a: U8x16, b: U8x16) -> U8x16 {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = a[i].saturating_add(b[i]);
    }
    out
}

/// Saturating unsigned byte subtract (`vsububs`/`vqsub_u8`).
pub fn sub_sat_u8(a: U8x16, b: U8x16) -> U8x16 {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = a[i].saturating_sub(b[i]);
    }
    out
}

/// Averaging add (`vavgsb` unsigned form / `vhaddu`), rounding down.
pub fn avg_u8(a: U8x16, b: U8x16) -> U8x16 {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = ((a[i] as u16 + b[i] as u16) >> 1) as u8;
    }
    out
}

/// Widen 8 bytes to 16-bit lanes.
pub fn unpack_u8_to_u16(bytes: [u8; 8]) -> U16x8 {
    let mut out = [0u16; 8];
    for (i, b) in bytes.iter().enumerate() {
        out[i] = *b as u16;
    }
    out
}

/// Alpha-blend two RGBA pixels in-place using one 16-byte vector.
/// `dst = src * a + dst * (1-a)` with `a` in 0..=255 per pixel.
pub fn blend_rgba(dst: U8x16, src: U8x16, alpha: u8) -> U8x16 {
    let a = alpha as u16;
    let mut out = [0u8; 16];
    for i in 0..16 {
        let d = dst[i] as u16;
        let s = src[i] as u16;
        out[i] = ((s * a + d * (255 - a) + 127) / 255) as u8;
    }
    out
}

/// Pack four RGBA8 pixels (16 bytes) into two 565 RGB words.
pub fn pack_rgb565(pixels: U8x16) -> U16x8 {
    let mut out = [0u16; 8];
    for (p, word) in out.iter_mut().enumerate().take(4) {
        let i = p * 4;
        let r = (pixels[i] >> 3) as u16;
        let g = (pixels[i + 1] >> 2) as u16;
        let b = (pixels[i + 2] >> 3) as u16;
        *word = (r << 11) | (g << 5) | b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_sub_saturate() {
        let hi = [255u8; 16];
        let one = [1u8; 16];
        assert_eq!(add_sat_u8(hi, one), hi);
        assert_eq!(sub_sat_u8([0u8; 16], one), [0u8; 16]);
    }

    #[test]
    fn averaging_is_exact() {
        assert_eq!(avg_u8([10u8; 16], [20u8; 16]), [15u8; 16]);
    }

    #[test]
    fn blend_half_is_average() {
        let a = [0u8; 16];
        let b = [200u8; 16];
        assert_eq!(blend_rgba(a, b, 128), [100u8; 16]);
    }

    #[test]
    fn rgb565_rounds_trivial_colors() {
        let white = [255u8, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let packed = pack_rgb565(white);
        assert_eq!(packed[0], 0xFFFF);
        assert_eq!(packed[1], 0x0000);
    }

    #[test]
    fn unpack_widens_without_loss() {
        assert_eq!(
            unpack_u8_to_u16([1, 2, 3, 4, 5, 6, 7, 8]),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }
}
