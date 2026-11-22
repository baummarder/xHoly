use clap::Parser;
use std::path::PathBuf;
use xholy::config::Config;
use xholy::gui::gpu_api_detector::GpuApiPreference;
use xholy::gui::GuiConfig;
use xholy::optical::{detect_drives, game_scanner, VirtualDrive};
use xholy::Emulator;

#[derive(Parser, Debug)]
#[command(
    name = "xholy",
    about = "xHoly - Xbox Series S|X / Xbox One emulator (research)"
)]
struct Args {
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

    /// Run without any UI (explicit headless mode; this is the default
    /// when a game image is supplied without --gui).
    #[arg(long)]
    headless: bool,

    /// Maximum number of frames to run (default: unlimited)
    #[arg(long)]
    max_frames: Option<u64>,

    /// Config file path (default: ~/.config/xholy/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() {
    env_logger::init();
    let args = Args::parse();

    // Resolve GPU API preference: explicit --gpu-api wins, then the
    // --gpu-*/--gui-* force flags, else Auto.
    let gpu_pref = args
        .gpu_api
        .as_deref()
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

    let gui_mode = args.gui || args.gui_wgpu;
    let tui_mode = args.gui_tui || args.gui_headless_ratatui;
    let gui_headless = args.gui_headless || args.gui_headless_wgpu;

    if gui_mode || tui_mode || gui_headless {
        let mut gui_config = GuiConfig::default();
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
    let frames = emu.run(args.max_frames);
    log::info!("xHoly: rendered {} frames", frames);
}
