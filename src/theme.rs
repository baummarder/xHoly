//! xHoly pride palette (Phase 24).
//!
//! The interface uses the **transgender flag** as its primary accent
//! (light blue, pink, white, pink, light blue). Individual controls are
//! tinted with other LGBTQ+ flag palettes so the toolbar reads as a
//! small, deliberate flag strip rather than arbitrary colours.
//!
//! Colours are plain RGBA8 to stay usable by both the software UI
//! compositor and the wgpu preview path without pulling in a colour
//! crate.

/// One RGBA8 colour.
pub type Rgba = [u8; 4];

// --- Primary: transgender flag ---------------------------------------------

/// Trans flag light blue `#5BCEFA`.
pub const TRANS_BLUE: Rgba = [0x5B, 0xCE, 0xFA, 0xFF];
/// Trans flag pink `#F5A9B8`.
pub const TRANS_PINK: Rgba = [0xF5, 0xA9, 0xB8, 0xFF];
/// Trans flag white.
pub const TRANS_WHITE: Rgba = [0xFF, 0xFF, 0xFF, 0xFF];
/// The full five-stripe trans flag.
pub const TRANS_FLAG: [Rgba; 5] = [TRANS_BLUE, TRANS_PINK, TRANS_WHITE, TRANS_PINK, TRANS_BLUE];

// --- Secondary: other LGBTQ+ flags -----------------------------------------

/// Rainbow (gay pride) six-stripe palette.
pub const RAINBOW: [Rgba; 6] = [
    [0xFF, 0x00, 0x18, 0xFF], // red
    [0xFF, 0xA5, 0x2C, 0xFF], // orange
    [0xFF, 0xFF, 0x41, 0xFF], // yellow
    [0x00, 0x80, 0x18, 0xFF], // green
    [0x00, 0x00, 0xF9, 0xFF], // blue
    [0x86, 0x00, 0x7D, 0xFF], // purple
];
/// Bisexual flag palette.
pub const BISEXUAL: [Rgba; 3] = [
    [0xD6, 0x02, 0x70, 0xFF],
    [0x9B, 0x4F, 0x96, 0xFF],
    [0x00, 0x38, 0xA8, 0xFF],
];
/// Non-binary flag palette.
pub const NONBINARY: [Rgba; 4] = [
    [0xFC, 0xF4, 0x34, 0xFF],
    TRANS_WHITE,
    [0x9C, 0x59, 0xD1, 0xFF],
    [0x2C, 0x2C, 0x2C, 0xFF],
];
/// Asexual flag palette.
pub const ASEXUAL: [Rgba; 4] = [
    [0x00, 0x00, 0x00, 0xFF],
    [0xA3, 0xA3, 0xA3, 0xFF],
    TRANS_WHITE,
    [0x81, 0x00, 0x81, 0xFF],
];
/// Pansexual flag palette.
pub const PANSEXUAL: [Rgba; 3] = [
    [0xFF, 0x21, 0x8C, 0xFF],
    [0xFF, 0xD8, 0x00, 0xFF],
    [0x21, 0xB1, 0xFF, 0xFF],
];
/// Genderqueer flag palette.
pub const GENDERQUEER: [Rgba; 3] = [
    [0xB5, 0x7E, 0xDC, 0xFF],
    TRANS_WHITE,
    [0x4A, 0x81, 0x23, 0xFF],
];
/// Genderfluid flag palette.
pub const GENDERFLUID: [Rgba; 5] = [
    [0xFF, 0x75, 0xAE, 0xFF], // pink
    TRANS_WHITE,
    [0xC0, 0x9D, 0xE6, 0xFF], // lavender
    [0x3E, 0x3E, 0x3E, 0xFF], // black
    TRANS_BLUE,
];
/// Lesbian flag palette (used for the console accent).
pub const LESBIAN: [Rgba; 5] = [
    [0xFA, 0x5C, 0x2B, 0xFF],
    [0xFF, 0x9A, 0x56, 0xFF],
    TRANS_WHITE,
    [0xD4, 0x62, 0x9F, 0xFF],
    [0xA2, 0x00, 0x67, 0xFF],
];

// --- Neutral surfaces ------------------------------------------------------

/// Deep blue-black window background.
pub const BG: Rgba = [0x14, 0x16, 0x1C, 0xFF];
/// Raised panel surface.
pub const PANEL: Rgba = [0x1C, 0x1F, 0x27, 0xFF];
/// Alternating / button surface.
pub const PANEL_ALT: Rgba = [0x27, 0x2B, 0x36, 0xFF];
/// Inset preview well.
pub const INSET: Rgba = [0x10, 0x12, 0x18, 0xFF];
/// Bright body text.
pub const TEXT: Rgba = [0xF2, 0xF4, 0xF8, 0xFF];
/// Secondary text.
pub const MUTED: Rgba = [0x9A, 0xA1, 0xB2, 0xFF];
/// Section-label text.
pub const DIM: Rgba = [0x5C, 0x63, 0x75, 0xFF];

/// Toolbar button → flag palette. The trans flag is the primary action;
/// every other button carries a different LGBTQ+ flag.
pub const BUTTON_FLAGS: [(&str, &[Rgba]); 7] = [
    ("LAUNCH", &TRANS_FLAG), // primary brand action
    ("PLAY", &RAINBOW),      // rainbow pride
    ("PAUSE", &PANSEXUAL),   // pansexual pride
    ("EJECT", &LESBIAN),     // lesbian pride
    ("SAVE", &ASEXUAL),      // asexual pride
    ("LOAD", &GENDERFLUID),  // genderfluid pride
    ("CHEATS", &BISEXUAL),   // bisexual pride
];

/// Pick the palette registered for a toolbar label (falls back to trans).
pub fn button_flag(label: &str) -> &'static [Rgba] {
    BUTTON_FLAGS
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, flag)| *flag)
        .unwrap_or(&TRANS_FLAG)
}

/// One stripe of a flag at relative position `t` in `0.0..=1.0`.
pub fn stripe_at(flag: &[Rgba], t: f32) -> Rgba {
    if flag.is_empty() {
        return TRANS_BLUE;
    }
    let n = flag.len();
    let idx = (t.clamp(0.0, 1.0) * n as f32) as usize;
    flag[idx.min(n - 1)]
}

/// Smooth blue → pink → blue gradient for headers and selection bars.
pub fn trans_gradient(t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let (a, b, u) = if t < 0.5 {
        (TRANS_BLUE, TRANS_PINK, t * 2.0)
    } else {
        (TRANS_PINK, TRANS_BLUE, (t - 0.5) * 2.0)
    };
    mix(a, b, u)
}

/// Linear colour mix.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 4];
    for i in 0..4 {
        out[i] = (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8;
    }
    out
}

/// Multiply RGB by `factor` (used for pressed/hover shading).
pub fn shade(color: Rgba, factor: f32) -> Rgba {
    let mut out = color;
    for c in out.iter_mut().take(3) {
        *c = ((*c as f32) * factor).clamp(0.0, 255.0).round() as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trans_flag_has_five_stripes() {
        assert_eq!(
            TRANS_FLAG,
            [TRANS_BLUE, TRANS_PINK, TRANS_WHITE, TRANS_PINK, TRANS_BLUE]
        );
        assert_eq!(stripe_at(&TRANS_FLAG, 0.5), TRANS_WHITE);
    }

    #[test]
    fn every_toolbar_button_has_a_flag() {
        let labels = ["LAUNCH", "PLAY", "PAUSE", "EJECT", "SAVE", "LOAD", "CHEATS"];
        let mut palettes = std::collections::HashSet::new();
        for label in labels {
            let flag = button_flag(label);
            assert!(!flag.is_empty());
            palettes.insert(flag.as_ptr() as usize);
        }
        assert_eq!(
            palettes.len(),
            labels.len(),
            "buttons should use distinct flags"
        );
    }

    #[test]
    fn gradient_stays_within_trans_hues() {
        let start = trans_gradient(0.0);
        let mid = trans_gradient(0.5);
        let end = trans_gradient(1.0);
        assert_eq!(start, TRANS_BLUE);
        assert_eq!(mid, TRANS_PINK);
        assert_eq!(end, TRANS_BLUE);
    }
}
