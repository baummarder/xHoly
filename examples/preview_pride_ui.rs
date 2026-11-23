//! Render a still PNG of the current windowed UI theme.
//!
//! Run: `cargo run --example preview_pride_ui`
//! Writes `target/xholy_pride_ui.png`.

use std::path::{Path, PathBuf};

use xholy::gui::windowed::state::WindowedGuiState;
use xholy::gui::windowed::ui;
use xholy::gui::GameListItem;

fn game(title: &str, region: &str) -> GameListItem {
    GameListItem {
        title: title.into(),
        path: PathBuf::from(format!("games/{title}")),
        cover_art: None,
        region: region.into(),
        discs: 1,
        release_date: None,
        is_multi_disc: false,
        last_played: None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = WindowedGuiState {
        resolution: (1280, 720),
        preview_size: (480, 270),
        fps: 60.0,
        cpu_usage: 0.24,
        gpu_usage: 0.58,
        memory_usage: 412 * 1_048_576,
        gpu_label: "Vulkan · RTX 4070".into(),
        fps_limit: Some(120),
        games: vec![
            game("xHoly Minigame", "NTSC-U"),
            game("Xbox Research Demo", "PAL"),
            game("Guest Frame Test", "NTSC-J"),
        ],
        console_log: vec![
            "xHoly: launching games/minigame".into(),
            "xHoly: guest frame 42 after 28432 steps".into(),
        ],
        ..Default::default()
    };
    // Park the mouse over LAUNCH to show the hover treatment.
    state.mouse = (270, 18);

    let (fb, _layout) = ui::composite(&state, 1280, 720);
    let out = Path::new("target/xholy_pride_ui.png");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    xholy::io::screenshot::write_png(out, &fb, 1280, 720)?;
    println!("wrote {}", out.display());
    Ok(())
}
