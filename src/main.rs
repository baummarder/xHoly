use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use xholy::config::Config;
use xholy::gui::{GpuApiPreference, GuiConfig, GuiTheme};
use xholy::optical::{detect_drives, game_scanner, VirtualDrive};
use xholy::Emulator;

#[derive(Parser, Debug)]
#[command(
    name = "xholy",
    about = "xHoly - Xbox Series X|S HLE emulator (research)"
)]
struct Args {
    /// Print the roadmap progress report (derived from progress.json).
    #[arg(long)]
    progress: bool,

    /// Read-only import analysis of a PE32+ executable. The file is never executed.
    #[arg(long, value_name = "PATH")]
    analyze: Option<std::path::PathBuf>,

    /// Execute a PE32+ file in the bounded x86-64 HLE pipeline and export its trace.
    #[arg(long, value_name = "PATH")]
    trace: Option<std::path::PathBuf>,

    /// Optional output path for --trace (prints to stdout if omitted).
    #[arg(long, value_name = "PATH", requires = "trace")]
    trace_out: Option<std::path::PathBuf>,

    /// Load a previously exported `.xholy-trace.json` into the GUI trace viewer.
    /// Works with --gui, --gui-headless and --gui-tui; never executes the file.
    #[arg(long, value_name = "PATH")]
    gui_trace: Option<PathBuf>,

    /// Run the self-contained synthetic M33 guest boot (no file access).
    /// Prints an ordered boot trace with load, memory, stack, entry point,
    /// CPU steps, HLE invocation/return, guest exit and final result.
    #[arg(long)]
    guest_test: bool,

    /// With --analyze, --trace or --guest-test: print JSON instead of the text summary.
    #[arg(long)]
    json: bool,
    /// Path to a game image or disc (e.g. .iso, .xvc, directory)
    image: Option<PathBuf>,

    /// Mount an optical source: a physical drive letter (D:\), a directory,
    /// or an ISO image file. Can be repeated for multi-disc setups.
    #[arg(long = "mount", value_name = "PATH")]
    mounts: Vec<PathBuf>,

    /// List detected games on mounted media and exit.
    #[arg(long)]
    list: bool,

    /// Detect and print physical optical drives, then exit.
    #[arg(long)]
    drives: bool,

    /// Launch the windowed GUI (winit + wgpu).
    #[arg(long)]
    gui: bool,

    /// Launch the terminal UI backend (ratatui).
    #[arg(long)]
    gui_tui: bool,

    /// Run the windowed GUI pipeline headless (no window; renders
    /// --max-frames frames offscreen, default 300).
    #[arg(long)]
    gui_headless: bool,

    /// Alias for --gui (windowed wgpu GUI).
    #[arg(long, hide = true)]
    gui_wgpu: bool,

    /// Alias for --gui-headless.
    #[arg(long, hide = true)]
    gui_headless_wgpu: bool,

    /// Alias for --gui-tui.
    #[arg(long, hide = true)]
    gui_headless_ratatui: bool,

    /// Force a GPU API for the GUI: auto | vulkan | opengl | software.
    /// Auto probes Vulkan → OpenGL → Software.
    #[arg(long, value_name = "API")]
    gpu_api: Option<String>,

    /// Alias for --gpu-api (Phase 9 naming).
    #[arg(long, value_name = "API", hide = true)]
    renderer: Option<String>,

    /// Force Vulkan backend (alias for --gpu-api vulkan).
    #[arg(long, hide = true)]
    gpu_vulkan: bool,

    /// Force OpenGL backend (alias for --gpu-api opengl).
    #[arg(long, hide = true)]
    gpu_opengl: bool,

    /// Force software renderer (alias for --gpu-api software).
    #[arg(long, hide = true)]
    gpu_software: bool,

    /// Shortcut: windowed GUI forced to Vulkan.
    #[arg(long, hide = true)]
    gui_vulkan: bool,

    /// Shortcut: windowed GUI forced to OpenGL.
    #[arg(long, hide = true)]
    gui_opengl: bool,

    /// Shortcut: windowed GUI forced to software renderer.
    #[arg(long, hide = true)]
    gui_software: bool,

    /// FPS limit for the GUI (0 = unlimited).
    #[arg(long, value_name = "N")]
    fps_limit: Option<u32>,

    /// GUI window resolution WIDTHxHEIGHT (e.g. --resolution 1280x720).
    #[arg(long, value_name = "WxH")]
    resolution: Option<String>,

    /// Start the GUI in fullscreen.
    #[arg(long)]
    fullscreen: bool,

    /// GUI theme: dark | gold | light (also trans/pride/pink).
    #[arg(long, value_name = "THEME")]
    theme: Option<String>,

    /// Frame pacing mode: auto | fixed | uncapped.
    #[arg(long, value_name = "MODE")]
    frame_pacing: Option<String>,

    /// Disable VSync (manual frame pacing takes over).
    #[arg(long)]
    no_vsync: bool,

    /// Enable bloom post-processing (default: on).
    #[arg(long)]
    bloom: bool,

    /// Disable bloom post-processing.
    #[arg(long)]
    no_bloom: bool,

    /// Tone mapping: linear | reinhard | filmic | aces.
    #[arg(long, value_name = "KIND")]
    tone_mapping: Option<String>,

    /// Anti-aliasing: off | fxaa | msaa4.
    #[arg(long, value_name = "METHOD")]
    aa: Option<String>,

    /// Run without any UI (explicit headless mode; this is the default
    /// when a game image is supplied without --gui).
    #[arg(long)]
    headless: bool,

    /// Game image or game directory to execute on the ARM64 interpreter
    /// + kernel stub (Phase 8). Runs headless regardless of --gui.
    #[arg(long, value_name = "PATH")]
    game: Option<PathBuf>,

    /// Maximum guest instructions for --game (default: 1_000_000).
    #[arg(long, value_name = "N")]
    max_steps: Option<u64>,

    /// Enable the x86-64 → WASM JIT branch for x86-64 guest code
    /// (Phase 14). Runs parallel to the ARM64 interpreter; falls back
    /// to the scalar x86 interpreter on JIT failure.
    #[arg(long)]
    enable_jit: bool,

    /// Disable the JIT interpreter fallback (JIT errors become fatal).
    #[arg(long)]
    jit_no_fallback: bool,

    /// Run the native C++ Win32 window manager demo (--native-wm).
    /// Windows only.
    #[arg(long)]
    native_wm: bool,

    /// Maximum number of frames to run (default: unlimited)
    #[arg(long)]
    max_frames: Option<u64>,

    /// Show the easter-egg start screen once this session (default on).
    #[arg(long)]
    easter_egg: bool,

    /// Always show the easter egg, bypassing the once-per-session guard.
    #[arg(long)]
    easter_egg_force: bool,

    /// Disable the easter-egg start screen.
    #[arg(long)]
    no_easter_egg: bool,

    /// Config file path (default: ~/.config/xholy/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Disable Discord Rich Presence for this session.
    #[arg(long)]
    no_discord_rpc: bool,
}

/// Parse "WxH" (e.g. "1280x720", "1920X1080") into (w, h).
fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn main() {
    env_logger::init();
    let args = Args::parse();

    // Read-only commands run before any presence, easter-egg or GPU setup.
    if args.progress {
        match xholy::progress::report() {
            Ok(report) => print!("{}", report.render_text()),
            Err(e) => {
                eprintln!("xHoly: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(path) = args.analyze.as_deref() {
        let registry = xholy::core::hle::HleRegistry::with_known_entries();
        match xholy::core::loader::import_analyzer::analyze_file(path, &registry) {
            Ok(report) if args.json => match serde_json::to_string_pretty(&report) {
                Ok(text) => println!("{text}"),
                Err(e) => {
                    eprintln!("xHoly: cannot serialise report: {e}");
                    std::process::exit(1);
                }
            },
            Ok(report) => print!("{}", report.render_text()),
            Err(e) => {
                eprintln!("xHoly: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(path) = args.trace.as_deref() {
        let max_steps = args.max_steps.unwrap_or(10_000);
        let report = match xholy::core::pipeline::trace::run_pe_file_trace(path, max_steps) {
            Ok(report) => report,
            Err(e) => {
                eprintln!("xHoly: {e}");
                std::process::exit(1);
            }
        };
        let rendered = if args.json {
            match xholy::core::pipeline::trace::export_json(&report) {
                Ok(text) => format!("{text}\n"),
                Err(e) => {
                    eprintln!("xHoly: cannot serialise trace: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            xholy::core::pipeline::trace::export_text(&report)
        };
        if let Some(out_path) = args.trace_out.as_deref() {
            if let Err(e) = std::fs::write(out_path, rendered.as_bytes()) {
                eprintln!("xHoly: cannot write trace to {}: {e}", out_path.display());
                std::process::exit(1);
            }
        } else {
            print!("{rendered}");
        }
        return;
    }
    if args.guest_test {
        let max_steps = args.max_steps.unwrap_or(10_000);
        let trace = match xholy::core::pipeline::boot::run_synthetic_boot(max_steps) {
            Ok(trace) => trace,
            Err(e) => {
                eprintln!("xHoly: synthetic guest boot failed to load: {e}");
                std::process::exit(1);
            }
        };
        let rendered = if args.json {
            match trace.export_json() {
                Ok(text) => format!("{text}\n"),
                Err(e) => {
                    eprintln!("xHoly: cannot serialise boot trace: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            trace.render_text()
        };
        print!("{rendered}");
        // A failed/incomplete guest boot is a real failure of the test.
        if trace.status != xholy::core::pipeline::boot::BootStatus::Success {
            std::process::exit(1);
        }
        return;
    }
    // Discord Rich Presence: shared, non-blocking sink. The worker only
    // connects when a real Application ID is configured and the feature
    // was not disabled via CLI.
    use xholy::presence::{Presence, PresenceSink, PresenceState};
    let mut discord_settings = xholy::presence::load_settings(args.config.as_deref());
    if args.no_discord_rpc {
        discord_settings.enabled = false;
    }
    let presence: Arc<dyn PresenceSink> = Arc::new(Presence::new(discord_settings));
    presence.update(PresenceState::MainMenu);

    // Easter egg: once-per-session trans start screen before any real
    // initialization (GUI/game/headless). Debug builds get the minimal
    // text-only variant so `cargo run` stays quick.
    use xholy::easter_egg::{self, EasterEgg, EasterEggRequest};
    match easter_egg::cli::resolve(args.no_easter_egg, args.easter_egg, args.easter_egg_force) {
        EasterEggRequest::Off => {}
        EasterEggRequest::Once => {
            let cfg = easter_egg::cli::config_for(cfg!(debug_assertions));
            EasterEgg::new(cfg).show_if_needed();
        }
        EasterEggRequest::Force => {
            let cfg = easter_egg::cli::config_for(cfg!(debug_assertions));
            EasterEgg::new(cfg).show();
        }
    }

    // Resolve GPU API preference: explicit --gpu-api wins, then the
    // --gpu-*/--gui-* force flags, else Auto.
    let gpu_pref = args
        .gpu_api
        .as_deref()
        .or(args.renderer.as_deref())
        .and_then(GpuApiPreference::parse)
        .or(if args.gpu_vulkan || args.gui_vulkan {
            Some(GpuApiPreference::Vulkan)
        } else if args.gpu_opengl || args.gui_opengl {
            Some(GpuApiPreference::OpenGL)
        } else if args.gpu_software || args.gui_software {
            Some(GpuApiPreference::Software)
        } else {
            None
        })
        .unwrap_or_default();
    if args.gpu_api.is_some()
        && gpu_pref == GpuApiPreference::Auto
        && args.gpu_api.as_deref() != Some("auto")
    {
        eprintln!("xHoly: warning: unrecognized --gpu-api value, using auto");
    }

    // Phase 8: native WM demo (Windows only).
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    if args.native_wm {
        let frames = args.max_frames.unwrap_or(600);
        match xholy::gui::native_wm::run_native_demo(frames) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("xHoly: native WM failed: {e}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    if args.native_wm {
        eprintln!("xHoly: --native-wm is Windows-only");
        std::process::exit(1);
    }

    // Phase 8: game execution on the ARM64 interpreter + kernel stub.
    if let Some(path) = &args.game {
        let steps = args.max_steps.unwrap_or(1_000_000);
        let game_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "a game".into());
        presence.update(PresenceState::Loading {
            game: game_name.clone(),
        });
        presence.update(PresenceState::Emulating {
            game: game_name,
            console: xholy::presence::ConsoleGeneration::XboxSeriesXS,
        });
        match xholy::core::execution::run_game_path(path, steps) {
            Ok(report) => {
                println!("xHoly: guest exited after {} instructions", report.steps);
                if !report.output.is_empty() {
                    print!("{}", String::from_utf8_lossy(&report.output));
                }
                for line in &report.kernel_log {
                    eprintln!("xHoly[kernel]: {line}");
                }
                presence.update(PresenceState::MainMenu);
                return;
            }
            Err(e) => {
                presence.update(PresenceState::MainMenu);
                eprintln!("xHoly: game execution failed: {e}");
                std::process::exit(1);
            }
        }
    }

    let gui_mode = args.gui || args.gui_wgpu;
    let tui_mode = args.gui_tui || args.gui_headless_ratatui;
    let gui_headless = args.gui_headless || args.gui_headless_wgpu;

    if gui_mode || tui_mode || gui_headless {
        let mut gui_config = GuiConfig {
            preferred_api: gpu_pref,
            presence: Some(presence.clone()),
            ..Default::default()
        };
        if let Some(f) = args.fps_limit {
            gui_config.fps_limit = (f > 0).then_some(f);
        }
        if let Some(res) = &args.resolution {
            if let Some((w, h)) = parse_resolution(res) {
                gui_config.window_size = (w, h);
            } else {
                eprintln!("xHoly: warning: bad --resolution '{res}', expected WxH");
            }
        }
        if args.fullscreen {
            gui_config.fullscreen = true;
        }
        if let Some(t) = &args.theme {
            match GuiTheme::parse(t) {
                Some(theme) => gui_config.theme = theme,
                None => eprintln!("xHoly: warning: bad --theme '{t}', expected dark|gold|light"),
            }
        }
        if let Some(m) = &args.frame_pacing {
            match xholy::gui::FramePacingMode::parse(m) {
                Some(mode) => gui_config.frame_pacing_mode = mode,
                None => eprintln!(
                    "xHoly: warning: bad --frame-pacing '{m}', expected auto|fixed|uncapped"
                ),
            }
        }
        if args.no_vsync {
            gui_config.vsync = false;
        }
        // Phase 9 post-processing flags.
        if args.bloom {
            gui_config.postfx.bloom = true;
        }
        if args.no_bloom {
            gui_config.postfx.bloom = false;
        }
        if let Some(t) = &args.tone_mapping {
            match xholy::gui::ToneMapping::parse(t) {
                Some(m) => gui_config.postfx.tone_mapping = m,
                None => eprintln!(
                    "xHoly: warning: bad --tone-mapping '{t}', expected linear|reinhard|filmic|aces"
                ),
            }
        }
        if let Some(a) = &args.aa {
            match xholy::gui::AaMethod::parse(a) {
                Some(m) => gui_config.postfx.aa = m,
                None => eprintln!("xHoly: warning: bad --aa '{a}', expected off|fxaa|msaa4"),
            }
        }
        gui_config.trace_file = args.gui_trace.clone();
        if gui_headless {
            gui_config.headless = true;
            let frames = args.max_frames.unwrap_or(300) as u32;
            match xholy::gui::windowed::run_headless(gui_config, frames, gpu_pref) {
                Ok(_) => return,
                Err(e) => {
                    eprintln!("xHoly: headless GUI failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        if tui_mode {
            if let Err(e) = xholy::gui::run(gui_config) {
                eprintln!("xHoly: TUI failed: {e}");
                std::process::exit(1);
            }
            return;
        }
        // Windowed winit+wgpu GUI
        match xholy::gui::windowed::run_windowed(gui_config, gpu_pref) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("xHoly: windowed GUI failed: {e}");
                std::process::exit(1);
            }
        }
    }
    let _ = args.headless; // explicit flag; headless is the default path
    let _ = args.gui_trace; // consumed by the GUI block above; no-op on CLI-only paths

    if args.drives {
        let drives = detect_drives();
        if drives.is_empty() {
            println!("No optical drives detected.");
        }
        for d in drives {
            println!(
                "{}  {}{}",
                d.mount_point.display(),
                d.name,
                d.media_label
                    .map(|l| format!("  [{}]", l))
                    .unwrap_or_default()
            );
        }
        return;
    }

    // Mount any requested optical sources up front.
    let mut drive = VirtualDrive::new("D:\\");
    for mount in &args.mounts {
        match drive.mount_path(mount) {
            Ok(()) => log::info!("xHoly: mounted {:?}", mount),
            Err(e) => {
                eprintln!("xHoly: failed to mount {:?}: {}", mount, e);
                std::process::exit(2);
            }
        }
    }

    if args.list {
        match drive.media() {
            Some(media) => match game_scanner::scan(media.filesystem()) {
                Ok(games) => {
                    let meta = media.filesystem().metadata();
                    println!("Volume: {} ({:?})", meta.volume_label, meta.filesystem);
                    if games.is_empty() {
                        println!("No games detected on this disc.");
                    }
                    for (i, g) in games.iter().enumerate() {
                        let m = &g.metadata;
                        println!(
                            "{}: {} [{}] disc {} — {} bytes, {} files ({})",
                            i + 1,
                            m.title_name.as_deref().unwrap_or("<untitled>"),
                            m.region.as_str(),
                            if m.disc_number == 0 {
                                "?".into()
                            } else {
                                m.disc_number.to_string()
                            },
                            g.size_bytes,
                            g.file_count,
                            g.payload_path
                        );
                    }
                }
                Err(e) => {
                    eprintln!("xHoly: scan failed: {}", e);
                    std::process::exit(1);
                }
            },
            None => {
                eprintln!("xHoly: no media mounted (use --mount PATH)");
                std::process::exit(1);
            }
        }
        return;
    }

    let Some(image) = args.image else {
        eprintln!("xHoly: no game image given. Usage: xholy <image> [--mount PATH] [--list]");
        std::process::exit(2);
    };

    let config = match &args.config {
        Some(p) => Config::load(p),
        None => Config::load_default(),
    };
    config.save_default().ok();

    let mut emu = Emulator::new(&image, config);
    if args.enable_jit {
        emu.enable_jit(xholy::jit::JitConfig {
            enabled: true,
            fallback_to_interpreter: !args.jit_no_fallback,
            max_instructions: args.max_steps.unwrap_or(10_000_000),
            ..Default::default()
        });
    }
    let frames = emu.run(args.max_frames);
    log::info!("xHoly: rendered {} frames", frames);
}
