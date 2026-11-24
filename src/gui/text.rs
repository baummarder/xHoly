//! Modern scaled text rendering for the software UI compositor.
//!
//! Replaces the old 5x7 bitmap glyphs with the embedded Inter font (SIL
//! Open Font License, see `assets/fonts/OFL-Inter.txt`). Text is drawn
//! directly into RGBA8 framebuffers so it also works headless and on
//! both Windows and Linux without a native font API.

use ab_glyph::{point, Font, FontVec, Glyph, PxScale, ScaleFont};
use std::sync::OnceLock;

/// Embedded Inter font (OFL).
const INTER_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter.ttf");

static FONT: OnceLock<FontVec> = OnceLock::new();

fn font() -> &'static FontVec {
    FONT.get_or_init(|| {
        FontVec::try_from_vec(INTER_TTF.to_vec()).expect("embedded Inter font must be valid")
    })
}

/// Font size in pixels for a compositor `scale` value.
pub fn px_for_scale(scale: u32) -> f32 {
    (13.0 * scale as f32).clamp(9.0, 96.0)
}

/// Width of `text` in pixels at the given compositor scale.
pub fn text_width(text: &str, scale: u32) -> u32 {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(px_for_scale(scale)));
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum::<f32>()
        .ceil() as u32
}

/// Draw `text` at `(x, y)` top-left into an RGBA8 framebuffer.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    fb: &mut [u8],
    fb_w: u32,
    fb_h: u32,
    x: u32,
    y: u32,
    text: &str,
    color: [u8; 4],
    scale: u32,
) {
    if fb_w == 0 || fb_h == 0 || text.is_empty() {
        return;
    }
    let font = font();
    let px_scale = PxScale::from(px_for_scale(scale));
    let scaled = font.as_scaled(px_scale);
    let baseline = y as f32 + scaled.ascent();
    let mut pen_x = x as f32;

    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let glyph = Glyph {
            id,
            scale: px_scale,
            position: point(pen_x, baseline),
        };
        if let Some(outline) = scaled.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                if coverage <= 0.0 {
                    return;
                }
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px as u32 >= fb_w || py as u32 >= fb_h {
                    return;
                }
                let i = (py as usize * fb_w as usize + px as usize) * 4;
                if i + 4 > fb.len() {
                    return;
                }
                let alpha = coverage.clamp(0.0, 1.0);
                for c in 0..3 {
                    let dst = fb[i + c] as f32;
                    fb[i + c] = (dst * (1.0 - alpha) + color[c] as f32 * alpha).round() as u8;
                }
                fb[i + 3] = 255;
            });
        }
        pen_x += scaled.h_advance(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_font_is_embedded_and_measurable() {
        assert!(text_width("xHoly", 1) > 20);
        assert!(text_width("xHoly", 2) > text_width("xHoly", 1));
    }

    #[test]
    fn draws_visible_scaled_glyphs() {
        let (w, h) = (160, 40);
        let mut fb = vec![0u8; w * h * 4];
        draw_text(
            &mut fb,
            w as u32,
            h as u32,
            2,
            2,
            "xHOLY",
            [255, 255, 255, 255],
            1,
        );
        assert!(fb.as_chunks::<4>().0.iter().any(|p| p[0] > 32));
    }
}
