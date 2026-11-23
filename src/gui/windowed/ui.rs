//! Software UI compositor for the windowed GUI.
//!
//! Draws the sidebar (game library), toolbar (transport buttons), info
//! panel (FPS/CPU/GPU/RAM) and preview area into an RGBA8 framebuffer.
//! Text uses a built-in 5×7 bitmap font — deterministic and dependency-
//! free, consistent with the Phase-2 software rasterizer approach.

use super::preview::render_game_preview;
use super::state::WindowedGuiState;
use crate::gui::GameListItem;
use crate::theme::{self, Rgba};

/// Framebuffer pixel = RGBA8.
pub type Framebuffer = Vec<u8>;

// Pride palette (Phase 24): the trans flag is the primary accent.
const BG: Rgba = theme::BG;
const PANEL: Rgba = theme::PANEL;
const PANEL_ALT: Rgba = theme::PANEL_ALT;
const ACCENT: Rgba = theme::TRANS_BLUE;
const DIM_ACCENT: Rgba = theme::DIM;
const TEXT: Rgba = theme::TEXT;
const GRAY: Rgba = theme::MUTED;
const RED: Rgba = theme::RAINBOW[0]; // pride red = "stop / active"
const GREEN: Rgba = theme::RAINBOW[3]; // pride green = "running / ok"

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

    // Toolbar — trans flag stripe across the top edge.
    fill_rect(&mut fb, w as u32, 0, 0, w as u32, toolbar_h, PANEL);
    draw_flag_bar(
        &mut fb,
        w as u32,
        0,
        0,
        w as u32,
        3,
        &theme::TRANS_FLAG,
        true,
    );
    // Soft pink drop-shadow keeps the wordmark legible on any surface.
    draw_text(
        &mut fb,
        w as u32,
        h as u32,
        13,
        11,
        "xHOLY",
        theme::TRANS_PINK,
        2,
    );
    draw_text(
        &mut fb,
        w as u32,
        h as u32,
        12,
        10,
        "xHOLY",
        theme::TRANS_WHITE,
        2,
    );
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
            theme::mix(theme::PANEL, theme::BISEXUAL[0], 0.35),
        );
        draw_text(
            &mut fb,
            w as u32,
            h as u32,
            12,
            toolbar_h + 5,
            &truncate(&state.software_message.to_uppercase(), 90),
            theme::TRANS_WHITE,
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
    stroke_rect(fb, fb_w, px, py, pw, ph, theme::TRANS_BLUE);
    draw_flag_bar(
        fb,
        fb_w,
        px + 1,
        py + 1,
        pw - 2,
        3,
        &theme::TRANS_FLAG,
        true,
    );
    draw_text(
        fb,
        fb_w,
        fb_h,
        px + 15,
        py + 11,
        "SETTINGS",
        theme::TRANS_PINK,
        2,
    );
    draw_text(
        fb,
        fb_w,
        fb_h,
        px + 14,
        py + 10,
        "SETTINGS",
        theme::TRANS_WHITE,
        2,
    );

    let mut y = py + 44;
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
            theme::TRANS_PINK,
            1,
        );
        fill_rect(fb, fb_w, px + 14, y + 15, pw - 28, 1, theme::DIM);
        y += 20;
    }
    draw_text(
        fb,
        fb_w,
        fb_h,
        px + 14,
        y + 6,
        "PRESS O TO CLOSE",
        DIM_ACCENT,
        1,
    );
}

fn render_toolbar(
    fb: &mut Framebuffer,
    fb_w: u32,
    state: &WindowedGuiState,
    toolbar_h: u32,
) -> Vec<(Rect, &'static str)> {
    // Every control carries a different LGBTQ+ flag; LAUNCH (the primary
    // action) uses the trans flag. Dynamic states overlay the flag stripe.
    let labels: [&'static str; 7] = ["LAUNCH", "PLAY", "PAUSE", "EJECT", "SAVE", "LOAD", "CHEATS"];
    let mut buttons = Vec::new();
    let fh = fb_h(fb, fb_w);
    let mut x = 250u32;
    let y = 7u32;
    let bw = 74u32;
    let bh = toolbar_h - 11;
    for label in labels {
        if x + bw > fb_w.saturating_sub(10) {
            break;
        }
        let r = Rect { x, y, w: bw, h: bh };
        let flag = theme::button_flag(label);
        let primary = flag[0];
        let hovered = r.contains(state.mouse.0, state.mouse.1);
        let active = match label {
            "PLAY" => state.running && !state.paused,
            "PAUSE" => state.paused,
            "CHEATS" => state.cheat_engine_enabled,
            _ => false,
        };
        let surface = if hovered {
            theme::shade(PANEL_ALT, 1.35)
        } else if active {
            theme::mix(PANEL_ALT, primary, 0.28)
        } else {
            PANEL_ALT
        };
        fill_rect(fb, fb_w, x, y, bw, bh, surface);
        stroke_rect(
            fb,
            fb_w,
            x,
            y,
            bw,
            bh,
            if hovered { theme::TRANS_WHITE } else { primary },
        );
        // Flag-coloured stripe along the bottom of each button.
        draw_flag_bar(fb, fb_w, x + 1, y + bh - 4, bw - 2, 3, flag, true);
        let tw = text_width(label, 1);
        draw_text(
            fb,
            fb_w,
            fh,
            x + (bw - tw) / 2,
            y + (bh - 11) / 2,
            label,
            if hovered {
                theme::TRANS_WHITE
            } else {
                theme::TEXT
            },
            1,
        );
        buttons.push((r, label));
        x += bw + 8;
    }

    // Status pill, right-aligned.
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
        13,
        status,
        status_color,
        1,
    );
    buttons
}

/// Draw a flag as `n` equal stripes, horizontally stacked (`vertical=false`)
/// or side by side (`vertical=true`).
#[allow(clippy::too_many_arguments)]
fn draw_flag_bar(
    fb: &mut Framebuffer,
    fb_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    flag: &[Rgba],
    vertical: bool,
) {
    if flag.is_empty() {
        return;
    }
    let n = flag.len() as u32;
    for (i, color) in flag.iter().enumerate() {
        let i = i as u32;
        if vertical {
            let sx = x + w * i / n;
            let sw = w * (i + 1) / n - w * i / n;
            fill_rect(fb, fb_w, sx, y, sw, h, *color);
        } else {
            let sy = y + h * i / n;
            let sh = h * (i + 1) / n - h * i / n;
            fill_rect(fb, fb_w, x, sy, w, sh, *color);
        }
    }
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
    draw_text(fb, fb_w, fb_h, 12, top + 8, "LIBRARY", ACCENT, 1);
    draw_flag_bar(
        fb,
        fb_w,
        8,
        top + 19,
        sidebar_w.saturating_sub(16),
        2,
        &theme::TRANS_FLAG,
        true,
    );
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
            // Tinted row with a blue→pink gradient marker on the left.
            fill_rect(
                fb,
                fb_w,
                r.x,
                r.y,
                r.w,
                r.h,
                theme::mix(PANEL_ALT, theme::TRANS_BLUE, 0.30),
            );
            for sy in 0..r.h {
                let t = sy as f32 / r.h.max(1) as f32;
                fill_rect(fb, fb_w, r.x, r.y + sy, 4, 1, theme::trans_gradient(t));
            }
            stroke_rect(fb, fb_w, r.x, r.y, r.w, r.h, theme::TRANS_PINK);
        } else {
            fill_rect(fb, fb_w, r.x, r.y, r.w, r.h, PANEL_ALT);
        }
        let label = truncate(&g.title.to_uppercase(), 26);
        draw_text(
            fb,
            fb_w,
            fb_h,
            r.x + (if i == state.selected_game { 10 } else { 8 }),
            r.y + 8,
            &label,
            if i == state.selected_game {
                theme::TRANS_WHITE
            } else {
                TEXT
            },
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
    draw_text(fb, fb_w, fb_h, x + 12, top + 10, "PERFORMANCE", ACCENT, 1);
    draw_flag_bar(
        fb,
        fb_w,
        x + 12,
        top + 19,
        w.saturating_sub(24),
        2,
        &theme::TRANS_FLAG,
        true,
    );
    if !state.gpu_label.is_empty() {
        draw_text(fb, fb_w, fb_h, x + 12, top + 22, "GPU API", GRAY, 1);
        draw_text(
            fb,
            fb_w,
            fb_h,
            x + 12,
            top + 34,
            &truncate(&state.gpu_label.to_uppercase(), 24),
            ACCENT,
            1,
        );
    }
    let mut y = top + 58;
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
    for (i, (label, value)) in rows.into_iter().enumerate() {
        draw_text(fb, fb_w, fb_h, x + 12, y, &label, GRAY, 1);
        // Alternate trans blue / pink for a lively but readable panel.
        let value_color = if i % 2 == 0 {
            theme::TRANS_BLUE
        } else {
            theme::TRANS_PINK
        };
        draw_text(
            fb,
            fb_w,
            fb_h,
            (x + w).saturating_sub(12 + text_width(&value, 1)),
            y,
            &value,
            value_color,
            1,
        );
        y += 22;
    }

    y += 14;
    draw_text(fb, fb_w, fb_h, x + 12, y, "CONTROLS", DIM_ACCENT, 1);
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

    // Guest console output (Phase 18): last launch's stdout/kernel log.
    if !state.console_log.is_empty() {
        y += 10;
        draw_text(fb, fb_w, fb_h, x + 12, y, "CONSOLE", DIM_ACCENT, 1);
        y += 16;
        for line in state.console_log.iter().rev().take(6).rev() {
            if y + 14 > fb_h {
                break;
            }
            draw_text(fb, fb_w, fb_h, x + 12, y, &truncate(line, 26), GRAY, 1);
            y += 14;
        }
    }
}

fn render_preview_area(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    fill_rect(fb, fb_w, area.x, area.y, area.w, area.h, theme::INSET);
    stroke_rect(fb, fb_w, area.x, area.y, area.w, area.h, theme::DIM);
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
            DIM_ACCENT,
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
    // Phase 18: when the guest wrote a framebuffer frame, blit *that*
    // instead of the synthetic preview card. The frame is held in
    // state across composites, so an empty guest FB keeps the last
    // frame on screen rather than resetting to black.
    let (pw, ph) = fit_size(state.preview_size.0, state.preview_size.1, area.w, area.h);
    let px = if let Some(gf) = &state.guest_frame {
        super::preview::scale_rgba(
            &gf.pixels,
            gf.width as usize,
            gf.height as usize,
            pw as usize,
            ph as usize,
        )
    } else {
        render_game_preview(game, pw, ph)
    };
    let ox = area.x + (area.w.saturating_sub(pw)) / 2;
    let oy = area.y + (area.h.saturating_sub(ph)) / 2;
    blit_rgba(fb, fb_w, fb_h, &px, pw as usize, ph as usize, ox, oy);

    // Title block under preview: trans stripe + shadowed white title.
    let title = truncate(&game.title.to_uppercase(), 40);
    draw_flag_bar(
        fb,
        fb_w,
        area.x + 16,
        area.y + area.h - 38,
        area.w.saturating_sub(32).min(160),
        3,
        &theme::TRANS_FLAG,
        true,
    );
    draw_text(
        fb,
        fb_w,
        fb_h,
        area.x + 17,
        area.y + area.h - 29,
        &title,
        theme::TRANS_PINK,
        2,
    );
    draw_text(
        fb,
        fb_w,
        fb_h,
        area.x + 16,
        area.y + area.h - 30,
        &title,
        theme::TRANS_WHITE,
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

/// Public fill_rect for the widget layer (Phase 9).
pub fn fill_rect_pub(
    fb: &mut Framebuffer,
    fb_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: [u8; 4],
) {
    fill_rect(fb, fb_w, x, y, w, h, color);
}

/// Public stroke_rect for the widget layer (Phase 9).
pub fn stroke_rect_pub(
    fb: &mut Framebuffer,
    fb_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: [u8; 4],
) {
    stroke_rect(fb, fb_w, x, y, w, h, color);
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
