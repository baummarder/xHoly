//! Software UI compositor for the windowed GUI.
//!
//! Draws the sidebar (game library), toolbar (transport buttons), info
//! panel (FPS/CPU/GPU/RAM) and preview area into an RGBA8 framebuffer.
//! Text uses a built-in 5×7 bitmap font — deterministic and dependency-
//! free, consistent with the Phase-2 software rasterizer approach.

use super::preview::render_game_preview;
use super::state::WindowedGuiState;
use crate::gui::GameListItem;

/// Framebuffer pixel = RGBA8.
pub type Framebuffer = Vec<u8>;

const BG: [u8; 4] = [0x1A, 0x1A, 0x1A, 0xFF];
const PANEL: [u8; 4] = [0x26, 0x26, 0x26, 0xFF];
const PANEL_ALT: [u8; 4] = [0x30, 0x30, 0x30, 0xFF];
const GOLD: [u8; 4] = [0xFF, 0xD7, 0x00, 0xFF];
const DIM_GOLD: [u8; 4] = [0x8A, 0x72, 0x00, 0xFF];
const TEXT: [u8; 4] = [0xE8, 0xE8, 0xE8, 0xFF];
const GRAY: [u8; 4] = [0x88, 0x88, 0x88, 0xFF];
const RED: [u8; 4] = [0xD7, 0x32, 0x32, 0xFF];
const GREEN: [u8; 4] = [0x4C, 0xC2, 0x6B, 0xFF];

/// Layout rectangles (in framebuffer pixels) for hit-testing.
#[derive(Debug, Clone, Default)]
pub struct UiLayout {
    /// Toolbar button rects paired with their labels.
    pub buttons: Vec<(Rect, &'static str)>,
    /// Sidebar game-row rects paired with game indices.
    pub rows: Vec<(Rect, usize)>,
    /// Bounds of the preview area.
    pub preview: Rect,
}

/// Integer rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Whether point (x, y) is inside.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Composite a full GUI frame: preview + sidebar + toolbar + info panel.
pub fn composite(state: &WindowedGuiState, width: u32, height: u32) -> (Framebuffer, UiLayout) {
    let (w, h) = (width as usize, height as usize);
    let mut fb = vec![0u8; w * h * 4];
    for p in fb.as_chunks_mut::<4>().0 {
        p.copy_from_slice(&BG);
    }

    let mut layout = UiLayout::default();

    let toolbar_h = 34u32;
    // Adaptive panel widths: never exceed ~30% of the framebuffer each,
    // so tiny windows/headless test sizes still lay out correctly.
    let sidebar_w = 230u32.min(w as u32 / 4).max(80);
    let info_w = 190u32.min(w as u32 / 4).max(80);

    // Toolbar
    fill_rect(&mut fb, w as u32, 0, 0, w as u32, toolbar_h, PANEL);
    draw_text(&mut fb, w as u32, h as u32, 12, 10, "xHoly", GOLD, 2);
    layout.buttons = render_toolbar(&mut fb, w as u32, state, toolbar_h);

    // Sidebar background + game list
    let list_top = toolbar_h;
    let list_bottom = h as u32;
    fill_rect(
        &mut fb,
        w as u32,
        0,
        list_top,
        sidebar_w,
        list_bottom.saturating_sub(list_top),
        PANEL,
    );
    layout.rows = render_sidebar(&mut fb, w as u32, h as u32, state, sidebar_w, list_top);

    // Info panel (right edge)
    let info_x = w as u32 - info_w;
    fill_rect(
        &mut fb,
        w as u32,
        info_x,
        list_top,
        info_w,
        h as u32 - list_top,
        PANEL,
    );
    render_info_panel(&mut fb, w as u32, h as u32, state, info_x, list_top, info_w);

    // Preview area (center)
    let prev_x = sidebar_w;
    let prev_y = list_top;
    let prev_w = info_x.saturating_sub(sidebar_w);
    let prev_h = h as u32 - list_top;
    layout.preview = Rect {
        x: prev_x,
        y: prev_y,
        w: prev_w,
        h: prev_h,
    };
    render_preview_area(&mut fb, w as u32, h as u32, state, &layout.preview);

    // Fallback-renderer warning banner (under the toolbar).
    if !state.software_message.is_empty() {
        let banner_h = 18u32;
        fill_rect(
            &mut fb,
            w as u32,
            0,
            toolbar_h,
            w as u32,
            banner_h,
            [0x4A, 0x1A, 0x1A, 0xFF],
        );
        draw_text(
            &mut fb,
            w as u32,
            h as u32,
            12,
            toolbar_h + 5,
            &truncate(&state.software_message.to_uppercase(), 90),
            [0xFF, 0xB0, 0xB0, 0xFF],
            1,
        );
    }

    // Settings overlay (toggled with O).
    if state.settings_open {
        render_settings_panel(&mut fb, w as u32, h as u32, state);
    }

    (fb, layout)
}

/// Centered settings panel: resolution, FPS limit, vsync, fullscreen,
/// theme, GPU API. Read-only display — values come from CLI/config.
fn render_settings_panel(fb: &mut Framebuffer, fb_w: u32, fb_h: u32, state: &WindowedGuiState) {
    let pw = 340u32;
    let ph = 220u32;
    let px = fb_w.saturating_sub(pw) / 2;
    let py = fb_h.saturating_sub(ph) / 2;

    // Dim the background.
    for y in 0..fb_h {
        for x in 0..fb_w {
            let i = (y as usize * fb_w as usize + x as usize) * 4;
            fb[i] = fb[i].saturating_mul(2) / 3;
            fb[i + 1] = fb[i + 1].saturating_mul(2) / 3;
            fb[i + 2] = fb[i + 2].saturating_mul(2) / 3;
        }
    }

    fill_rect(fb, fb_w, px, py, pw, ph, PANEL);
    stroke_rect(fb, fb_w, px, py, pw, ph, GOLD);
    draw_text(fb, fb_w, fb_h, px + 14, py + 10, "SETTINGS", GOLD, 2);

    let mut y = py + 40;
    let rows: [(String, String); 7] = [
        (
            "RESOLUTION".into(),
            format!("{}X{}", state.resolution.0, state.resolution.1),
        ),
        (
            "FPS LIMIT".into(),
            state
                .fps_limit
                .map(|f| f.to_string())
                .unwrap_or_else(|| "UNLIMITED".into()),
        ),
        (
            "VSYNC".into(),
            if state.vsync {
                "ON".into()
            } else {
                "OFF".into()
            },
        ),
        (
            "FULLSCREEN".into(),
            if state.fullscreen {
                "ON".into()
            } else {
                "OFF".into()
            },
        ),
        ("THEME".into(), format!("{:?}", state.theme).to_uppercase()),
        ("GPU API".into(), state.gpu_label.to_uppercase()),
        (
            "SOFTWARE".into(),
            if state.software_mode {
                "YES".into()
            } else {
                "NO".into()
            },
        ),
    ];
    for (label, value) in rows {
        draw_text(fb, fb_w, fb_h, px + 14, y, &label, GRAY, 1);
        draw_text(
            fb,
            fb_w,
            fb_h,
            (px + pw).saturating_sub(14 + text_width(&value, 1)),
            y,
            &value,
            GOLD,
            1,
        );
        y += 20;
    }
    draw_text(
        fb,
        fb_w,
        fb_h,
        px + 14,
        y + 6,
        "PRESS O TO CLOSE",
        DIM_GOLD,
        1,
    );
}

fn render_toolbar(
    fb: &mut Framebuffer,
    fb_w: u32,
    state: &WindowedGuiState,
    toolbar_h: u32,
) -> Vec<(Rect, &'static str)> {
    let labels: [(&'static str, [u8; 4]); 6] = [
        (
            "PLAY",
            if state.running && !state.paused {
                GREEN
            } else {
                GOLD
            },
        ),
        ("PAUSE", if state.paused { RED } else { TEXT }),
        ("EJECT", GRAY),
        ("SAVE", GOLD),
        ("LOAD", GOLD),
        (
            "CHEATS",
            if state.cheat_engine_enabled {
                RED
            } else {
                DIM_GOLD
            },
        ),
    ];
    let mut buttons = Vec::new();
    let fh = fb_h(fb, fb_w);
    let mut x = 250u32;
    let y = 6u32;
    let bw = 74u32;
    let bh = toolbar_h - 12;
    for (label, color) in labels {
        if x + bw > fb_w - 10 {
            break;
        }
        let r = Rect { x, y, w: bw, h: bh };
        fill_rect(fb, fb_w, x, y, bw, bh, PANEL_ALT);
        stroke_rect(fb, fb_w, x, y, bw, bh, color);
        let tw = text_width(label, 1);
        draw_text(
            fb,
            fb_w,
            fh,
            x + (bw - tw) / 2,
            y + (bh - 7) / 2,
            label,
            color,
            1,
        );
        buttons.push((r, label));
        x += bw + 8;
    }

    // Status text right-aligned in toolbar
    let status = if state.paused {
        "PAUSED"
    } else if state.running {
        "RUNNING"
    } else {
        "IDLE"
    };
    let status_color = if state.paused {
        RED
    } else if state.running {
        GREEN
    } else {
        GRAY
    };
    let sw = text_width(status, 1);
    draw_text(
        fb,
        fb_w,
        fh,
        fb_w.saturating_sub(sw + 12),
        12,
        status,
        status_color,
        1,
    );
    buttons
}

fn fb_h(fb: &[u8], w: u32) -> u32 {
    (fb.len() / 4 / w.max(1) as usize) as u32
}

fn render_sidebar(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    state: &WindowedGuiState,
    sidebar_w: u32,
    top: u32,
) -> Vec<(Rect, usize)> {
    draw_text(fb, fb_w, fb_h, 12, top + 8, "LIBRARY", DIM_GOLD, 1);
    let mut rows = Vec::new();
    let row_h = 26u32;
    let mut y = top + 26;
    for (i, g) in state.games.iter().enumerate() {
        if y + row_h > fb_h {
            break;
        }
        let r = Rect {
            x: 6,
            y,
            w: sidebar_w - 12,
            h: row_h - 4,
        };
        if i == state.selected_game {
            fill_rect(fb, fb_w, r.x, r.y, r.w, r.h, [0x3A, 0x2E, 0x00, 0xFF]);
            stroke_rect(fb, fb_w, r.x, r.y, r.w, r.h, GOLD);
        } else {
            fill_rect(fb, fb_w, r.x, r.y, r.w, r.h, PANEL_ALT);
        }
        let label = truncate(&g.title.to_uppercase(), 26);
        draw_text(
            fb,
            fb_w,
            fb_h,
            r.x + 8,
            r.y + 8,
            &label,
            if i == state.selected_game { GOLD } else { TEXT },
            1,
        );
        rows.push((r, i));
        y += row_h;
    }
    if state.games.is_empty() {
        draw_text(fb, fb_w, fb_h, 14, top + 60, "NO GAMES FOUND", GRAY, 1);
        draw_text(fb, fb_w, fb_h, 14, top + 76, "ADD .ISO FILES", GRAY, 1);
        draw_text(fb, fb_w, fb_h, 14, top + 90, "OR USE --MOUNT", GRAY, 1);
    }
    rows
}

fn render_info_panel(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    state: &WindowedGuiState,
    x: u32,
    top: u32,
    w: u32,
) {
    draw_text(fb, fb_w, fb_h, x + 12, top + 10, "PERFORMANCE", DIM_GOLD, 1);
    if !state.gpu_label.is_empty() {
        draw_text(fb, fb_w, fb_h, x + 12, top + 22, "GPU API", GRAY, 1);
        draw_text(
            fb,
            fb_w,
            fb_h,
            x + 12,
            top + 34,
            &truncate(&state.gpu_label.to_uppercase(), 24),
            GOLD,
            1,
        );
    }
    let mut y = top + 56;
    let rows: [(String, String); 5] = [
        ("FPS".into(), format!("{:.0}", state.fps)),
        ("CPU".into(), format!("{:.0}%", state.cpu_usage * 100.0)),
        ("GPU".into(), format!("{:.0}%", state.gpu_usage * 100.0)),
        (
            "RAM".into(),
            format!("{} MB", state.memory_usage / 1_048_576),
        ),
        ("SLOTS".into(), state.save_slots.len().to_string()),
    ];
    for (label, value) in rows {
        draw_text(fb, fb_w, fb_h, x + 12, y, &label, GRAY, 1);
        draw_text(
            fb,
            fb_w,
            fb_h,
            (x + w).saturating_sub(12 + text_width(&value, 1)),
            y,
            &value,
            GOLD,
            1,
        );
        y += 22;
    }

    y += 14;
    draw_text(fb, fb_w, fb_h, x + 12, y, "CONTROLS", DIM_GOLD, 1);
    y += 22;
    for line in [
        "ESC  PAUSE",
        "F1   SHOT",
        "F5   SAVE",
        "F9   LOAD",
        "E    EJECT",
        "C    CHEAT",
        "Q    QUIT",
    ] {
        draw_text(fb, fb_w, fb_h, x + 12, y, line, TEXT, 1);
        y += 18;
    }
}

fn render_preview_area(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    fill_rect(
        fb,
        fb_w,
        area.x,
        area.y,
        area.w,
        area.h,
        [0x14, 0x14, 0x14, 0xFF],
    );
    let Some(game) = state.selected() else {
        // Empty-state logo-ish placeholder
        let msg = "SELECT A GAME";
        draw_text(
            fb,
            fb_w,
            fb_h,
            area.x + (area.w.saturating_sub(text_width(msg, 3))) / 2,
            area.y + area.h / 2,
            msg,
            DIM_GOLD,
            3,
        );
        let sub = "FROM THE LIBRARY SIDEBAR";
        draw_text(
            fb,
            fb_w,
            fb_h,
            area.x + (area.w.saturating_sub(text_width(sub, 1))) / 2,
            area.y + area.h / 2 + 40,
            sub,
            GRAY,
            1,
        );
        return;
    };

    // Render the game preview and blit centered (letterboxed).
    let (pw, ph) = fit_size(state.preview_size.0, state.preview_size.1, area.w, area.h);
    let px = render_game_preview(game, pw, ph);
    let ox = area.x + (area.w.saturating_sub(pw)) / 2;
    let oy = area.y + (area.h.saturating_sub(ph)) / 2;
    blit_rgba(fb, fb_w, fb_h, &px, pw as usize, ph as usize, ox, oy);

    // Title bar under preview
    let title = truncate(&game.title.to_uppercase(), 40);
    draw_text(
        fb,
        fb_w,
        fb_h,
        area.x + 16,
        area.y + area.h - 30,
        &title,
        GOLD,
        2,
    );
    let meta = format!(
        "{} - {} DISC{}",
        game.region,
        game.discs,
        if game.discs > 1 { "S" } else { "" }
    );
    draw_text(
        fb,
        fb_w,
        fb_h,
        area.x + 16,
        area.y + area.h - 14,
        &meta,
        GRAY,
        1,
    );
}

/// Largest (w,h) with aspect ratio of (aw,ah) fitting inside (max_w,max_h).
fn fit_size(aw: u32, ah: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if aw == 0 || ah == 0 || max_w == 0 || max_h == 0 {
        return (max_w, max_h);
    }
    let scale = (max_w as f64 / aw as f64).min(max_h as f64 / ah as f64);
    (
        ((aw as f64 * scale) as u32).max(1),
        ((ah as f64 * scale) as u32).max(1),
    )
}

#[allow(clippy::too_many_arguments)]
fn blit_rgba(
    dst: &mut Framebuffer,
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: usize,
    src_h: usize,
    ox: u32,
    oy: u32,
) {
    for sy in 0..src_h {
        for sx in 0..src_w {
            let si = (sy * src_w + sx) * 4;
            if si + 4 > src.len() {
                break;
            }
            let x = ox as usize + sx;
            let y = oy as usize + sy;
            if x < dst_w as usize && y < dst_h as usize {
                let di = (y * dst_w as usize + x) * 4;
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

fn fill_rect(fb: &mut Framebuffer, fb_w: u32, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    for yy in y..y.saturating_add(h) {
        for xx in x..x.saturating_add(w) {
            if xx < fb_w && (yy as usize) * (fb_w as usize) + (xx as usize) < fb.len() / 4 {
                let i = (yy as usize * fb_w as usize + xx as usize) * 4;
                fb[i..i + 4].copy_from_slice(&color);
            }
        }
    }
}

fn stroke_rect(fb: &mut Framebuffer, fb_w: u32, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    fill_rect(fb, fb_w, x, y, w, 1, color);
    fill_rect(fb, fb_w, x, y + h - 1, w, 1, color);
    fill_rect(fb, fb_w, x, y, 1, h, color);
    fill_rect(fb, fb_w, x + w - 1, y, 1, h, color);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('.');
        out
    }
}

// ---------------------------------------------------------------------------
// 5x7 bitmap font
// ---------------------------------------------------------------------------

/// Draw text at (x, y) top-left, `scale` = integer pixel scale.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    x: u32,
    y: u32,
    text: &str,
    color: [u8; 4],
    scale: u32,
) {
    let mut cx = x;
    for ch in text.chars() {
        let glyph = glyph(ch);
        for (col, &bits) in glyph.iter().enumerate() {
            for row in 0..7u32 {
                if bits & (1 << row) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = cx + col as u32 * scale + sx;
                            let py = y + row * scale + sy;
                            if px < fb_w && py < fb_h {
                                let i = (py as usize * fb_w as usize + px as usize) * 4;
                                fb[i..i + 4].copy_from_slice(&color);
                            }
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

/// Width of `text` in pixels at the given scale.
pub fn text_width(text: &str, scale: u32) -> u32 {
    text.chars().count() as u32 * 6 * scale
}

/// 5x7 glyph column bitmasks (bit `row` = pixel row 0..6). Unknown → blank.
fn glyph(ch: char) -> [u8; 5] {
    match ch {
        ' ' => [0; 5],
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
        'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
        'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        'W' => [0x3F, 0x40, 0x38, 0x40, 0x3F],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        ',' => [0x00, 0x50, 0x30, 0x00, 0x00],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '_' => [0x40, 0x40, 0x40, 0x40, 0x40],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        '%' => [0x23, 0x13, 0x08, 0x64, 0x62],
        '(' => [0x00, 0x1C, 0x22, 0x41, 0x00],
        ')' => [0x00, 0x41, 0x22, 0x1C, 0x00],
        '[' => [0x00, 0x7F, 0x41, 0x41, 0x00],
        ']' => [0x00, 0x41, 0x41, 0x7F, 0x00],
        '<' => [0x08, 0x14, 0x22, 0x41, 0x00],
        '>' => [0x00, 0x41, 0x22, 0x14, 0x08],
        '=' => [0x14, 0x14, 0x14, 0x14, 0x14],
        '+' => [0x08, 0x08, 0x3E, 0x08, 0x08],
        '*' => [0x14, 0x08, 0x3E, 0x08, 0x14],
        '!' => [0x00, 0x00, 0x5F, 0x00, 0x00],
        '?' => [0x02, 0x01, 0x51, 0x09, 0x06],
        c if c.is_ascii_lowercase() => glyph(c.to_ascii_uppercase()),
        _ => [0; 5],
    }
}

// Keep GameListItem referenced for doc links.
#[allow(unused)]
fn _doc(_: Option<&GameListItem>) {}
