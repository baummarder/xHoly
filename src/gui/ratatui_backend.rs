//! Ratatui TUI backend — the always-available GUI frontend.
//!
//! Views: game library (scrollable), game detail, settings panel,
//! performance toolbar. Hotkeys: ESC menu, P pause, F1 screenshot,
//! F5 save state, F9 load state, Tab/Shift-Tab multi-game tabs.

use super::preview;
use super::settings::GuiSettings;
use super::terminal::{enter_alternate_screen, leave_alternate_screen};
use super::window::WindowManager;
use super::{GameListItem, GuiConfig, GuiState};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Which panel is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Library,
    Detail,
    Settings,
    Menu,
}

/// Run the TUI until the user quits (ESC in menu / Ctrl-C / q in menu).
pub fn run(config: GuiConfig) -> io::Result<()> {
    let settings = GuiSettings::load_default();
    let mut config = config;
    settings.apply(&mut config);

    let mut state = GuiState {
        games: scan_games(),
        ..Default::default()
    };

    let _window = WindowManager::new(&config.window_title, config.window_size);
    let mut view = View::Library;
    let mut scroll: u16 = 0;
    let mut last_tick = Instant::now();
    let mut frame_count = 0u32;
    let mut fps_ema = 0.0f32;

    enter_alternate_screen()?;
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;

    let result = (|| -> io::Result<()> {
        loop {
            // ~30 Hz UI refresh (also paces the FPS counter)
            if last_tick.elapsed() < Duration::from_millis(33) {
                if event::poll(Duration::from_millis(5))? {
                    // fall through to event handling below
                } else {
                    continue;
                }
            }
            last_tick = Instant::now();
            frame_count += 1;
            fps_ema = if fps_ema == 0.0 {
                30.0
            } else {
                fps_ema * 0.9 + 30.0 * 0.1
            };
            state.fps = fps_ema;

            terminal.draw(|f| draw(f, &state, &config, view, scroll))?;

            if !event::poll(Duration::from_millis(1))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }

            match view {
                View::Menu => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => {
                        state.running = false;
                        state.paused = false;
                        view = View::Library;
                    }
                    KeyCode::Char('s') => {
                        let _ = GuiSettings::from_config(&config).save_default();
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                },
                View::Settings => match key.code {
                    KeyCode::Esc => view = View::Library,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                },
                View::Library | View::Detail => match key.code {
                    KeyCode::Esc => view = View::Menu,
                    KeyCode::Char('o') | KeyCode::Char('O') => view = View::Settings,
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        view = if view == View::Detail {
                            View::Library
                        } else {
                            View::Detail
                        };
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        state.paused = !state.paused;
                        state.running = state.paused || state.running;
                    }
                    KeyCode::F(1) => screenshot(&state),
                    KeyCode::F(5) => save_state(&state),
                    KeyCode::F(9) => load_state(&state),
                    KeyCode::Tab => state.next_tab(),
                    KeyCode::BackTab => state.prev_tab(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        state.select_next();
                        scroll = scroll.saturating_add(1);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.select_prev();
                        scroll = scroll.saturating_sub(1);
                    }
                    KeyCode::Enter => {
                        let path = state.selected().map(|g| g.path.clone());
                        if let Some(p) = path {
                            launch_game(&mut state, &p);
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                },
            }
        }
    })();

    let _ = leave_alternate_screen();
    result
}

/// Scan optical drives (and an optional local `games/` dir) for games.
/// Best-effort: any drive that fails to mount or scan is skipped.
fn scan_games() -> Vec<GameListItem> {
    use crate::optical::{drive, game_scanner};

    let mut items = Vec::new();
    let mut sources: Vec<PathBuf> = Vec::new();

    // Local drop-in folder first (easiest way to populate the library).
    let local = PathBuf::from("games");
    if local.is_dir() {
        sources.push(local);
    }

    // Detected physical/virtual optical drives.
    for info in drive::detect_drives() {
        sources.push(info.mount_point);
    }

    for path in sources {
        let mut drive = drive::VirtualDrive::new(path.display().to_string());
        if drive.mount_path(&path).is_err() {
            continue;
        }
        let Some(media) = drive.media() else { continue };
        let Ok(games) = game_scanner::scan(media.filesystem()) else {
            continue;
        };
        // Group multi-disc titles so the disc count is accurate.
        // Resolve payload paths against the host source so Launch can
        // hand a real filesystem path to the game runner: directories
        // map to `root/<payload>`, ISO files map to the ISO itself
        // (run_path dispatches to run_iso for those).
        let host_is_dir = path.is_dir();
        for group in game_scanner::group_multidisc(&games) {
            let discs = group.len().min(u8::MAX as usize) as u8;
            for g in group {
                let mut item = GameListItem::from_scanned(g, discs);
                item.path = if host_is_dir {
                    if g.payload_path.is_empty() {
                        path.clone()
                    } else {
                        path.join(&g.payload_path)
                    }
                } else {
                    path.clone()
                };
                items.push(item);
            }
        }
    }
    items
}

/// Launch the selected game through the real loader→kernel→ARM64
/// interpreter path and capture its console output into the GUI log.
/// Bounded step budget keeps the TUI responsive on the scaffold.
fn launch_game(state: &mut GuiState, path: &std::path::Path) {
    state.running = true;
    push_log(state, &format!("xHoly: launching {}", path.display()));
    match crate::core::loader::game_runner::run_path(path, 500_000) {
        Ok(outcome) => {
            let text = String::from_utf8_lossy(&outcome.report.output);
            for line in text.lines() {
                push_log(state, line);
            }
            for line in &outcome.report.kernel_log {
                push_log(state, &format!("kernel: {line}"));
            }
            for (shader, warnings) in &outcome.shader_results {
                for w in warnings {
                    push_log(state, &format!("shader {shader}: {w}"));
                }
            }
            push_log(
                state,
                &format!(
                    "xHoly: guest exited after {} steps ({} threads)",
                    outcome.report.steps, outcome.report.threads
                ),
            );
        }
        Err(e) => push_log(state, &format!("xHoly: launch failed: {e}")),
    }
    state.running = false;
}

/// Append a line to the console log, keeping the last 200 entries.
fn push_log(state: &mut GuiState, line: &str) {
    state.console_log.push(line.to_string());
    if state.console_log.len() > 200 {
        let excess = state.console_log.len() - 200;
        state.console_log.drain(..excess);
    }
}

fn screenshot(state: &GuiState) {
    if let Some(g) = state.selected() {
        let px = preview::render_preview(g);
        let _ = std::fs::create_dir_all("screenshots");
        let path = format!("screenshots/xholy-{}.rgba", sanitize(&g.title));
        if std::fs::write(&path, &px).is_ok() {
            eprintln!("xHoly: screenshot -> {path} ({} bytes)", px.len());
        }
    }
}

fn save_state(state: &GuiState) {
    eprintln!(
        "xHoly: save state requested (game running={})",
        state.running
    );
}

fn load_state(state: &GuiState) {
    eprintln!("xHoly: load state requested (paused={})", state.paused);
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

fn gold() -> Style {
    Style::default().fg(Color::Rgb(0xFF, 0xD7, 0x00))
}

fn dim_gold() -> Style {
    Style::default().fg(Color::Rgb(0x8A, 0x72, 0x00))
}

fn draw(f: &mut Frame, state: &GuiState, config: &GuiConfig, view: View, scroll: u16) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    // Toolbar: title + perf stats
    let stats = format!(
        " {} | {} games | {} | {:.0} fps | cpu {:.0}% | gpu {:.0}% ",
        config.window_title,
        state.games.len(),
        if state.paused {
            "PAUSED"
        } else if state.running {
            "RUNNING"
        } else {
            "IDLE"
        },
        state.fps,
        state.cpu_usage * 100.0,
        state.gpu_usage * 100.0,
    );
    let toolbar = Paragraph::new(Line::from(vec![
        Span::styled("xHoly", gold().add_modifier(Modifier::BOLD)),
        Span::styled(stats, Style::default().fg(Color::DarkGray)),
    ]))
    .style(Style::default().bg(Color::Rgb(0x1A, 0x1A, 0x1A)));
    f.render_widget(toolbar, chunks[0]);

    match view {
        View::Menu => draw_menu(f, chunks[1]),
        View::Settings => draw_settings(f, chunks[1], config),
        View::Detail => draw_detail(f, state, chunks[1], config),
        _ => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(chunks[1]);
            draw_library(f, state, cols[0], scroll);
            draw_detail(f, state, cols[1], config);
        }
    }

    // Footer: hotkey hints
    let hints = match view {
        View::Menu => " ESC/q resume · R restart · S save settings · Ctrl-C quit ".to_string(),
        _ => format!(
            " ESC menu · P pause · F1 screenshot · F5 save · F9 load · Tab games · j/k move · Enter launch · {}",
            if state.cheat_engine_enabled { "CHEATS ON" } else { "cheats off" }
        ),
    };
    f.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn draw_library(f: &mut Frame, state: &GuiState, area: Rect, scroll: u16) {
    let items: Vec<ListItem> = state
        .games
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let style = if i == state.selected_game {
                gold().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let discs = if g.is_multi_disc {
                format!(" [x{}]", g.discs)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<28}", g.title), style),
                Span::styled(format!("{}{}", g.region, discs), dim_gold()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(dim_gold())
                .title(Span::styled(" Game Library ", gold())),
        )
        .highlight_style(gold().bg(Color::Rgb(0x33, 0x33, 0x33)));

    let mut list_state =
        ratatui::widgets::ListState::default().with_selected(Some(state.selected_game));
    // Keep the selection visible by centering the offset around it.
    let visible = area.height.saturating_sub(2) as usize;
    let offset = state
        .selected_game
        .saturating_sub(visible / 2)
        .min(state.games.len().saturating_sub(visible));
    *list_state.offset_mut() = offset;
    let _ = scroll;
    f.render_stateful_widget(list, area, &mut list_state);
}

fn draw_detail(f: &mut Frame, state: &GuiState, area: Rect, config: &GuiConfig) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(dim_gold())
        .title(Span::styled(" Game Detail ", gold()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(g) = state.selected() else {
        f.render_widget(
            Paragraph::new("No games found.\nInsert a disc or check optical drives.")
                .style(Style::default().fg(Color::Gray))
                .block(Block::default().borders(Borders::NONE)),
            inner,
        );
        return;
    };

    let lines = vec![
        Line::from(Span::styled(&g.title, gold().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(format!("Path:    {}", g.path.display())),
        Line::from(format!("Region:  {}", g.region)),
        Line::from(format!(
            "Discs:   {}{}",
            g.discs,
            if g.is_multi_disc { " (multi-disc)" } else { "" }
        )),
        Line::from(format!(
            "Released:{}",
            g.release_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| " unknown".into())
        )),
        Line::from(format!(
            "Last played: {}",
            g.last_played
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".into())
        )),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] launch  [P] pause  [F5] save  [F9] load",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let mut lines = lines;
    if !state.console_log.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Console ",
            gold().add_modifier(Modifier::BOLD),
        )));
        for l in state.console_log.iter().rev().take(12).rev() {
            lines.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(Color::Gray),
            )));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    let _ = config;
}

fn draw_menu(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            " xHoly Menu ",
            gold().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(" R  resume emulation"),
        Line::from(" S  save GUI settings"),
        Line::from(" ESC/q  quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Phase 4: library/detail/TUI active. wgpu windowed shell pending.",
            dim_gold(),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(dim_gold())
                .title(Span::styled(" Menu ", gold())),
        ),
        area,
    );
}

fn draw_settings(f: &mut Frame, area: Rect, config: &GuiConfig) {
    let lines = vec![
        Line::from(Span::styled(
            " Settings ",
            gold().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(" Backend:        {:?}", config.backend)),
        Line::from(format!(
            " Window:         {}x{}",
            config.window_size.0, config.window_size.1
        )),
        Line::from(format!(" HiDPI:          {}", config.high_dpi)),
        Line::from(format!(" FPS limit:      {:?}", config.fps_limit)),
        Line::from(format!(" Software rend: {}", config.software_rendering)),
        Line::from(format!(" Show FPS:       {}", config.show_fps)),
        Line::from(format!(" Show CPU:       {}", config.show_cpu_usage)),
        Line::from(format!(" Theme:          {:?}", config.theme)),
        Line::from(""),
        Line::from(Span::styled(
            "(read-only in TUI; edit ~/.config/xholy/gui.toml)",
            dim_gold(),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(dim_gold())
                .title(Span::styled(" Settings ", gold())),
        ),
        area,
    );
}
