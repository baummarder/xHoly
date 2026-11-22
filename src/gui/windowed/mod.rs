//! Windowed GUI — Phase 5 (winit + wgpu).
//!
//! Real windowed application: winit event loop → software UI composite
//! → wgpu texture upload → fullscreen blit to the swapchain.
//!
//! [`run_windowed`] is the windowed entry point; [`run_headless`] runs
//! the same pipeline offscreen (no window) for tests and
//! `--gui-headless`.

pub mod input;
pub mod preview;
pub mod renderer;
pub mod state;
pub mod ui;
pub mod window;

use crate::gui::gpu_api_detector::{self, GpuApi, GpuApiPreference};
use crate::gui::renderer::GuiFrameRenderer;
use crate::gui::{GameListItem, GuiConfig};
use input::{GuiAction, InputState};
use state::WindowedGuiState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

/// The windowed GUI application.
pub struct WindowApp {
    config: GuiConfig,
    state: WindowedGuiState,
    input: InputState,
    window: Option<Arc<winit::window::Window>>,
    renderer: Option<Box<dyn GuiFrameRenderer>>,
    instance: wgpu::Instance,
    /// Detected GPU API (drives backend wrapping + info panel).
    gpu_api: GpuApi,
    last_tick: Instant,
    fps_ema: f32,
    /// Set when the app should exit.
    pub should_exit: bool,
}

impl WindowApp {
    /// Create the app (window + renderer are created in [`Self::resumed`]).
    ///
    /// `backends` restricts the wgpu instance to the detected GPU API.
    pub fn new(config: GuiConfig, backends: wgpu::Backends, gpu_api: GpuApi) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let state = WindowedGuiState {
            games: scan_games_windowed(),
            ..Default::default()
        };
        Self {
            config,
            state,
            input: InputState::new(),
            window: None,
            renderer: None,
            instance,
            gpu_api,
            last_tick: Instant::now(),
            fps_ema: 0.0,
            should_exit: false,
        }
    }

    /// Headless constructor: app state without any window/surface.
    pub fn new_headless(config: GuiConfig, gpu_api: GpuApi) -> Self {
        let mut app = Self::new(config, gpu_api_detector::backends_for(gpu_api), gpu_api);
        app.window = None;
        app
    }

    /// Process one composited frame through the renderer.
    /// Returns Ok(()) on success.
    pub fn render_frame(&mut self) -> Result<(), String> {
        let (w, h) = self.state.resolution;
        let (fb, _layout) = ui::composite(&self.state, w, h);
        if let Some(r) = self.renderer.as_mut() {
            r.render_frame(&fb, w, h)?;
        }
        self.tick_fps();
        Ok(())
    }

    fn tick_fps(&mut self) {
        let dt = self.last_tick.elapsed().as_secs_f32();
        self.last_tick = Instant::now();
        let fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        self.fps_ema = if self.fps_ema == 0.0 {
            fps
        } else {
            self.fps_ema * 0.9 + fps * 0.1
        };
        self.state.fps = self.fps_ema;
    }

    /// Apply queued input actions to the state.
    fn apply_actions(&mut self) {
        for action in self.input.drain_actions() {
            match action {
                GuiAction::Pause => self.state.toggle_pause(),
                GuiAction::Screenshot => self.screenshot(),
                GuiAction::SaveState => self.save_state(),
                GuiAction::LoadState => self.load_state(),
                GuiAction::Eject => eprintln!("xHoly: eject requested"),
                GuiAction::ToggleCheats => self.state.toggle_cheats(),
                GuiAction::Launch => self.launch(),
                GuiAction::NextGame => self.state.select_next(),
                GuiAction::PrevGame => self.state.select_prev(),
                GuiAction::ToggleFullscreen => self.toggle_fullscreen(),
                GuiAction::Quit => self.should_exit = true,
            }
        }
    }

    fn screenshot(&self) {
        if let Some(g) = self.state.selected() {
            let px = preview::render_game_preview(g, 320, 180);
            let _ = std::fs::create_dir_all("screenshots");
            let name = g
                .title
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect::<String>();
            let path = format!("screenshots/xholy-{name}.rgba");
            if std::fs::write(&path, &px).is_ok() {
                eprintln!("xHoly: screenshot -> {path}");
            }
        }
    }

    fn save_state(&mut self) {
        let slot = self.state.save_slots.len() as u8;
        self.state.save_slots.push(state::SaveState {
            slot,
            written_at: std::time::SystemTime::now(),
            description: self
                .state
                .selected()
                .map(|g| g.title.clone())
                .unwrap_or_default(),
        });
        eprintln!("xHoly: save state -> slot {slot}");
    }

    fn load_state(&self) {
        eprintln!("xHoly: load state requested");
    }

    fn launch(&self) {
        if let Some(g) = self.state.selected() {
            eprintln!("xHoly: launch {} ({})", g.title, g.path.display());
        }
    }

    fn toggle_fullscreen(&self) {
        if let Some(w) = &self.window {
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
            } else {
                w.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
        }
    }
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match window::create_window(
            event_loop,
            &self.config.window_title,
            self.state.resolution,
            self.config.fullscreen,
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("xHoly: {e}");
                self.should_exit = true;
                return;
            }
        };
        let surface = match window::create_surface(&self.instance, &window) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("xHoly: {e}");
                self.should_exit = true;
                return;
            }
        };
        let size = window.inner_size();
        let wgpu_renderer = match renderer::WgpuRenderer::new_surface(
            &self.instance,
            surface,
            size.width,
            size.height,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("xHoly: renderer init failed: {e}");
                self.should_exit = true;
                return;
            }
        };
        // Wrap in the GL-fallback type when the detected API is OpenGL.
        let boxed: Box<dyn GuiFrameRenderer> = match self.gpu_api {
            GpuApi::OpenGL => Box::new(crate::gui::gl_backend::GlBackend::new(wgpu_renderer)),
            _ => Box::new(wgpu_renderer),
        };
        self.state.resolution = (size.width, size.height);
        self.window = Some(window);
        self.renderer = Some(boxed);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.should_exit = true;
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.state.resolution = (size.width, size.height);
                    if let Some(r) = self.renderer.as_mut() {
                        r.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let ke: KeyEvent = event;
                self.input.keyboard_input(&ke);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.input.mouse_input(button, state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_motion(position.x, position.y);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.mouse_scroll(delta);
            }
            WindowEvent::RedrawRequested => {
                // Click hit-testing against the last composited layout.
                self.handle_click();
                self.apply_actions();
                if let Err(e) = self.render_frame() {
                    eprintln!("xHoly: render error: {e}");
                }
                if self.should_exit {
                    event_loop.exit();
                } else if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

impl WindowApp {
    /// Map mouse position to UI actions on click (buttons + list rows).
    fn handle_click(&mut self) {
        if !self.input.mouse_down {
            return;
        }
        let (w, h) = self.state.resolution;
        let (_fb, layout) = ui::composite(&self.state, w, h);
        let (mx, my) = self.input.mouse;
        let (mx, my) = (mx as u32, my as u32);

        // Toolbar buttons
        for (rect, label) in &layout.buttons {
            if rect.contains(mx, my) {
                match *label {
                    "PLAY" | "PAUSE" => self.state.toggle_pause(),
                    "EJECT" => eprintln!("xHoly: eject requested"),
                    "SAVE" => self.save_state(),
                    "LOAD" => self.load_state(),
                    "CHEATS" => self.state.toggle_cheats(),
                    _ => {}
                }
                self.input.mouse_down = false;
                return;
            }
        }
        // Sidebar rows
        for (rect, idx) in &layout.rows {
            if rect.contains(mx, my) {
                self.state.selected_game = *idx;
                self.input.mouse_down = false;
                return;
            }
        }
        // Preview area click = launch
        if layout.preview.contains(mx, my) {
            self.launch();
        }
    }
}

/// Run the windowed GUI (blocks until the window closes).
///
/// `pref` selects the GPU API: `Auto` probes Vulkan → OpenGL →
/// Software; explicit values force that backend.
pub fn run_windowed(config: GuiConfig, pref: GpuApiPreference) -> Result<(), String> {
    let detection = gpu_api_detector::detect_with_preference(pref);
    gpu_api_detector::save_detection(&detection);
    eprintln!(
        "xHoly: GPU API = {} ({})",
        detection.describe(),
        detection.backend
    );

    // Software has no presentable backend — try GL once more for the
    // surface path, else tell the user to use the TUI.
    let backends = if detection.api == GpuApi::Software {
        wgpu::Backends::GL
    } else {
        gpu_api_detector::backends_for(detection.api)
    };
    if backends.is_empty() {
        return Err(
            "no GPU API available for windowed mode; try --gui-tui or --gui-software".into(),
        );
    }

    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = WindowApp::new(config, backends, detection.api);
    app.state.gpu_label = detection.describe();
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop: {e}"))?;
    Ok(())
}

/// Run the GUI headless: composite + render `frames` frames offscreen.
///
/// Used by `--gui-headless` and tests. With `GpuApi::Software` (or a
/// failed GPU probe under `Auto`) frames are composited on the CPU via
/// [`software_renderer::SoftwareRenderer`]; otherwise the wgpu
/// offscreen path is used. Returns the last composited framebuffer.
pub fn run_headless(
    config: GuiConfig,
    frames: u32,
    pref: GpuApiPreference,
) -> Result<Vec<u8>, String> {
    let detection = gpu_api_detector::detect_with_preference(pref);
    eprintln!(
        "xHoly: GPU API = {} ({})",
        detection.describe(),
        detection.backend
    );
    let (w, h) = config.window_size;
    let mut app = WindowApp::new_headless(config, detection.api);
    app.state.resolution = (w, h);
    app.state.gpu_label = detection.describe();

    // Software path: no GPU at all.
    if detection.api == GpuApi::Software {
        let mut sw = crate::gui::software_renderer::SoftwareRenderer::new(w, h);
        let mut last_fb = Vec::new();
        for _ in 0..frames.max(1) {
            let (fb, _layout) = ui::composite(&app.state, w, h);
            sw.render_frame(&fb, w, h)?;
            last_fb = fb;
            app.tick_fps();
            pace(app.config.fps_limit);
        }
        return Ok(last_fb);
    }

    app.renderer = Some(Box::new(
        renderer::WgpuRenderer::new_offscreen_with_backends(
            w,
            h,
            gpu_api_detector::backends_for(detection.api),
        )?,
    ));

    let mut last_fb = Vec::new();
    for _ in 0..frames.max(1) {
        let (fb, _layout) = ui::composite(&app.state, w, h);
        last_fb = fb.clone();
        if let Some(r) = app.renderer.as_mut() {
            r.render_frame(&fb, w, h)?;
        }
        app.tick_fps();
        pace(app.config.fps_limit);
    }
    Ok(last_fb)
}

/// Rough frame pacing for the headless loop.
fn pace(fps_limit: Option<u32>) {
    if let Some(limit) = fps_limit {
        if limit > 0 {
            std::thread::sleep(Duration::from_millis(1000 / limit as u64));
        }
    }
}

/// Scan games for the windowed library (same sources as the TUI).
fn scan_games_windowed() -> Vec<GameListItem> {
    use crate::optical::{drive, game_scanner};
    let mut items = Vec::new();
    let mut sources: Vec<std::path::PathBuf> = Vec::new();
    let local = std::path::PathBuf::from("games");
    if local.is_dir() {
        sources.push(local);
    }
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
        for group in game_scanner::group_multidisc(&games) {
            let discs = group.len().min(u8::MAX as usize) as u8;
            for g in group {
                items.push(GameListItem::from_scanned(g, discs));
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_navigation() {
        let mut s = WindowedGuiState {
            games: vec![
                GameListItem {
                    title: "A".into(),
                    path: "a".into(),
                    cover_art: None,
                    region: "NTSC-U".into(),
                    discs: 1,
                    release_date: None,
                    is_multi_disc: false,
                    last_played: None,
                },
                GameListItem {
                    title: "B".into(),
                    path: "b".into(),
                    cover_art: None,
                    region: "PAL".into(),
                    discs: 1,
                    release_date: None,
                    is_multi_disc: false,
                    last_played: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(s.selected().unwrap().title, "A");
        s.select_next();
        assert_eq!(s.selected().unwrap().title, "B");
        s.select_next(); // wraps
        assert_eq!(s.selected().unwrap().title, "A");
        s.toggle_pause();
        assert!(s.paused);
        s.toggle_cheats();
        assert!(s.cheat_engine_enabled);
    }

    #[test]
    fn ui_composite_produces_framebuffer() {
        let state = WindowedGuiState::default();
        let (fb, layout) = ui::composite(&state, 640, 400);
        assert_eq!(fb.len(), 640 * 400 * 4);
        assert!(!layout.buttons.is_empty());
        // Sidebar pixel should be panel color #262626.
        let i = (100 * 640 + 10) * 4;
        assert_eq!(&fb[i..i + 3], &[0x26, 0x26, 0x26]);
    }

    #[test]
    fn headless_render_smoke() {
        let config = GuiConfig {
            window_size: (320, 200),
            fps_limit: None,
            ..Default::default()
        };
        let fb = run_headless(config, 2, GpuApiPreference::Auto).expect("headless render");
        assert_eq!(fb.len(), 320 * 200 * 4);
    }
}
