//! Game preview renderer — draws a title-card-style RGBA frame for a
//! [`GameListItem`] using the Phase-2 `bytes_to_bitmap` bit-column font
//! (deterministic, no GPU or window required).

use super::GameListItem;
use crate::core::gpu::bytes_to_bitmap;

/// Default preview resolution.
pub const PREVIEW_WIDTH: u32 = 320;
pub const PREVIEW_HEIGHT: u32 = 180;

const BG: [u8; 4] = [0x1A, 0x1A, 0x1A, 0xFF];
const GOLD: [u8; 4] = [0xFF, 0xD7, 0x00, 0xFF];
const DIM_GOLD: [u8; 4] = [0x8A, 0x72, 0x00, 0xFF];

/// Render a preview frame (RGBA8, `PREVIEW_WIDTH`×`PREVIEW_HEIGHT`).
///
/// Layout: dark background, gold accent bar on top, centered title,
/// region/disc-count footer.
pub fn render_preview(item: &GameListItem) -> Vec<u8> {
    let (w, h) = (PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize);
    let mut px = vec![0u8; w * h * 4];
    for p in px.as_chunks_mut::<4>().0 {
        p.copy_from_slice(&BG);
    }

    // Gold accent bar at top
    for y in 0..4usize {
        for x in 0..w {
            let i = (y * w + x) * 4;
            px[i..i + 4].copy_from_slice(&GOLD);
        }
    }

    // Title text (uppercase, 2x scale → 16px tall)
    let title = item.title.to_uppercase();
    let (tw, th, bits) = bytes_to_bitmap(title.as_bytes());
    let scale = 2usize;
    let text_w = tw as usize * scale;
    let text_h = th as usize * scale;
    let ox = w.saturating_sub(text_w) / 2;
    let oy = (h / 2).saturating_sub(text_h / 2 + 4);
    blit_r8(
        &mut px,
        w,
        h,
        &bits,
        tw as usize,
        th as usize,
        scale,
        ox,
        oy,
        &GOLD,
    );

    // Footer metadata
    let meta = format!(
        "{} · {} DISC{}",
        item.region,
        item.discs,
        if item.discs > 1 { "S" } else { "" }
    );
    let (mw, mh, mbits) = bytes_to_bitmap(meta.as_bytes());
    let meta_w = mw as usize * scale;
    let meta_x = w.saturating_sub(meta_w) / 2;
    let meta_y = h - mh as usize * scale - 8;
    blit_r8(
        &mut px,
        w,
        h,
        &mbits,
        mw as usize,
        mh as usize,
        scale,
        meta_x,
        meta_y,
        &DIM_GOLD,
    );

    px
}

/// Blit an R8 bitmap (row-major, `bw`×`bh`) onto an RGBA buffer,
/// scaling each source pixel by `scale` and coloring with `color`.
#[allow(clippy::too_many_arguments)]
fn blit_r8(
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    src: &[u8],
    bw: usize,
    bh: usize,
    scale: usize,
    ox: usize,
    oy: usize,
    color: &[u8; 4],
) {
    for sy in 0..bh {
        for sx in 0..bw {
            if src[sy * bw + sx] == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let x = ox + sx * scale + dx;
                    let y = oy + sy * scale + dy;
                    if x < dst_w && y < dst_h {
                        let i = (y * dst_w + x) * 4;
                        dst[i..i + 4].copy_from_slice(color);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_item() -> GameListItem {
        GameListItem {
            title: "HALO INFINITE".into(),
            path: PathBuf::from("games/halo"),
            cover_art: None,
            region: "NTSC-U".into(),
            discs: 1,
            release_date: None,
            is_multi_disc: false,
            last_played: None,
        }
    }

    #[test]
    fn preview_has_expected_size() {
        let px = render_preview(&sample_item());
        assert_eq!(px.len(), (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize);
    }

    #[test]
    fn preview_has_gold_pixels() {
        let px = render_preview(&sample_item());
        let gold = px
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|c| c[..3] == GOLD[..3])
            .count();
        assert!(gold > 100, "expected gold pixels, got {gold}");
    }

    #[test]
    fn preview_background_is_dark() {
        let px = render_preview(&sample_item());
        let i = ((PREVIEW_HEIGHT as usize - 1) * PREVIEW_WIDTH as usize + PREVIEW_WIDTH as usize
            - 1)
            * 4;
        assert_eq!(&px[i..i + 3], &[0x1A, 0x1A, 0x1A]);
    }
}
