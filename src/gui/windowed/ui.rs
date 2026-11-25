//! Software UI compositor for the windowed GUI (1080p layout).
//!
//! Structure follows a launcher-style design: a dark top bar with view tabs,
//! a header row with title and subtitle, the active view as content, and a
//! footer with the transport controls. Accents use the trans flag; each
//! action button carries its own flag palette. Only real emulator state is
//! shown; performance values appear only while an emulation is running.

use super::preview::{render_game_preview, scale_rgba};
use super::state::{DiagnosticsTab, GuiView, LogFilter, TracePanel, WindowedGuiState};
use crate::core::cpu::x64::register_name;
use crate::core::hle::ImplementationState;
use crate::core::pipeline::diagnostics::DiagnosticEvent;
use crate::core::pipeline::trace::{
    event_detail_lines, filter_events, import_summary, stack_bounds_from_report, TraceEventFilter,
};
use crate::core::pipeline::ExecutionReport;
use crate::gui::thumbnails;
use crate::gui::GameListItem;
use crate::theme::{self, Rgba};

/// Framebuffer pixel = RGBA8.
pub type Framebuffer = Vec<u8>;

const BG: Rgba = theme::BG;
const BAR: Rgba = theme::INSET;
const PANEL: Rgba = theme::PANEL;
const CARD: Rgba = theme::PANEL_ALT;
const TEXT: Rgba = theme::TEXT;
const MUTED: Rgba = theme::MUTED;
const DIM: Rgba = theme::DIM;
const ACCENT: Rgba = theme::TRANS_BLUE;
const PINK: Rgba = theme::TRANS_PINK;
const WHITE: Rgba = theme::TRANS_WHITE;
const RED: Rgba = theme::RAINBOW[0];
const YELLOW: Rgba = theme::RAINBOW[2];

/// Top tab bar height (1080p).
pub const TOP_H: u32 = 48;
/// Header row (title + subtitle) height.
pub const HEAD_H: u32 = 92;
/// Footer with transport controls.
pub const FOOT_H: u32 = 116;
const MARGIN: u32 = 48;
const TAB_X: u32 = 200;
const BUTTON_H: u32 = 56;
const GAP: u32 = 12;

/// Layout rectangles (in framebuffer pixels) for hit-testing.
#[derive(Debug, Clone, Default)]
pub struct UiLayout {
    /// View tabs paired with their labels.
    pub nav: Vec<(Rect, &'static str)>,
    /// Transport buttons paired with their labels.
    pub buttons: Vec<(Rect, &'static str)>,
    /// Game cards (home view) paired with game indices.
    pub rows: Vec<(Rect, usize)>,
    /// Kept for compatibility; the home view uses `rows`.
    pub recent: Vec<(Rect, usize)>,
    /// Cover-grid cells paired with game indices.
    pub covers: Vec<(Rect, usize)>,
    /// Log filter chips paired with their labels.
    pub filters: Vec<(Rect, &'static str)>,
    /// Diagnostics sub-mode chips (`CONSOLE` / `HLE TRACE`).
    pub diag_tabs: Vec<(Rect, &'static str)>,
    /// HLE Trace Viewer sub-panel tabs.
    pub trace_panels: Vec<(Rect, &'static str)>,
    /// HLE Trace Viewer event-type filter chips.
    pub trace_filters: Vec<(Rect, &'static str)>,
    /// HLE Trace Viewer timeline rows paired with their filtered index.
    pub trace_events: Vec<(Rect, usize)>,
    /// Buttons of the emulation overlay.
    pub menu: Vec<(Rect, &'static str)>,
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

/// Composite a full GUI frame.
pub fn composite(state: &WindowedGuiState, width: u32, height: u32) -> (Framebuffer, UiLayout) {
    let w = width.max(1);
    let h = height.max(1);
    let mut fb = vec![0u8; w as usize * h as usize * 4];
    for px in fb.as_chunks_mut::<4>().0 {
        *px = BG;
    }
    let mut layout = UiLayout::default();

    // While a game is running the guest frame is the background; the
    // chrome is hidden and only the emulation overlay (Ctrl+Alt+T) is drawn.
    if state.running {
        if let Some(gf) = &state.guest_frame {
            let px = super::preview::scale_rgba(
                &gf.pixels,
                gf.width as usize,
                gf.height as usize,
                w as usize,
                h as usize,
            );
            if px.len() == fb.len() {
                fb.copy_from_slice(&px);
            }
        }
        if state.emu_menu_open {
            draw_emu_menu(&mut fb, w, h, state, &mut layout);
        }
        return (fb, layout);
    }
    draw_topbar(&mut fb, w, h, state, &mut layout);
    let body_top = TOP_H + HEAD_H;
    let body_bottom = h.saturating_sub(FOOT_H);
    draw_header(&mut fb, w, h, state);
    let banner = if state.software_mode {
        draw_banner(&mut fb, w, h, state);
        32
    } else {
        0
    };
    let area = Rect {
        x: MARGIN,
        y: body_top + banner + 24,
        w: w.saturating_sub(MARGIN * 2),
        h: body_bottom.saturating_sub(body_top + 24),
    };
    match state.view {
        GuiView::Home => draw_home(&mut fb, w, h, state, &area, &mut layout),
        GuiView::Library => draw_library(&mut fb, w, h, state, &area, &mut layout),
        GuiView::Emulation => draw_emulation(&mut fb, w, h, state, &area, &mut layout),
        GuiView::Settings => draw_settings_view(&mut fb, w, h, state, &area),
        GuiView::Logs => draw_logs(&mut fb, w, h, state, &area, &mut layout),
        GuiView::About => draw_about(&mut fb, w, h, state, &area),
    }
    draw_footer(&mut fb, w, h, state, &mut layout);
    if state.emu_menu_open {
        draw_emu_menu(&mut fb, w, h, state, &mut layout);
    }
    if state.settings_open {
        draw_settings_modal(&mut fb, w, h, state);
    }
    (fb, layout)
}

/// Human-readable emulator state.
fn state_label(state: &WindowedGuiState) -> &'static str {
    if !state.running {
        "IDLE"
    } else if state.paused {
        "PAUSED"
    } else {
        "RUNNING"
    }
}

/// Short metadata line for a game entry.
fn game_meta(game: &GameListItem) -> String {
    format!(
        "{}  \u{2022}  {} DISC{}",
        game.region,
        game.discs,
        if game.discs > 1 { "S" } else { "" }
    )
}

/// Whether a transport button can currently be used.
pub fn button_enabled(label: &str, state: &WindowedGuiState) -> bool {
    match label {
        "LAUNCH" => !state.running && !state.games.is_empty(),
        "PLAY" => state.running && state.paused,
        "PAUSE" => state.running && !state.paused,
        "EJECT" | "SAVE" | "LOAD" => state.running,
        _ => true,
    }
}

fn button_active(label: &str, state: &WindowedGuiState) -> bool {
    match label {
        "LAUNCH" => state.running,
        "CHEATS" => state.cheat_engine_enabled,
        _ => false,
    }
}

fn draw_topbar(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    layout: &mut UiLayout,
) {
    fill_rect(fb, w, 0, 0, w, TOP_H, BAR);
    draw_flag_bar(fb, w, 0, TOP_H - 3, w, 3, &theme::TRANS_FLAG, true);
    draw_text(fb, w, h, 48, 14, "XHOLY", WHITE, 2);
    let mut x = TAB_X;
    for view in GuiView::ALL.iter() {
        let label = view.label();
        let lw = text_width(label, 1);
        let r = Rect {
            x,
            y: 0,
            w: lw + 36,
            h: TOP_H - 3,
        };
        let active = state.view == *view;
        let hovered = r.contains(state.mouse.0, state.mouse.1);
        let col = if active {
            WHITE
        } else if hovered {
            TEXT
        } else {
            MUTED
        };
        draw_text(fb, w, h, r.x + 18, r.y + 17, label, col, 1);
        if active {
            fill_rect(fb, w, r.x + 10, TOP_H - 6, r.w - 20, 3, ACCENT);
        }
        layout.nav.push((r, label));
        x += r.w + 4;
    }
    let label = state_label(state);
    let color = match label {
        "PAUSED" => PINK,
        "RUNNING" => ACCENT,
        _ => MUTED,
    };
    let pw = text_width(label, 1) + 40;
    let px = w.saturating_sub(pw + MARGIN);
    fill_rect_rounded(fb, w, px, 12, pw, 24, 12, CARD);
    fill_rect_rounded(fb, w, px + 14, 20, 8, 8, 4, color);
    draw_text(fb, w, h, px + 30, 17, label, TEXT, 1);
}

fn draw_header(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState) {
    fill_rect(fb, w, 0, TOP_H, w, HEAD_H, PANEL);
    fill_rect(fb, w, 0, TOP_H + HEAD_H - 1, w, 1, CARD);
    let (title, sub) = match state.view {
        GuiView::Home => (
            "HOME",
            "Your library at a glance. Pick a game and launch it.",
        ),
        GuiView::Library => ("GAME LIBRARY", "Every game found in your library folder."),
        GuiView::Emulation => ("EMULATION", "Live state of the running title."),
        GuiView::Settings => ("SETTINGS", "Current configuration, read-only in this view."),
        GuiView::Logs => ("DIAGNOSTICS", "Output and messages from the emulator."),
        GuiView::About => ("ABOUT", "Version, platform and project information."),
    };
    draw_text(fb, w, h, 48, TOP_H + 16, title, ACCENT, 2);
    draw_text(fb, w, h, 48, TOP_H + 52, sub, MUTED, 1);
}

fn draw_footer(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    layout: &mut UiLayout,
) {
    let top = h.saturating_sub(FOOT_H);
    fill_rect(fb, w, 0, top, w, FOOT_H, BAR);
    fill_rect(fb, w, 0, top, w, 1, CARD);
    draw_text(fb, w, h, MARGIN, top + 14, "CONTROLS", MUTED, 1);
    let inner = w.saturating_sub(MARGIN * 2);
    let bw = (inner.saturating_sub(GAP * 6) / 7).clamp(96, 240);
    let by = top + 40;
    for (i, (label, flag)) in theme::BUTTON_FLAGS.iter().enumerate() {
        let r = Rect {
            x: MARGIN + i as u32 * (bw + GAP),
            y: by,
            w: bw,
            h: BUTTON_H,
        };
        let enabled = button_enabled(label, state);
        let hovered = enabled && r.contains(state.mouse.0, state.mouse.1);
        let pressed = hovered && state.mouse_down;
        let active = button_active(label, state);
        draw_button(fb, w, h, r, label, flag, enabled, hovered, pressed, active);
        layout.buttons.push((r, *label));
    }
}
/// Transport button: flag stripes on the surface, dark label chip.
#[allow(clippy::too_many_arguments)]
fn draw_button(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    r: Rect,
    label: &str,
    flag: &[Rgba],
    enabled: bool,
    hovered: bool,
    pressed: bool,
    active: bool,
) {
    let bands = flag.len().max(1) as u32;
    for (i, color) in flag.iter().enumerate() {
        let i = i as u32;
        let y0 = r.y + i * r.h / bands;
        let y1 = r.y + (i + 1) * r.h / bands;
        let c = if !enabled {
            theme::mix(PANEL, *color, 0.30)
        } else if pressed {
            theme::shade(*color, 0.72)
        } else if hovered {
            theme::mix(*color, WHITE, 0.18)
        } else {
            *color
        };
        fill_rect(fb, w, r.x, y0, r.w, y1 - y0, c);
    }
    let cw = text_width(label, 2) + 28;
    let cx = r.x + r.w.saturating_sub(cw) / 2;
    let cy = r.y + r.h.saturating_sub(30) / 2;
    fill_rect_rounded(fb, w, cx, cy, cw, 30, 8, BG);
    let col = if enabled { WHITE } else { MUTED };
    draw_text(fb, w, h, cx + 14, cy + 8, label, col, 2);
    if active {
        fill_rect(
            fb,
            w,
            r.x + 10,
            r.y + r.h - 4,
            r.w.saturating_sub(20),
            4,
            WHITE,
        );
    }
    if hovered {
        stroke_rect(fb, w, r.x, r.y, r.w, r.h, WHITE);
    }
}

/// Rounded surface with a heading.
fn card(fb: &mut Framebuffer, w: u32, h: u32, r: Rect, title: &str) {
    fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 14, CARD);
    draw_text(fb, w, h, r.x + 20, r.y + 18, title, MUTED, 1);
}

/// Label/value list; returns the y coordinate after the last row.
fn kv_rows(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    width: u32,
    rows: &[(&str, String)],
) -> u32 {
    let mut yy = y;
    for (label, value) in rows {
        draw_text(fb, w, h, x, yy, label, MUTED, 1);
        let vw = text_width(value, 1);
        draw_text(
            fb,
            w,
            h,
            (x + width).saturating_sub(vw),
            yy,
            value,
            ACCENT,
            1,
        );
        fill_rect(fb, w, x, yy + 22, width, 1, BG);
        yy += 34;
    }
    yy
}

fn on_off(value: bool) -> String {
    if value {
        "ON".into()
    } else {
        "OFF".into()
    }
}

/// Settings as label/value pairs (read-only; values come from CLI/config).
fn settings_rows(state: &WindowedGuiState) -> Vec<(&'static str, String)> {
    let fps = state
        .fps_limit
        .map(|f| f.to_string())
        .unwrap_or_else(|| "UNLIMITED".into());
    vec![
        (
            "RESOLUTION",
            format!("{} X {}", state.resolution.0, state.resolution.1),
        ),
        ("FPS LIMIT", fps),
        ("VSYNC", on_off(state.vsync)),
        ("FULLSCREEN", on_off(state.fullscreen)),
        ("THEME", format!("{:?}", state.theme).to_uppercase()),
        ("GPU API", state.gpu_label.to_uppercase()),
        ("SOFTWARE RENDERER", on_off(state.software_mode)),
    ]
}

/// Pride palette per game tile, chosen by position so the mapping is stable.
fn tile_palette(index: usize) -> &'static [Rgba] {
    match index % 4 {
        0 => &theme::TRANS_FLAG,
        1 => &theme::BISEXUAL,
        2 => &theme::PANSEXUAL,
        _ => &theme::LESBIAN,
    }
}

/// Vertical gradient tile in a pride palette, with title and metadata.
fn draw_tile(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    r: Rect,
    game: &GameListItem,
    index: usize,
    selected: bool,
) {
    let palette = tile_palette(index);
    let top = palette[0];
    for row in 0..r.h {
        let t = row as f32 / r.h as f32;
        let c = theme::mix(top, BG, (t * 0.9).min(1.0));
        fill_rect(fb, w, r.x, r.y + row, r.w, 1, c);
    }
    let band_h = (r.h / 10).max(4);
    draw_flag_bar(fb, w, r.x, r.y, r.w, band_h, palette, true);
    let title = truncate(&game.title.to_uppercase(), 24);
    draw_text(fb, w, h, r.x + 20, r.y + r.h - 74, &title, WHITE, 2);
    draw_text(
        fb,
        w,
        h,
        r.x + 20,
        r.y + r.h - 36,
        &game_meta(game),
        TEXT,
        1,
    );
    if selected {
        stroke_rect(fb, w, r.x, r.y, r.w, r.h, WHITE);
    }
}

fn draw_home(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    if state.games.is_empty() {
        draw_text(
            fb,
            w,
            h,
            area.x,
            area.y,
            "NO GAMES IN LIBRARY YET",
            MUTED,
            2,
        );
        draw_text(
            fb,
            w,
            h,
            area.x,
            area.y + 34,
            "Add game images or dumps to the library folder.",
            DIM,
            1,
        );
    } else {
        let mut order: Vec<usize> = (0..state.games.len()).collect();
        order.sort_by(|a, b| {
            state.games[*b]
                .last_played
                .cmp(&state.games[*a].last_played)
        });
        let no_history = order
            .first()
            .map(|i| state.games[*i].last_played.is_none())
            .unwrap_or(true);
        let label = if no_history {
            "YOUR GAMES  \u{2022}  NO PLAY HISTORY YET"
        } else {
            "RECENTLY PLAYED"
        };
        draw_text(fb, w, h, area.x, area.y, label, MUTED, 1);

        let gap = 24;
        let tw = 340u32;
        let th = area.h.saturating_sub(36).clamp(120, 220);
        let cols = ((area.w + gap) / (tw + gap)).max(1);
        let top = area.y + 36;
        for (slot, idx) in order.iter().enumerate() {
            let col = slot as u32 % cols;
            let row = slot as u32 / cols;
            let r = Rect {
                x: area.x + col * (tw + gap),
                y: top + row * (th + gap),
                w: tw,
                h: th,
            };
            if r.y + r.h > area.y + area.h {
                break;
            }
            let game = &state.games[*idx];
            draw_tile(fb, w, h, r, game, *idx, *idx == state.selected_game);
            layout.rows.push((r, *idx));
        }
    }
    let sys_y = area.y + area.h.saturating_sub(150);
    if sys_y > area.y + 260 {
        let gap = 24;
        let cw = (area.w.saturating_sub(gap * 2) / 3).max(200);
        let ch = 124;
        let c1 = Rect {
            x: area.x,
            y: sys_y,
            w: cw,
            h: ch,
        };
        card(fb, w, h, c1, "EMULATOR");
        draw_text(fb, w, h, c1.x + 20, c1.y + 46, state_label(state), TEXT, 2);
        let games = format!("{} GAMES IN LIBRARY", state.games.len());
        draw_text(fb, w, h, c1.x + 20, c1.y + 88, &games, MUTED, 1);

        let c2 = Rect {
            x: area.x + cw + gap,
            y: sys_y,
            w: cw,
            h: ch,
        };
        card(fb, w, h, c2, "DISPLAY");
        let res = format!("{} X {}", state.resolution.0, state.resolution.1);
        draw_text(fb, w, h, c2.x + 20, c2.y + 46, &res, TEXT, 2);
        let gpu = truncate(&state.gpu_label.to_uppercase(), 40);
        draw_text(fb, w, h, c2.x + 20, c2.y + 88, &gpu, MUTED, 1);

        let c3 = Rect {
            x: area.x + 2 * (cw + gap),
            y: sys_y,
            w: cw,
            h: ch,
        };
        card(fb, w, h, c3, "KEYBOARD");
        let keys = [
            "ESC  PAUSE   E  EJECT   Q  QUIT",
            "F5 SAVE   F9 LOAD   C  CHEATS",
        ];
        for (i, key) in keys.iter().enumerate() {
            draw_text(
                fb,
                w,
                h,
                c3.x + 20,
                c3.y + 52 + i as u32 * 24,
                key,
                MUTED,
                1,
            );
        }
    }
}

fn draw_library(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    let query = state.search.trim().to_ascii_lowercase();
    let games: Vec<(usize, &GameListItem)> = state
        .games
        .iter()
        .enumerate()
        .filter(|(_, g)| query.is_empty() || g.title.to_ascii_lowercase().contains(&query))
        .collect();
    if games.is_empty() {
        draw_text(fb, w, h, area.x, area.y, "NO GAMES MATCH", MUTED, 2);
        return;
    }
    let gap = 24;
    let cw = 300u32;
    let img_h = cw * 3 / 4;
    let ch = img_h + 70;
    let cols = ((area.w + gap) / (cw + gap)).max(1);
    let loader = thumbnails::loader();
    for (slot, (index, game)) in games.iter().enumerate() {
        let col = slot as u32 % cols;
        let row = slot as u32 / cols;
        let r = Rect {
            x: area.x + col * (cw + gap),
            y: area.y + row * (ch + gap),
            w: cw,
            h: ch,
        };
        if r.y + r.h > area.y + area.h {
            break;
        }
        let selected = *index == state.selected_game;
        fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 14, CARD);
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
            w,
            h,
            &image.pixels,
            image.width as usize,
            image.height as usize,
            r.x,
            r.y,
        );
        if selected {
            stroke_rect(fb, w, r.x, r.y, r.w, r.h, WHITE);
        }
        draw_flag_bar(
            fb,
            w,
            r.x,
            r.y + img_h - 4,
            r.w,
            4,
            &theme::TRANS_FLAG,
            true,
        );
        let title = truncate(&game.title.to_uppercase(), 26);
        let col_t = if selected { WHITE } else { TEXT };
        draw_text(fb, w, h, r.x + 14, r.y + img_h + 14, &title, col_t, 1);
        draw_text(
            fb,
            w,
            h,
            r.x + 14,
            r.y + img_h + 34,
            &game_meta(game),
            MUTED,
            1,
        );
        draw_text(
            fb,
            w,
            h,
            r.x + 14,
            r.y + img_h + 52,
            "COMPATIBILITY NOT VERIFIED",
            DIM,
            1,
        );
        layout.covers.push((r, *index));
    }
}
fn draw_emulation(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    let pv_w = area.w * 3 / 5;
    let pv = Rect {
        x: area.x,
        y: area.y,
        w: pv_w.saturating_sub(12),
        h: area.h,
    };
    draw_preview(fb, w, h, state, &pv);
    layout.preview = pv;

    let side_x = area.x + pv_w;
    let side_w = area.w.saturating_sub(pv_w);
    let st = Rect {
        x: side_x,
        y: area.y,
        w: side_w,
        h: 124,
    };
    card(fb, w, h, st, "STATE");
    draw_text(fb, w, h, st.x + 20, st.y + 46, state_label(state), TEXT, 2);

    let perf = Rect {
        x: side_x,
        y: area.y + 140,
        w: side_w,
        h: 236,
    };
    card(fb, w, h, perf, "PERFORMANCE");
    let rows: Vec<(&str, String)> = if state.running {
        vec![
            ("FPS", format!("{:.0}", state.fps)),
            ("CPU", format!("{:.0} %", state.cpu_usage * 100.0)),
            ("GPU", format!("{:.0} %", state.gpu_usage * 100.0)),
            (
                "MEMORY",
                format!("{} MB", state.memory_usage / (1024 * 1024)),
            ),
        ]
    } else {
        vec![
            ("FPS", "-".into()),
            ("CPU", "-".into()),
            ("GPU", "-".into()),
            ("MEMORY", "-".into()),
        ]
    };
    kv_rows(
        fb,
        w,
        h,
        perf.x + 20,
        perf.y + 50,
        perf.w.saturating_sub(40),
        &rows,
    );
    let note = if state.running {
        "LIVE VALUES"
    } else {
        "VALUES APPEAR WHILE RUNNING"
    };
    draw_text(fb, w, h, perf.x + 20, perf.y + 204, note, DIM, 1);

    let out_y = perf.y + perf.h + 20;
    if area.y + area.h > out_y + 80 {
        let out = Rect {
            x: side_x,
            y: out_y,
            w: side_w,
            h: area.y + area.h - out_y,
        };
        card(fb, w, h, out, "OUTPUT");
        let max_rows = (out.h.saturating_sub(50) / 18) as usize;
        let max_chars = (out.w / 9) as usize;
        let mut ty = out.y + 50;
        for line in state.console_log.iter().rev().take(max_rows) {
            draw_text(
                fb,
                w,
                h,
                out.x + 20,
                ty,
                &truncate(line, max_chars),
                MUTED,
                1,
            );
            ty += 18;
        }
        if state.console_log.is_empty() {
            draw_text(fb, w, h, out.x + 20, ty, "NO OUTPUT YET", DIM, 1);
        }
    }
}

fn draw_preview(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState, area: &Rect) {
    fill_rect_rounded(fb, w, area.x, area.y, area.w, area.h, 14, BAR);
    let Some(game) = state.selected() else {
        let msg = "SELECT A GAME";
        let mx = area.x + area.w.saturating_sub(text_width(msg, 2)) / 2;
        draw_text(fb, w, h, mx, area.y + area.h / 2, msg, MUTED, 2);
        return;
    };
    let (pw, ph) = fit_size(
        state.preview_size.0,
        state.preview_size.1,
        area.w.saturating_sub(48),
        area.h.saturating_sub(120),
    );
    let pixels = match &state.guest_frame {
        Some(gf) => scale_rgba(
            &gf.pixels,
            gf.width as usize,
            gf.height as usize,
            pw as usize,
            ph as usize,
        ),
        None => render_game_preview(game, pw, ph),
    };
    let ox = area.x + area.w.saturating_sub(pw) / 2;
    let oy = area.y + 24;
    blit_rgba(fb, w, h, &pixels, pw as usize, ph as usize, ox, oy);
    let title = truncate(&game.title.to_uppercase(), 40);
    draw_text(
        fb,
        w,
        h,
        area.x + 24,
        area.y + area.h.saturating_sub(70),
        &title,
        WHITE,
        2,
    );
    draw_text(
        fb,
        w,
        h,
        area.x + 24,
        area.y + area.h.saturating_sub(36),
        &game_meta(game),
        MUTED,
        1,
    );
}

fn draw_settings_view(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState, area: &Rect) {
    draw_text(
        fb,
        w,
        h,
        area.x,
        area.y,
        "Read-only here. Change values via CLI flags or the config file.",
        DIM,
        1,
    );
    let rows = settings_rows(state);
    let gap = 24;
    let col_w = area.w.saturating_sub(gap) / 2;
    let top = area.y + 40;
    let ch = 320;
    let left = Rect {
        x: area.x,
        y: top,
        w: col_w,
        h: ch,
    };
    let right = Rect {
        x: area.x + col_w + gap,
        y: top,
        w: col_w,
        h: ch,
    };
    card(fb, w, h, left, "DISPLAY AND PERFORMANCE");
    kv_rows(
        fb,
        w,
        h,
        left.x + 20,
        left.y + 52,
        col_w.saturating_sub(40),
        &rows[0..4],
    );
    card(fb, w, h, right, "GRAPHICS");
    kv_rows(
        fb,
        w,
        h,
        right.x + 20,
        right.y + 52,
        col_w.saturating_sub(40),
        &rows[4..7],
    );
}

const LOG_CHIPS: [(&str, LogFilter); 4] = [
    ("ALL", LogFilter::All),
    ("INFO", LogFilter::Info),
    ("WARNING", LogFilter::Warning),
    ("ERROR", LogFilter::Error),
];

/// Severity tag of a log line (ERR, WRN or INF).
fn severity(line: &str) -> &'static str {
    let l = line.to_ascii_lowercase();
    if l.contains("error") {
        "ERR"
    } else if l.contains("warn") {
        "WRN"
    } else {
        "INF"
    }
}

fn line_visible(line: &str, filter: LogFilter) -> bool {
    let sev = severity(line);
    match filter {
        LogFilter::Error => sev == "ERR",
        LogFilter::Warning => sev != "INF",
        _ => true,
    }
}

fn draw_logs(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    // Sub-mode chips: CONSOLE | HLE TRACE
    let mut mx = area.x;
    for tab in DiagnosticsTab::ALL {
        let label = tab.label();
        let tw = text_width(label, 1) + 32;
        let r = Rect {
            x: mx,
            y: area.y,
            w: tw,
            h: 28,
        };
        let active = state.diagnostics_tab == tab;
        let bg = if active { PINK } else { CARD };
        let col = if active { BG } else { MUTED };
        fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 14, bg);
        draw_text(fb, w, h, r.x + 16, r.y + 10, label, col, 1);
        layout.diag_tabs.push((r, label));
        mx += tw + 10;
    }

    let body = Rect {
        x: area.x,
        y: area.y + 40,
        w: area.w,
        h: area.h.saturating_sub(40),
    };

    if state.diagnostics_tab == DiagnosticsTab::HleTrace {
        draw_hle_trace(fb, w, h, state, &body, layout);
        return;
    }

    let mut cx = body.x;
    let cy = body.y;
    for (label, filter) in LOG_CHIPS {
        let cw = text_width(label, 1) + 36;
        let r = Rect {
            x: cx,
            y: cy,
            w: cw,
            h: 32,
        };
        let active = match filter {
            LogFilter::All => matches!(state.log_filter, LogFilter::All | LogFilter::Debug),
            other => state.log_filter == other,
        };
        let bg = if active { ACCENT } else { CARD };
        let col = if active { BG } else { MUTED };
        fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 16, bg);
        draw_text(fb, w, h, r.x + 18, r.y + 12, label, col, 1);
        layout.filters.push((r, label));
        cx += cw + 12;
    }

    let mut y = body.y + 48;
    let bottom = body.y + body.h;
    let max_rows = (bottom.saturating_sub(y) / 26) as usize;
    let visible: Vec<&String> = state
        .console_log
        .iter()
        .rev()
        .filter(|l| line_visible(l, state.log_filter))
        .take(max_rows)
        .collect();
    if visible.is_empty() {
        let msg = if state.console_log.is_empty() {
            "NO LOG ENTRIES YET"
        } else {
            "NO ENTRIES MATCH THIS FILTER"
        };
        draw_text(fb, w, h, body.x, y, msg, MUTED, 1);
        return;
    }
    let max_chars = (body.w.saturating_sub(120) / 9) as usize;
    for line in visible {
        let sev = severity(line);
        let color = match sev {
            "ERR" => RED,
            "WRN" => YELLOW,
            _ => ACCENT,
        };
        fill_rect_rounded(fb, w, body.x, y, 46, 18, 6, color);
        draw_text(fb, w, h, body.x + 10, y + 4, sev, BG, 1);
        draw_text(
            fb,
            w,
            h,
            body.x + 62,
            y + 4,
            &truncate(line, max_chars),
            TEXT,
            1,
        );
        y += 26;
    }
}

fn draw_hle_trace(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    area: &Rect,
    layout: &mut UiLayout,
) {
    let Some(report) = &state.trace_report else {
        fill_rect_rounded(fb, w, area.x, area.y, area.w, 88, 12, PANEL);
        draw_text(
            fb,
            w,
            h,
            area.x + 20,
            area.y + 20,
            "NO HLE TRACE LOADED",
            TEXT,
            1,
        );
        draw_text(
            fb,
            w,
            h,
            area.x + 20,
            area.y + 48,
            "RUN A SYNTHETIC GUEST OR EXPORT VIA CLI: XHOLY --TRACE <EXE> [--JSON]",
            MUTED,
            1,
        );
        return;
    };

    // Summary bar: image, entry, RIP, steps, stop reason
    fill_rect_rounded(fb, w, area.x, area.y, area.w, 40, 10, PANEL);
    let summary_line = format!(
        "{} | ENTRY={:#X} | RIP={:#X} | STEPS={} | STOP={}",
        report.image.name, report.image.entry_va, report.cpu.rip, report.steps, report.stop
    );
    let max_sum = (area.w.saturating_sub(32) / 8) as usize;
    draw_text(
        fb,
        w,
        h,
        area.x + 16,
        area.y + 14,
        &truncate(&summary_line.to_ascii_uppercase(), max_sum),
        TEXT,
        1,
    );

    // Sub-panel tabs
    let mut px = area.x;
    let py = area.y + 50;
    for panel in TracePanel::ALL {
        let label = panel.label();
        let pw = text_width(label, 1) + 28;
        let r = Rect {
            x: px,
            y: py,
            w: pw,
            h: 28,
        };
        let active = state.trace_panel == panel;
        let bg = if active { ACCENT } else { CARD };
        let col = if active { BG } else { MUTED };
        fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 14, bg);
        draw_text(fb, w, h, r.x + 14, r.y + 10, label, col, 1);
        layout.trace_panels.push((r, label));
        px += pw + 10;
    }

    let content = Rect {
        x: area.x,
        y: py + 40,
        w: area.w,
        h: area.h.saturating_sub(90),
    };

    match state.trace_panel {
        TracePanel::Timeline => draw_trace_timeline(fb, w, h, state, report, &content, layout),
        TracePanel::Registers => draw_trace_registers(fb, w, h, report, &content),
        TracePanel::Imports => draw_trace_imports(fb, w, h, report, &content),
        TracePanel::HleCalls => draw_trace_hle_calls(fb, w, h, report, &content),
        TracePanel::Faults => draw_trace_faults(fb, w, h, report, &content),
    }
}

fn event_badge(event: &DiagnosticEvent) -> (&'static str, Rgba) {
    match event {
        DiagnosticEvent::Loaded { .. } => ("LOAD", ACCENT),
        DiagnosticEvent::StackInitialized { .. } => ("STCK", ACCENT),
        DiagnosticEvent::ImportResolved { state, .. } => match state {
            ImplementationState::Implemented | ImplementationState::PartiallyImplemented => {
                ("IMPT", ACCENT)
            }
            ImplementationState::Unknown => ("IMPT", YELLOW),
            ImplementationState::Unsupported => ("IMPT", RED),
        },
        DiagnosticEvent::StackArgumentsRead { .. } => ("SARG", PINK),
        DiagnosticEvent::HleDispatch { .. } => ("HLE ", PINK),
        DiagnosticEvent::CpuFault { .. } => ("CPUF", RED),
        DiagnosticEvent::AbiFault { .. } => ("ABIF", RED),
        DiagnosticEvent::Stopped { .. } => ("STOP", WHITE),
    }
}

fn draw_trace_timeline(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    report: &ExecutionReport,
    area: &Rect,
    layout: &mut UiLayout,
) {
    // Event-type filter chips
    let mut fx = area.x;
    for filter in TraceEventFilter::ALL {
        let label = filter.label();
        let fw = text_width(label, 1) + 24;
        let r = Rect {
            x: fx,
            y: area.y,
            w: fw,
            h: 26,
        };
        let active = state.trace_filter == filter;
        let bg = if active { ACCENT } else { CARD };
        let col = if active { BG } else { MUTED };
        fill_rect_rounded(fb, w, r.x, r.y, r.w, r.h, 13, bg);
        draw_text(fb, w, h, r.x + 12, r.y + 9, label, col, 1);
        layout.trace_filters.push((r, label));
        fx += fw + 8;
    }

    let list_y = area.y + 36;
    let list_h = area.h.saturating_sub(36);
    let left_w = (area.w * 58 / 100).max(240);
    let right_x = area.x + left_w + 16;
    let right_w = area.w.saturating_sub(left_w + 16);

    fill_rect_rounded(fb, w, area.x, list_y, left_w, list_h, 12, PANEL);
    fill_rect_rounded(fb, w, right_x, list_y, right_w, list_h, 12, PANEL);

    let filtered = filter_events(report, state.trace_filter, &state.search);
    if filtered.is_empty() {
        draw_text(
            fb,
            w,
            h,
            area.x + 16,
            list_y + 18,
            "NO TRACE EVENTS MATCH FILTER",
            MUTED,
            1,
        );
        draw_text(
            fb,
            w,
            h,
            right_x + 16,
            list_y + 18,
            "NO EVENT SELECTED",
            MUTED,
            1,
        );
        return;
    }

    let selected_idx = state.trace_selected.min(filtered.len() - 1);
    let max_rows = (list_h.saturating_sub(24) / 26) as usize;
    let max_chars = (left_w.saturating_sub(88) / 8) as usize;
    let mut y = list_y + 12;
    for (row_idx, (_orig_idx, event)) in filtered.iter().take(max_rows).enumerate() {
        let row_rect = Rect {
            x: area.x + 8,
            y,
            w: left_w.saturating_sub(16),
            h: 22,
        };
        if row_idx == selected_idx {
            fill_rect_rounded(
                fb, w, row_rect.x, row_rect.y, row_rect.w, row_rect.h, 6, CARD,
            );
        }
        let (badge, color) = event_badge(event);
        fill_rect_rounded(fb, w, row_rect.x + 4, y + 2, 46, 18, 5, color);
        draw_text(fb, w, h, row_rect.x + 9, y + 6, badge, BG, 1);
        draw_text(
            fb,
            w,
            h,
            row_rect.x + 58,
            y + 6,
            &truncate(&event.render_line().to_ascii_uppercase(), max_chars),
            TEXT,
            1,
        );
        layout.trace_events.push((row_rect, row_idx));
        y += 26;
    }

    // Detail panel on the right
    let (_orig, selected_event) = filtered[selected_idx];
    let mut dy = list_y + 16;
    let max_detail = (right_w.saturating_sub(32) / 8) as usize;
    for line in event_detail_lines(selected_event) {
        if dy + 20 > list_y + list_h {
            break;
        }
        draw_text(
            fb,
            w,
            h,
            right_x + 16,
            dy,
            &truncate(&line.to_ascii_uppercase(), max_detail),
            TEXT,
            1,
        );
        dy += 22;
    }
}

fn draw_trace_registers(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    report: &ExecutionReport,
    area: &Rect,
) {
    fill_rect_rounded(fb, w, area.x, area.y, area.w, area.h, 12, PANEL);
    let stack_str = match stack_bounds_from_report(report) {
        Some((bottom, top, initial_rsp)) => {
            format!("{bottom:#X}..{top:#X} (INIT={initial_rsp:#X})")
        }
        None => "UNAVAILABLE".into(),
    };
    let top_line = format!(
        "RIP={:#018X}  RFLAGS={:#X}  INSTR={}  HALTED={}",
        report.cpu.rip, report.cpu.rflags, report.cpu.instructions, report.cpu.halted
    );
    draw_text(fb, w, h, area.x + 16, area.y + 16, &top_line, ACCENT, 1);
    draw_text(
        fb,
        w,
        h,
        area.x + 16,
        area.y + 40,
        &format!("STACK BOUNDS: {stack_str}"),
        TEXT,
        1,
    );

    let col_w = (area.w.saturating_sub(32) / 4).max(160);
    for (i, val) in report.cpu.regs.iter().enumerate() {
        let col = (i % 4) as u32;
        let row = (i / 4) as u32;
        let rx = area.x + 16 + col * col_w;
        let ry = area.y + 76 + row * 28;
        if ry + 20 > area.y + area.h {
            break;
        }
        let name = register_name(i).to_ascii_uppercase();
        let line = format!("{name:>3}={val:#018X}");
        draw_text(fb, w, h, rx, ry, &line, TEXT, 1);
    }
}

fn draw_trace_imports(fb: &mut Framebuffer, w: u32, h: u32, report: &ExecutionReport, area: &Rect) {
    fill_rect_rounded(fb, w, area.x, area.y, area.w, area.h, 12, PANEL);
    let summary = import_summary(report);
    let header = format!(
        "IMPORTS: {} TOTAL (IMPL {}, PARTIAL {}, UNSUPPORTED {}, UNKNOWN {})",
        summary.total,
        summary.implemented,
        summary.partially_implemented,
        summary.unsupported,
        summary.unknown
    );
    draw_text(fb, w, h, area.x + 16, area.y + 16, &header, TEXT, 1);

    let mut y = area.y + 48;
    let max_chars = (area.w.saturating_sub(180) / 8) as usize;
    for import in &report.imports {
        if y + 24 > area.y + area.h {
            break;
        }
        let (badge, color) = match import.state {
            ImplementationState::Implemented => ("IMPL", ACCENT),
            ImplementationState::PartiallyImplemented => ("PART", ACCENT),
            ImplementationState::Unsupported => ("UNSP", RED),
            ImplementationState::Unknown => ("UNKN", YELLOW),
        };
        fill_rect_rounded(fb, w, area.x + 16, y, 52, 18, 5, color);
        draw_text(fb, w, h, area.x + 22, y + 4, badge, BG, 1);
        let slot = import
            .slot_va
            .map_or_else(|| "NO SLOT".into(), |s| format!("{s:#X}"));
        let text = format!("{}!{} [{}]", import.module, import.symbol, slot);
        draw_text(
            fb,
            w,
            h,
            area.x + 80,
            y + 4,
            &truncate(&text.to_ascii_uppercase(), max_chars),
            TEXT,
            1,
        );
        y += 26;
    }
}

fn draw_trace_hle_calls(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    report: &ExecutionReport,
    area: &Rect,
) {
    fill_rect_rounded(fb, w, area.x, area.y, area.w, area.h, 12, PANEL);
    let calls: Vec<&DiagnosticEvent> = report.dispatches().collect();
    if calls.is_empty() {
        draw_text(
            fb,
            w,
            h,
            area.x + 16,
            area.y + 18,
            "NO HLE DISPATCH EVENTS RECORDED",
            MUTED,
            1,
        );
        return;
    }
    let mut y = area.y + 16;
    let max_chars = (area.w.saturating_sub(32) / 8) as usize;
    for event in calls {
        if y + 24 > area.y + area.h {
            break;
        }
        draw_text(
            fb,
            w,
            h,
            area.x + 16,
            y,
            &truncate(&event.render_line().to_ascii_uppercase(), max_chars),
            TEXT,
            1,
        );
        y += 26;
    }
}

fn draw_trace_faults(fb: &mut Framebuffer, w: u32, h: u32, report: &ExecutionReport, area: &Rect) {
    fill_rect_rounded(fb, w, area.x, area.y, area.w, area.h, 12, PANEL);
    draw_text(
        fb,
        w,
        h,
        area.x + 16,
        area.y + 16,
        &format!("STOP REASON: {}", report.stop).to_ascii_uppercase(),
        TEXT,
        1,
    );
    let faults: Vec<&DiagnosticEvent> = report
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                DiagnosticEvent::CpuFault { .. } | DiagnosticEvent::AbiFault { .. }
            )
        })
        .collect();
    let mut y = area.y + 48;
    if faults.is_empty() {
        draw_text(
            fb,
            w,
            h,
            area.x + 16,
            y,
            "NO CPU OR ABI FAULTS RECORDED",
            MUTED,
            1,
        );
        return;
    }
    let max_chars = (area.w.saturating_sub(32) / 8) as usize;
    for fault in faults {
        if y + 24 > area.y + area.h {
            break;
        }
        draw_text(
            fb,
            w,
            h,
            area.x + 16,
            y,
            &truncate(&fault.render_line().to_ascii_uppercase(), max_chars),
            RED,
            1,
        );
        y += 26;
    }
}

fn draw_about(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState, area: &Rect) {
    let c = Rect {
        x: area.x,
        y: area.y,
        w: area.w.min(860),
        h: area.h.min(460),
    };
    card(fb, w, h, c, "ABOUT");
    draw_text(fb, w, h, c.x + 20, c.y + 50, "XHOLY", WHITE, 3);
    let version = format!("VERSION {}", env!("CARGO_PKG_VERSION"));
    draw_text(fb, w, h, c.x + 20, c.y + 104, &version, MUTED, 1);
    draw_flag_bar(fb, w, c.x + 20, c.y + 126, 220, 4, &theme::TRANS_FLAG, true);
    let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH).to_uppercase();
    let rows: Vec<(&str, String)> = vec![
        ("PLATFORM", platform),
        ("GPU", state.gpu_label.to_uppercase()),
        ("TARGET", "XBOX SERIES X|S (GEN9)".into()),
        ("HOSTS", "WINDOWS X86-64, LINUX X86-64".into()),
        ("DISCORD", "RICH PRESENCE, CONFIGURABLE".into()),
        ("REFERENCE", "SCARLETTX (SOURCE NOT COPIED)".into()),
        ("FONT", "INTER (SIL OPEN FONT LICENSE)".into()),
    ];
    kv_rows(fb, w, h, c.x + 20, c.y + 160, c.w.saturating_sub(40), &rows);
}

/// Dimmed backdrop plus centered settings dialog.
fn draw_settings_modal(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState) {
    for px in fb.as_chunks_mut::<4>().0 {
        px[0] = px[0].saturating_mul(2) / 3;
        px[1] = px[1].saturating_mul(2) / 3;
        px[2] = px[2].saturating_mul(2) / 3;
    }
    let pw = 520u32;
    let ph = 420u32;
    let px = w.saturating_sub(pw) / 2;
    let py = h.saturating_sub(ph) / 2;
    fill_rect_rounded(fb, w, px, py, pw, ph, 14, PANEL);
    draw_flag_bar(fb, w, px + 1, py + 1, pw - 2, 4, &theme::TRANS_FLAG, true);
    draw_text(fb, w, h, px + 26, py + 30, "SETTINGS", WHITE, 2);
    let rows = settings_rows(state);
    let end = kv_rows(fb, w, h, px + 26, py + 84, pw - 52, &rows);
    draw_text(fb, w, h, px + 26, end + 10, "PRESS O TO CLOSE", DIM, 1);
}
fn draw_banner(fb: &mut Framebuffer, w: u32, h: u32, state: &WindowedGuiState) {
    let y = TOP_H + HEAD_H;
    fill_rect(
        fb,
        w,
        0,
        y,
        w,
        32,
        theme::mix(PANEL, theme::BISEXUAL[0], 0.35),
    );
    let msg = format!(
        "SOFTWARE RENDERER  -  {}",
        truncate(&state.software_message.to_uppercase(), 90)
    );
    draw_text(fb, w, h, MARGIN, y + 9, &msg, TEXT, 1);
}
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

/// Emulation overlay: dimmed backdrop, RESUME and RETURN TO MAIN MENU.
fn draw_emu_menu(
    fb: &mut Framebuffer,
    w: u32,
    h: u32,
    state: &WindowedGuiState,
    layout: &mut UiLayout,
) {
    for px in fb.as_chunks_mut::<4>().0 {
        px[0] = px[0].saturating_mul(2) / 3;
        px[1] = px[1].saturating_mul(2) / 3;
        px[2] = px[2].saturating_mul(2) / 3;
    }
    let pw = 620u32;
    let ph = 360u32;
    let px = w.saturating_sub(pw) / 2;
    let py = h.saturating_sub(ph) / 2;
    fill_rect_rounded(fb, w, px, py, pw, ph, 16, PANEL);
    draw_flag_bar(fb, w, px + 1, py + 1, pw - 2, 4, &theme::TRANS_FLAG, true);
    draw_text(fb, w, h, px + 32, py + 36, "EMULATION MENU", WHITE, 2);
    let sub = match state.selected() {
        Some(game) if state.running => truncate(&game.title.to_uppercase(), 48),
        _ => "NO GAME RUNNING".to_string(),
    };
    draw_text(fb, w, h, px + 32, py + 76, &sub, MUTED, 1);

    let bw = (pw - 64 - GAP) / 2;
    let by = py + ph - 150;
    let items = [("RESUME", "PLAY"), ("MAIN MENU", "LAUNCH")];
    for (i, (label, flag_key)) in items.iter().enumerate() {
        let r = Rect {
            x: px + 32 + i as u32 * (bw + GAP),
            y: by,
            w: bw,
            h: 64,
        };
        let flag = theme::button_flag(flag_key);
        let hovered = r.contains(state.mouse.0, state.mouse.1);
        let pressed = hovered && state.mouse_down;
        draw_button(fb, w, h, r, label, flag, true, hovered, pressed, false);
        layout.menu.push((r, label));
    }
    draw_text(
        fb,
        w,
        h,
        px + 32,
        py + ph - 56,
        "CTRL + ALT + T OR ESC TO CLOSE",
        DIM,
        1,
    );
}
