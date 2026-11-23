//! Integration tests for the xHoly GUI subsystem.

use xholy::gui::windowed::input::{GuiAction, InputState};
use xholy::gui::windowed::state::WindowedGuiState;
use xholy::gui::windowed::ui;
use xholy::gui::{GameListItem, GpuApiPreference, GuiBackend, GuiConfig, GuiTheme};
use xholy::theme;

fn sample_game(title: &str) -> GameListItem {
    GameListItem {
        title: title.into(),
        path: "games/test".into(),
        cover_art: None,
        region: "NTSC-U".into(),
        discs: 1,
        release_date: None,
        is_multi_disc: false,
        last_played: None,
    }
}

#[test]
fn gui_config_defaults() {
    let c = GuiConfig::default();
    assert_eq!(c.backend, GuiBackend::Ratatui);
    assert_eq!(c.theme, GuiTheme::Dark);
    assert_eq!(c.fps_limit, Some(120));
    assert_eq!(c.preferred_api, GpuApiPreference::Auto);
}

#[test]
fn theme_parse() {
    assert_eq!(GuiTheme::parse("dark"), Some(GuiTheme::Dark));
    assert_eq!(GuiTheme::parse("GOLD"), Some(GuiTheme::Gold));
    assert_eq!(GuiTheme::parse("nope"), None);
}

#[test]
fn windowed_state_navigation_and_toggles() {
    let mut s = WindowedGuiState {
        games: vec![sample_game("A"), sample_game("B"), sample_game("C")],
        ..Default::default()
    };
    assert_eq!(s.selected().unwrap().title, "A");
    s.select_next();
    assert_eq!(s.selected().unwrap().title, "B");
    s.select_prev();
    s.select_prev(); // wraps to C
    assert_eq!(s.selected().unwrap().title, "C");
    s.toggle_pause();
    assert!(s.paused && s.running);
    s.toggle_cheats();
    assert!(s.cheat_engine_enabled);
}

#[test]
fn ui_composite_with_games_draws_rows() {
    let s = WindowedGuiState {
        games: vec![sample_game("TEST GAME")],
        fps: 120.0,
        gpu_label: "Vulkan · Test GPU".into(),
        ..Default::default()
    };
    let (fb, layout) = ui::composite(&s, 800, 500);
    assert_eq!(fb.len(), 800 * 500 * 4);
    assert_eq!(layout.rows.len(), 1);
    assert!(!layout.buttons.is_empty());
}

#[test]
fn settings_panel_renders_when_open() {
    let s = WindowedGuiState {
        settings_open: true,
        fps_limit: Some(60),
        ..Default::default()
    };
    let (fb, _) = ui::composite(&s, 800, 500);
    // Panel interior should use the new blue-black panel surface.
    let cx = 400usize;
    let cy = 250usize;
    let i = (cy * 800 + cx) * 4;
    assert_eq!(&fb[i..i + 3], &theme::PANEL[..3]);
}

#[test]
fn software_warning_banner_shows() {
    let s = WindowedGuiState {
        software_mode: true,
        software_message: "Running on software renderer".into(),
        ..Default::default()
    };
    let (fb, _) = ui::composite(&s, 800, 500);
    // Banner row (just under the toolbar) is a bisexual-pink tint.
    let i = (40 * 800 + 400) * 4;
    let expected = theme::mix(theme::PANEL, theme::BISEXUAL[0], 0.35);
    assert_eq!(&fb[i..i + 3], &expected[..3]);
}

#[test]
fn input_scroll_maps_to_navigation() {
    let mut input = InputState::new();
    input.mouse_scroll(winit::event::MouseScrollDelta::LineDelta(0.0, -1.0));
    let actions = input.drain_actions();
    assert!(actions.contains(&GuiAction::NextGame));
}

#[test]
fn hit_test_button_rects() {
    let s = WindowedGuiState::default();
    let (_fb, layout) = ui::composite(&s, 800, 500);
    // First toolbar button should contain its own center point.
    let (rect, _) = &layout.buttons[0];
    assert!(rect.contains(rect.x + rect.w / 2, rect.y + rect.h / 2));
}
