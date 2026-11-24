//! Software UI compositor for the windowed GUI.
//!
//! Draws the sidebar (game library), toolbar (transport buttons), info
//! panel (FPS/CPU/GPU/RAM) and preview area into an RGBA8 framebuffer.
//! Text uses a built-in 5×7 bitmap font — deterministic and dependency-
//! free, consistent with the Phase-2 software rasterizer approach.

use super::preview::render_game_preview;
use super::state::{GuiView, WindowedGuiState};
use crate::gui::scarletx::BootStage;
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
    /// Top navigation rects paired with their view labels.
    pub nav: Vec<(Rect, &'static str)>,
    /// Toolbar button rects paired with their labels.
    pub buttons: Vec<(Rect, &'static str)>,
    /// Sidebar game-row rects paired with game indices.
    pub rows: Vec<(Rect, usize)>,
    /// Cover-grid cells paired with game indices.
    pub covers: Vec<(Rect, usize)>,
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
    layout.nav = render_navigation(&mut fb, w as u32, state, toolbar_h);

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
    let preview = layout.preview;
    render_content(&mut fb, w as u32, h as u32, state, &preview, &mut layout);

    // Retail-boot progress bar over the preview (Phase 24).
    if let Some(boot) = &state.boot_progress {
        let bar_x = prev_x + 10;
        let bar_y = prev_y + 8;
        let bar_w = prev_w.saturating_sub(20);
        fill_rect(&mut fb, w as u32, bar_x, bar_y, bar_w, 6, PANEL_ALT);
        let done = bar_w * boot.percent as u32 / 100;
        let color = if boot.stage == BootStage::Failed {
            RED
        } else {
            ACCENT
        };
        fill_rect(&mut fb, w as u32, bar_x, bar_y, done, 6, color);
    }

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

/// Top navigation strip: six views, one consistent visual language.
fn render_navigation(
    fb: &mut Framebuffer,
    fb_w: u32,
    state: &WindowedGuiState,
    toolbar_h: u32,
) -> Vec<(Rect, &'static str)> {
    let mut items = Vec::new();
    let nav_y = toolbar_h;
    let nav_h = 20u32;
    let gap = 6u32;
    let bw = (fb_w.saturating_sub(24 + gap * 5)) / 6;
    if bw < 24 {
        return items;
    }
    for (i, view) in GuiView::ALL.iter().enumerate() {
        let x = 12 + i as u32 * (bw + gap);
        let r = Rect {
            x,
            y: nav_y,
            w: bw,
            h: nav_h,
        };
        let active = state.view == *view;
        let fill = if active {
            theme::mix(PANEL_ALT, theme::TRANS_BLUE, 0.35)
        } else {
            PANEL_ALT
        };
        fill_rect(fb, fb_w, x, nav_y, bw, nav_h, fill);
        // Active underline uses the trans flag; inactive uses a dim line.
        draw_flag_bar(
            fb,
            fb_w,
            x,
            nav_y + nav_h - 2,
            bw,
            2,
            if active {
                &theme::TRANS_FLAG
            } else {
                &[DIM_ACCENT]
            },
            true,
        );
        let label = view.label();
        let tw = text_width(label, 1);
        if tw <= bw {
            draw_text(
                fb,
                fb_w,
                fb_h(fb, fb_w),
                x + (bw - tw) / 2,
                nav_y + (nav_h - 7) / 2,
                label,
                if active { theme::TRANS_WHITE } else { GRAY },
                1,
            );
        }
        items.push((r, label));
    }
    items
}

/// Draw the active view's main content area.
///
/// The software compositor stays the single source of UI pixels; no
/// emulation logic runs here. All values come from `WindowedGuiState`.
fn render_content(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    match state.view {
        GuiView::Emulation => render_preview_area(fb, fb_w, fb_h_px, state, area),
        GuiView::Library => {
            if state.running && state.guest_frame.is_some() {
                render_preview_area(fb, fb_w, fb_h_px, state, area);
            } else {
                render_cover_grid(fb, fb_w, fb_h_px, state, area, layout);
            }
        }
        GuiView::Home => render_home(fb, fb_w, fb_h_px, state, area),
        GuiView::Settings => render_settings_view(fb, fb_w, fb_h_px, state, area),
        GuiView::Logs => render_logs(fb, fb_w, fb_h_px, state, area),
        GuiView::About => render_about(fb, fb_w, fb_h_px, state, area),
    }
}

fn panel_heading(fb: &mut Framebuffer, fb_w: u32, fb_h_px: u32, area: &Rect, title: &str) {
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        area.y + 16,
        title,
        ACCENT,
        2,
    );
    draw_flag_bar(
        fb,
        fb_w,
        area.x + 18,
        area.y + 38,
        area.w.saturating_sub(36).min(180),
        3,
        &theme::TRANS_FLAG,
        true,
    );
}

fn render_home(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    panel_heading(fb, fb_w, fb_h_px, area, "xHOLY");
    let mut y = area.y + 56;
    let status = if state.running {
        "EMULATOR: RUNNING"
    } else if state.paused {
        "EMULATOR: PAUSED"
    } else {
        "EMULATOR: IDLE"
    };
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y,
        status,
        theme::TRANS_PINK,
        1,
    );
    y += 20;
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y,
        &format!("{} GAMES IN LIBRARY", state.games.len()),
        theme::TRANS_WHITE,
        1,
    );
    y += 28;
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y,
        "RECENT GAMES",
        DIM_ACCENT,
        1,
    );
    y += 18;
    for (i, game) in state.games.iter().take(3).enumerate() {
        if y + 12 > area.y + area.h {
            break;
        }
        draw_text(
            fb,
            fb_w,
            fb_h_px,
            area.x + 18,
            y,
            &truncate(&game.title.to_uppercase(), 32),
            if i == 0 { ACCENT } else { TEXT },
            1,
        );
        y += 16;
    }
    y += 12;
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y,
        "PRESS LIBRARY TO BROWSE",
        GRAY,
        1,
    );
    y += 16;
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y,
        "O: SETTINGS  |  L: LOGS",
        GRAY,
        1,
    );
    let _ = fb_h_px;
}

fn render_settings_view(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    panel_heading(fb, fb_w, fb_h_px, area, "SETTINGS");
    let groups = [
        "GENERAL",
        "GRAPHICS",
        "AUDIO",
        "INPUT",
        "CPU / EMULATION",
        "DISCORD",
        "LOGS & DEBUG",
    ];
    let mut y = area.y + 56;
    for (i, group) in groups.iter().enumerate() {
        if y + 14 > area.y + area.h {
            break;
        }
        let color = if i % 2 == 0 {
            ACCENT
        } else {
            theme::TRANS_PINK
        };
        draw_text(fb, fb_w, fb_h_px, area.x + 18, y, group, color, 1);
        fill_rect(
            fb,
            fb_w,
            area.x + 18,
            y + 13,
            area.w.saturating_sub(36).min(220),
            1,
            DIM_ACCENT,
        );
        y += 22;
    }
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y + 4,
        &format!(
            "THEME: {:?}  GPU: {}",
            state.theme,
            truncate(&state.gpu_label, 20)
        ),
        GRAY,
        1,
    );
}

fn render_logs(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    panel_heading(fb, fb_w, fb_h_px, area, "DIAGNOSTICS");
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        area.y + 48,
        "FILTER: ALL / INFO / WARNING / ERROR",
        GRAY,
        1,
    );
    let mut y = area.y + 66;
    for line in state.console_log.iter().rev().take(8) {
        if y + 12 > area.y + area.h {
            break;
        }
        // Structured view: level tag + module + message where available.
        let (tag, color) = if line.contains("error") {
            ("ERR", theme::TRANS_PINK)
        } else if line.contains("warn") {
            ("WRN", theme::TRANS_PINK)
        } else {
            ("INF", ACCENT)
        };
        draw_text(fb, fb_w, fb_h_px, area.x + 18, y, tag, color, 1);
        draw_text(
            fb,
            fb_w,
            fb_h_px,
            area.x + 60,
            y,
            &truncate(line, 36),
            TEXT,
            1,
        );
        y += 14;
    }
    if state.console_log.is_empty() {
        draw_text(
            fb,
            fb_w,
            fb_h_px,
            area.x + 18,
            y,
            "NO LOG ENTRIES YET",
            GRAY,
            1,
        );
    }
}

fn render_about(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
) {
    panel_heading(fb, fb_w, fb_h_px, area, "ABOUT xHOLY");
    let mut y = area.y + 56;
    for line in [
        "xHoly - research HLE emulator",
        "Xbox One / Xbox Series X|S",
        "Windows x86-64 / Linux x86-64",
        "Pride UI: trans-blue / trans-pink / white",
        "Discord Rich Presence: enabled via config",
        "ScarlettX: compatibility reference",
    ] {
        draw_text(fb, fb_w, fb_h_px, area.x + 18, y, line, TEXT, 1);
        y += 18;
    }
    draw_text(
        fb,
        fb_w,
        fb_h_px,
        area.x + 18,
        y + 6,
        &format!("GPU: {}", truncate(&state.gpu_label, 32)),
        GRAY,
        1,
    );
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

    fill_rect_rounded(fb, fb_w, px, py, pw, ph, 10, PANEL);
    fill_rect(fb, fb_w, px + 8, py + ph - 1, pw - 16, 1, theme::TRANS_BLUE);
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
        // Each flag is recognisable on the button *surface*, not only in
        // a decorative line: hover/active strengthen the tint.
        let surface = if hovered {
            theme::mix(PANEL_ALT, primary, 0.55)
        } else if active {
            theme::mix(PANEL_ALT, primary, 0.45)
        } else {
            theme::mix(PANEL_ALT, primary, 0.30)
        };
        fill_rect_rounded(fb, fb_w, x, y, bw, bh, 5, surface);
        // Wider flag band so the palette reads as part of the control.
        draw_flag_bar(fb, fb_w, x + 1, y + bh - 7, bw - 2, 6, flag, true);
        // Thin top highlight instead of a heavy full border.
        fill_rect(
            fb,
            fb_w,
            x + 5,
            y,
            bw.saturating_sub(10),
            1,
            if hovered {
                theme::TRANS_WHITE
            } else {
                theme::mix(primary, theme::TRANS_WHITE, 0.45)
            },
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
            fill_rect_rounded(
                fb,
                fb_w,
                r.x,
                r.y,
                r.w,
                r.h,
                4,
                theme::mix(PANEL_ALT, theme::TRANS_BLUE, 0.30),
            );
            for sy in 0..r.h {
                let t = sy as f32 / r.h.max(1) as f32;
                fill_rect(fb, fb_w, r.x, r.y + sy, 4, 1, theme::trans_gradient(t));
            }
            fill_rect(
                fb,
                fb_w,
                r.x + 4,
                r.y + r.h - 1,
                r.w - 4,
                1,
                theme::TRANS_PINK,
            );
        } else {
            fill_rect_rounded(fb, fb_w, r.x, r.y, r.w, r.h, 4, PANEL_ALT);
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

/// Cover grid for the Game Library view.
fn render_cover_grid(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h_px: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    use crate::gui::thumbnails;
    panel_heading(fb, fb_w, fb_h_px, area, "GAME LIBRARY");
    let query = state.search.trim().to_ascii_lowercase();
    let games: Vec<(usize, &crate::gui::GameListItem)> = state
        .games
        .iter()
        .enumerate()
        .filter(|(_, g)| query.is_empty() || g.title.to_ascii_lowercase().contains(&query))
        .collect();
    if games.is_empty() {
        draw_text(
            fb,
            fb_w,
            fb_h_px,
            area.x + 18,
            area.y + 64,
            "NO GAMES MATCH",
            GRAY,
            1,
        );
        return;
    }
    let cols = ((area.w.saturating_sub(24)) / 156).clamp(1, 6);
    let cell_w = (area.w.saturating_sub(24) / cols).min(180);
    let img_h = (cell_w * 3 / 4).max(40);
    let cell_h = img_h + 26;
    let loader = thumbnails::loader();
    for (slot, (index, game)) in games.iter().enumerate() {
        let col = slot as u32 % cols;
        let row = slot as u32 / cols;
        let x = area.x + 12 + col * cell_w;
        let y = area.y + 52 + row * cell_h;
        if y + cell_h > area.y + area.h {
            break;
        }
        let r = Rect {
            x,
            y,
            w: cell_w.saturating_sub(6),
            h: cell_h.saturating_sub(4),
        };
        let selected = *index == state.selected_game;
        fill_rect_rounded(
            fb,
            fb_w,
            r.x,
            r.y,
            r.w,
            r.h,
            5,
            if selected {
                theme::mix(PANEL_ALT, theme::TRANS_BLUE, 0.28)
            } else {
                PANEL_ALT
            },
        );
        // Cover image, async-loaded and cached; placeholder on miss.
        let cover = game.cover_art.as_ref().and_then(|path| {
            let cache = loader.cache();
            if cache.get(path).is_none() {
                loader.queue(path.clone());
            }
            cache.get(path).flatten()
        });
        let image = cover
            .map(|img| thumbnails::cover_fit(&img, r.w, img_h))
            .unwrap_or_else(|| thumbnails::placeholder(r.w, img_h));
        blit_rgba(
            fb,
            fb_w,
            fb_h_px,
            &image.pixels,
            image.width as usize,
            image.height as usize,
            r.x,
            r.y,
        );
        if selected {
            draw_flag_bar(
                fb,
                fb_w,
                r.x,
                r.y + img_h - 3,
                r.w,
                3,
                &theme::TRANS_FLAG,
                true,
            );
        }
        draw_text(
            fb,
            fb_w,
            fb_h_px,
            r.x + 4,
            r.y + img_h + 4,
            &truncate(&game.title.to_uppercase(), (r.w / 8) as usize),
            if selected { theme::TRANS_WHITE } else { TEXT },
            1,
        );
        layout.covers.push((r, *index));
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
    _dst_h: u32,
    src: &[u8],
    src_w: usize,
    src_h: usize,
    ox: u32,
    oy: u32,
) {
    let dst_h_px = (dst.len() / 4 / dst_w.max(1) as usize) as u32;
    for sy in 0..src_h {
        let y = oy as usize + sy;
        if y >= dst_h_px as usize {
            break;
        }
        let src_start = sy * src_w * 4;
        if src_start >= src.len() {
            break;
        }
        let src_end = ((sy + 1) * src_w * 4).min(src.len());
        let ox_usize = ox as usize;
        let available = (dst_w as usize).saturating_sub(ox_usize);
        let copy_w = ((src_end - src_start) / 4).min(available);
        if copy_w == 0 {
            continue;
        }
        let dst_start = (y * dst_w as usize + ox as usize) * 4;
        let dst_end = dst_start + copy_w * 4;
        if dst_end <= dst.len() {
            dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + copy_w * 4]);
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
    if fb_w == 0 {
        return;
    }
    let fb_h_px = (fb.len() / 4 / fb_w as usize) as u32;
    let x0 = x.min(fb_w);
    let x1 = x.saturating_add(w).min(fb_w);
    let y0 = y.min(fb_h_px);
    let y1 = y.saturating_add(h).min(fb_h_px);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    // Clamp once, then copy whole rows — far cheaper than a per-pixel
    // bounds check on every byte of a 1080p framebuffer.
    for yy in y0..y1 {
        let start = (yy as usize * fb_w as usize + x0 as usize) * 4;
        let end = (yy as usize * fb_w as usize + x1 as usize) * 4;
        for px in fb[start..end].as_chunks_mut::<4>().0 {
            px.copy_from_slice(&color);
        }
    }
}

/// Fill a rounded rectangle (used for buttons and floating panels).
#[allow(clippy::too_many_arguments)]
fn fill_rect_rounded(
    fb: &mut Framebuffer,
    fb_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: [u8; 4],
) {
    if w < 2 || h < 2 {
        fill_rect(fb, fb_w, x, y, w, h, color);
        return;
    }
    let r = radius.min(w / 2).min(h / 2);
    fill_rect(fb, fb_w, x, y + r, w, h.saturating_sub(2 * r), color);
    fill_rect(fb, fb_w, x + r, y, w.saturating_sub(2 * r), h, color);
    for i in 0..r {
        let cut = r - i;
        if w > 2 * cut {
            fill_rect(fb, fb_w, x + cut, y + i, w - 2 * cut, 1, color);
            fill_rect(fb, fb_w, x + cut, y + h - 1 - i, w - 2 * cut, 1, color);
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
// Text drawing - embedded Inter font (no pixel/terminal font in the GUI)
// ---------------------------------------------------------------------------

/// Draw text at `(x, y)` top-left, `scale` = integer size step.
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
    crate::gui::text::draw_text(fb, fb_w, fb_h, x, y, text, color, scale);
}

/// Width of `text` in pixels at the given scale.
pub fn text_width(text: &str, scale: u32) -> u32 {
    crate::gui::text::text_width(text, scale)
}

// Keep GameListItem referenced for doc links.
#[allow(unused)]
fn _doc(_: Option<&GameListItem>) {}
