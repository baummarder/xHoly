//! Windowed GUI — Phase 5 (winit + wgpu).
//!
//! Real windowed application: winit event loop → software UI composite
//! → wgpu texture upload → fullscreen blit to the swapchain.
//!
//! [`run_windowed`] is the windowed entry point; [`run_headless`] runs
//! the same pipeline offscreen (no window) for tests and
//! `--gui-headless`.

pub mod frame_buffer;
pub mod input;
pub mod postfx;
pub mod preview;
pub mod renderer;
pub mod state;
pub mod ui;
pub mod window;

use crate::core::execution::{GuestInput, GuestSession};
use crate::gui::gpu_api_detector::{self, GpuApi, GpuApiPreference};
use crate::gui::native_gpu_backend::NativeGpuBackend;
use crate::gui::renderer::GuiFrameRenderer;
use crate::gui::{FramePacer, GameListItem, GuiConfig};
use crate::presence::{ConsoleGeneration, NoopPresence, PresenceSink, PresenceState};
use input::{GuiAction, InputState};
use state::{GuiView, WindowedGuiState};
use std::sync::Arc;
use std::time::Instant;
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
    /// Consecutive frames below half the FPS limit (Phase-9 perf log).
    slow_frames: u32,
    /// Frame-rate limiter shared by windowed + headless paths.
    pacer: FramePacer,
    /// Persistent guest session while a game is running (Phase 22).
    /// Boxed so the kernel's self-pointers stay valid.
    session: Option<Box<GuestSession>>,
    /// Shared Discord presence sink (no-op when not configured).
    presence: Arc<dyn PresenceSink>,
    /// Set when the app should exit.
    pub should_exit: bool,
}

impl WindowApp {
    /// Create the app (window + renderer are created in [`Self::resumed`]).
    ///
    /// `backends` restricts the wgpu instance to the detected GPU API.
    pub fn new(config: GuiConfig, backends: wgpu::Backends, gpu_api: GpuApi) -> Self {
        let presence: Arc<dyn PresenceSink> = config
            .presence
            .clone()
            .unwrap_or_else(|| Arc::new(NoopPresence));
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let state = WindowedGuiState {
            games: scan_games_windowed(),
            fps_limit: config.fps_limit,
            vsync: config.vsync,
            fullscreen: config.fullscreen,
            ..Default::default()
        };
        let pacer = FramePacer::with_mode(
            config.fps_limit.unwrap_or(0),
            config.vsync,
            config.frame_pacing_mode,
        );
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
            slow_frames: 0,
            pacer,
            session: None,
            presence,
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
    ///
    /// Honors `fps_limit`/`vsync` via [`FramePacer`]: records the frame
    /// start, renders, then sleeps until the next frame deadline so the
    /// loop holds a steady cadence instead of spinning uncapped.
    pub fn render_frame(&mut self) -> Result<(), String> {
        self.pacer.record_frame_start();
        // Advance the persistent guest by one frame before presenting.
        self.tick_guest();
        // While a guest is running its framebuffer goes straight to the
        // GPU renderer — no software UI composite in the guest path.
        let guest_frame = if self.state.running && !self.state.emu_menu_open {
            self.state.guest_frame.clone()
        } else {
            None
        };
        if let (Some(frame), Some(renderer)) = (guest_frame.as_ref(), self.renderer.as_mut()) {
            renderer.render_guest_frame(frame)?;
            self.tick_fps();
            self.pacer.sleep_until_next_frame();
            return Ok(());
        }
        let (w, h) = self.state.resolution;
        let (fb, _layout) = ui::composite(&self.state, w, h);
        if let Some(r) = self.renderer.as_mut() {
            r.render_frame(&fb, w, h)?;
        }
        self.tick_fps();
        self.pacer.sleep_until_next_frame();
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

        // Phase-9 perf monitoring: log sustained frame drops.
        if let Some(limit) = self.state.fps_limit {
            if self.fps_ema > 0.0 && self.fps_ema < limit as f32 * 0.5 {
                self.slow_frames += 1;
                if self.slow_frames == 120 {
                    eprintln!(
                        "xHoly: perf warning — sustained {}/{} FPS",
                        self.fps_ema as u32, limit
                    );
                }
            } else {
                self.slow_frames = 0;
            }
        }
    }

    /// Apply queued input actions to the state.
    fn apply_actions(&mut self) {
        for action in self.input.drain_actions() {
            match action {
                GuiAction::Pause => {
                    if self.state.emu_menu_open {
                        self.state.emu_menu_open = false;
                    } else {
                        self.state.toggle_pause();
                    }
                }
                GuiAction::EmulationMenu => self.state.emu_menu_open = !self.state.emu_menu_open,
                GuiAction::Screenshot => self.screenshot(),
                GuiAction::SaveState => self.save_state(),
                GuiAction::LoadState => self.load_state(),
                GuiAction::Eject => eprintln!("xHoly: eject requested"),
                GuiAction::ToggleCheats => self.state.toggle_cheats(),
                GuiAction::Launch => self.launch(),
                GuiAction::NextGame => self.state.select_next(),
                GuiAction::PrevGame => self.state.select_prev(),
                GuiAction::ToggleFullscreen => self.toggle_fullscreen(),
                GuiAction::ToggleSettings => self.state.settings_open = !self.state.settings_open,
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

    /// Launch the selected game through the real loader → ARM64
    /// interpreter → kernel path (same as the TUI). Guest console
    /// output lands in `state.console_log`; a written guest framebuffer
    /// frame lands in `state.guest_frame` and is blitted by the UI.
    /// When the guest FB stays empty the previous frame is kept.
    fn launch(&mut self) {
        let Some(path) = self.state.selected().map(|g| g.path.clone()) else {
            return;
        };
        self.push_log(&format!("xHoly: launching {}", path.display()));
        // Only the visible title is reported — never the local path.
        let game_name = self
            .state
            .selected()
            .map(|g| g.title.clone())
            .unwrap_or_else(|| "a game".into());
        self.presence.update(PresenceState::Loading {
            game: game_name.clone(),
        });
        let image = match crate::core::loader::load_game(&path) {
            Ok(image) => image,
            Err(e) => {
                self.push_log(&format!("xHoly: launch failed: {e}"));
                self.presence.update(PresenceState::MainMenu);
                return;
            }
        };
        let mut session = match GuestSession::new(&image, 100_000) {
            Ok(session) => session,
            Err(e) => {
                self.push_log(&format!("xHoly: launch failed: {e}"));
                self.presence.update(PresenceState::MainMenu);
                return;
            }
        };
        let input = self.guest_input();
        if let Ok(Some(frame)) = session.run_frame(input) {
            self.state.guest_frame = Some(frame);
        }
        self.drain_guest_output(&mut session);
        self.push_log(&format!(
            "xHoly: guest frame 1 after {} steps",
            session.cpu.steps
        ));
        self.session = Some(session);
        self.state.running = true;
        self.presence.update(PresenceState::Emulating {
            game: game_name,
            console: ConsoleGeneration::XboxSeriesXS,
        });
    }

    /// Build guest input from the GUI's current held keys + mouse.
    fn guest_input(&self) -> GuestInput {
        GuestInput {
            buttons: self.input.guest_buttons,
            dx: self.input.mouse.0.clamp(-127.0, 127.0) as i32,
            dy: self.input.mouse.1.clamp(-127.0, 127.0) as i32,
        }
    }

    /// Forward guest console output into the on-screen console pane.
    fn drain_guest_output(&mut self, session: &mut GuestSession) {
        let bytes = std::mem::take(&mut session.cpu.output);
        for line in String::from_utf8_lossy(&bytes).lines() {
            self.push_log(line);
        }
        let lines = std::mem::take(&mut session.kernel.log);
        for line in lines {
            self.push_log(&format!("kernel: {line}"));
        }
    }

    /// Advance the running guest by one frame (Phase 22).
    ///
    /// Called once per GUI frame from [`Self::render_frame`]. Guest state
    /// persists in `self.session`; when the guest writes no framebuffer
    /// the previously captured frame is kept.
    pub fn tick_guest(&mut self) {
        if !self.state.running || self.state.paused || self.session.is_none() {
            return;
        }
        let input = self.guest_input();
        let result = self
            .session
            .as_mut()
            .map(|session| session.run_frame(input));
        match result {
            Some(Ok(Some(frame))) => self.state.guest_frame = Some(frame),
            Some(Ok(None)) => {} // hold last frame
            Some(Err(e)) => {
                self.push_log(&format!("xHoly: guest error: {e}"));
                self.state.running = false;
            }
            None => self.state.running = false,
        }
        // Take the boxed session out briefly so `self` can be borrowed
        // mutably by the log drain, then put it back at the same heap
        // address (its kernel self-pointers remain valid).
        if let Some(mut session) = self.session.take() {
            self.drain_guest_output(&mut session);
            self.session = Some(session);
        }
        if self.state.running {
            let game = self
                .state
                .selected()
                .map(|g| g.title.clone())
                .unwrap_or_else(|| "a game".into());
            // Deduped by the presence controller — safe to call per frame.
            self.presence.update(PresenceState::Emulating {
                game,
                console: ConsoleGeneration::XboxSeriesXS,
            });
        } else {
            self.presence.update(PresenceState::MainMenu);
        }
    }

    /// Append a console line, keeping the last 200 entries.
    fn push_log(&mut self, line: &str) {
        self.state.console_log.push(line.to_string());
        if self.state.console_log.len() > 200 {
            let excess = self.state.console_log.len() - 200;
            self.state.console_log.drain(..excess);
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
            self.config.vsync,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("xHoly: renderer init failed: {e}");
                self.should_exit = true;
                return;
            }
        };
        // Wrap in the GL-fallback type when the detected API is OpenGL.
        let mut boxed: Box<dyn GuiFrameRenderer> = match self.gpu_api {
            GpuApi::OpenGL => Box::new(crate::gui::gl_backend::GlBackend::new(wgpu_renderer)),
            _ => Box::new(wgpu_renderer),
        };
        boxed.set_postfx(self.config.postfx);
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
            WindowEvent::ModifiersChanged(m) => {
                self.input.modifiers_changed(m.state());
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

        // The emulation overlay swallows clicks while it is open.
        if self.state.emu_menu_open {
            for (rect, label) in &layout.menu {
                if rect.contains(mx, my) {
                    match *label {
                        "RESUME" => self.state.emu_menu_open = false,
                        "MAIN MENU" => {
                            self.state.emu_menu_open = false;
                            self.state.view = GuiView::Home;
                            if self.state.running && !self.state.paused {
                                self.state.toggle_pause();
                            }
                        }
                        _ => {}
                    }
                    self.input.mouse_down = false;
                    return;
                }
            }
            self.input.mouse_down = false;
            return;
        }

        // Top navigation (Home / Library / Emulation / Settings / Logs / About)
        for (rect, label) in &layout.nav {
            if rect.contains(mx, my) {
                self.state.view = match *label {
                    "HOME" => GuiView::Home,
                    "LIBRARY" => GuiView::Library,
                    "EMULATION" => GuiView::Emulation,
                    "SETTINGS" => GuiView::Settings,
                    "LOGS" => GuiView::Logs,
                    "ABOUT" => GuiView::About,
                    _ => self.state.view,
                };
                self.input.mouse_down = false;
                return;
            }
        }

        // Toolbar buttons
        for (rect, label) in &layout.buttons {
            if rect.contains(mx, my) {
                if !ui::button_enabled(label, &self.state) {
                    self.input.mouse_down = false;
                    return;
                }
                match *label {
                    "LAUNCH" => self.launch(),
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
        // Recently played entries (home view)
        for (rect, idx) in &layout.recent {
            if rect.contains(mx, my) {
                self.state.selected_game = *idx;
                self.input.mouse_down = false;
                return;
            }
        }
        // Log filter chips (diagnostics view)
        for (rect, label) in &layout.filters {
            if rect.contains(mx, my) {
                if let Some(filter) = state::LogFilter::parse(label) {
                    self.state.log_filter = filter;
                }
                self.input.mouse_down = false;
                return;
            }
        }
        // Cover grid (Game Library)
        for (rect, idx) in &layout.covers {
            if rect.contains(mx, my) {
                self.state.selected_game = *idx;
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
/// `pref` selects the GPU API: `Auto` probes the platform order
/// (Windows: D3D12, Linux: Vulkan, macOS: Metal) with wgpu/software
/// fallbacks; explicit values force that backend.
pub fn run_windowed(config: GuiConfig, pref: GpuApiPreference) -> Result<(), String> {
    let detection = gpu_api_detector::detect_with_preference(pref);
    gpu_api_detector::save_detection(&detection);
    eprintln!(
        "xHoly: GPU API = {} ({}) [native: {}]",
        detection.describe(),
        detection.backend,
        NativeGpuBackend::for_host().label()
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
    app.state.software_mode = detection.api == GpuApi::Software;
    app.state.software_message = if detection.api == GpuApi::Software {
        "Running on software renderer — performance may be lower".into()
    } else {
        String::new()
    };
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
        "xHoly: GPU API = {} ({}) [native: {}]",
        detection.describe(),
        detection.backend,
        NativeGpuBackend::for_host().label()
    );
    let (w, h) = config.window_size;
    let mut app = WindowApp::new_headless(config, detection.api);
    app.state.resolution = (w, h);
    app.state.gpu_label = detection.describe();
    app.state.software_mode = detection.api == GpuApi::Software;
    app.state.software_message = if detection.api == GpuApi::Software {
        "Running on software renderer — performance may be lower".into()
    } else {
        String::new()
    };

    // Software path: no GPU at all.
    if detection.api == GpuApi::Software {
        let mut sw = crate::gui::software_renderer::SoftwareRenderer::new(w, h);
        let mut last_fb = Vec::new();
        for _ in 0..frames.max(1) {
            app.pacer.record_frame_start();
            app.tick_guest();
            let (fb, _layout) = ui::composite(&app.state, w, h);
            sw.render_frame(&fb, w, h)?;
            last_fb = fb;
            app.tick_fps();
            app.pacer.sleep_until_next_frame();
        }
        return Ok(last_fb);
    }

    let mut offscreen = renderer::WgpuRenderer::new_offscreen_with_backends(
        w,
        h,
        gpu_api_detector::backends_for(detection.api),
    )?;
    offscreen.set_postfx(app.config.postfx);
    app.renderer = Some(Box::new(offscreen));

    let mut last_fb = Vec::new();
    for _ in 0..frames.max(1) {
        app.pacer.record_frame_start();
        app.tick_guest();
        let (fb, _layout) = ui::composite(&app.state, w, h);
        last_fb = fb.clone();
        if let Some(r) = app.renderer.as_mut() {
            r.render_frame(&fb, w, h)?;
        }
        app.tick_fps();
        app.pacer.sleep_until_next_frame();
    }
    Ok(last_fb)
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
        // Resolve payload paths against the host source (same rule as
        // the TUI): directories map to root/<payload>, ISO files map to
        // the ISO itself (run_path dispatches run_iso for those).
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
                // Automatic cover discovery in the dump's own directory.
                let game_dir = if item.path.is_file() {
                    item.path.parent().unwrap_or(&item.path)
                } else {
                    &item.path
                };
                let stem = item
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| item.title.clone());
                if let Some(cover) = crate::gui::thumbnails::discover(game_dir, &stem) {
                    item.cover_art = Some(cover.clone());
                    crate::gui::thumbnails::loader().queue(cover);
                }
                items.push(item);
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
        // Sidebar pixel should use the new blue-black panel surface.
        let i = (100 * 640 + 10) * 4;
        assert_eq!(&fb[i..i + 3], &crate::theme::PANEL[..3]);
    }

    #[test]
    fn launch_runs_guest_and_captures_frame() {
        // Phase 18: the windowed Launch path runs the real
        // loader→interpreter→kernel pipeline and captures both the
        // console output and the guest framebuffer frame.
        let config = GuiConfig {
            window_size: (320, 200),
            fps_limit: None,
            ..Default::default()
        };
        let mut app = WindowApp::new_headless(config, GpuApi::Software);
        // Pin the selection: the scanner also finds games/minigame,
        // which sorts before games/minimal.
        app.state.games.clear();
        app.state.selected_game = 0;
        app.state.games.push(GameListItem {
            title: "xHoly Minimal Guest".into(),
            path: "games/minimal".into(),
            cover_art: None,
            region: "OTHER".into(),
            discs: 1,
            release_date: None,
            is_multi_disc: false,
            last_played: None,
        });
        app.launch();
        let log = app.state.console_log.join("\n");
        assert!(log.contains("Hello, xHoly!"), "log was: {log}");
        let frame = app.state.guest_frame.as_ref().expect("guest frame");
        assert_eq!(frame.width, 160);
        assert_eq!(frame.height, 120);
        // Top-left pixel = "x" color, bottom-right = "y" color.
        assert_eq!(&frame.pixels[0..4], &[200, 40, 40, 255]);
        let last = frame.pixels.len() - 4;
        assert_eq!(&frame.pixels[last..], &[30, 226, 226, 255]);
    }

    #[test]
    fn minigame_runs_across_gui_frames_with_input() {
        // Phase 22: Launch creates a persistent GuestSession; each GUI
        // frame advances the guest instead of restarting it.
        let config = GuiConfig {
            window_size: (320, 200),
            fps_limit: None,
            ..Default::default()
        };
        let mut app = WindowApp::new_headless(config, GpuApi::Software);
        app.state.games.clear();
        app.state.selected_game = 0;
        app.state.games.push(GameListItem {
            title: "xHoly Minigame".into(),
            path: "games/minigame".into(),
            cover_art: None,
            region: "OTHER".into(),
            discs: 1,
            release_date: None,
            is_multi_disc: false,
            last_played: None,
        });
        app.launch();
        assert!(app.state.running);
        let first_seq = app.state.guest_frame.as_ref().expect("frame").seq;
        let first_pc = app.session.as_ref().expect("session").cpu.pc;

        // Hold the guest's LEFT button: the sprite must switch to cyan.
        app.input.guest_buttons = crate::core::execution::GuestInput::LEFT;
        app.tick_guest();
        app.tick_guest();

        let frame = app.state.guest_frame.as_ref().expect("held frame");
        assert_eq!(frame.seq, first_seq + 2);
        assert_eq!(app.session.as_ref().unwrap().cpu.pc, first_pc);
        assert!(frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p == &[30, 226, 226, 255]));
        assert_eq!(app.session.as_ref().unwrap().frames, 3);
    }

    #[test]
    fn guest_frame_holds_when_guest_writes_nothing() {
        // Hold-last-frame semantics: launch a guest that exits without
        // touching the FB aperture — the previous frame must remain.
        let dir = std::env::temp_dir().join("xholy_hold_frame");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("nofb.bin");
        // MOVZ x0,#7 ; SVC ; BRK — exits immediately, no FB writes.
        let mut bytes = Vec::new();
        for w in [0xD2800000u32 | (7 << 5), 0xD4000001, 0xD4200000] {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        std::fs::write(&bin, &bytes).unwrap();

        let config = GuiConfig {
            window_size: (320, 200),
            fps_limit: None,
            ..Default::default()
        };
        let mut app = WindowApp::new_headless(config, GpuApi::Software);
        app.state.games.push(GameListItem {
            title: "frame".into(),
            path: "games/minimal".into(),
            cover_art: None,
            region: "OTHER".into(),
            discs: 1,
            release_date: None,
            is_multi_disc: false,
            last_played: None,
        });
        app.launch();
        assert!(app.state.guest_frame.is_some());

        // Second launch: a guest with no framebuffer output.
        app.state.games[0].path = bin.clone();
        app.launch();
        assert!(
            app.state.guest_frame.is_some(),
            "last guest frame must be held when the FB is empty"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
