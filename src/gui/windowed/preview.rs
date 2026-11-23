//! Game preview thumbnails for the windowed GUI.

use crate::gui::preview::{render_preview, PREVIEW_HEIGHT, PREVIEW_WIDTH};
use crate::gui::GameListItem;

/// Render a game preview at `(width, height)` RGBA8.
///
/// Uses the Phase-4 software preview card (trans-flag accents on dark)
/// scaled nearest-neighbor to the requested size. A hardware path can
/// be added later by routing through `core::gpu::render_text`.
pub fn render_game_preview(item: &GameListItem, width: u32, height: u32) -> Vec<u8> {
    let src = render_preview(item);
    scale_rgba(
        &src,
        PREVIEW_WIDTH as usize,
        PREVIEW_HEIGHT as usize,
        width as usize,
        height as usize,
    )
}

/// Nearest-neighbor RGBA scale.
pub fn scale_rgba(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = y * sh / dh.max(1);
        for x in 0..dw {
            let sx = x * sw / dw.max(1);
            let si = (sy * sw + sx) * 4;
            let di = (y * dw + x) * 4;
            if si + 4 <= src.len() {
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn preview_scales_to_requested_size() {
        let item = GameListItem {
            title: "TEST".into(),
            path: PathBuf::from("t"),
            cover_art: None,
            region: "NTSC-U".into(),
            discs: 1,
            release_date: None,
            is_multi_disc: false,
            last_played: None,
        };
        let px = render_game_preview(&item, 160, 90);
        assert_eq!(px.len(), 160 * 90 * 4);
    }
}
