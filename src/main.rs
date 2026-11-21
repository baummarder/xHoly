use clap::Parser;
use xholy::config::Config;
use xholy::Emulator;

#[derive(Parser, Debug)]
#[command(
    name = "xholy",
    about = "xHoly - Xbox Series S|X / Xbox One emulator (research)"
)]
struct Args {
    /// Path to a game image (e.g. .xvc, .iso, directory with game files)
    image: std::path::PathBuf,

    /// Maximum number of frames to run (default: unlimited)
    #[arg(long)]
    max_frames: Option<u64>,

    /// Config file path (default: ~/.config/xholy/config.toml)
    #[arg(long)]
    config: Option<std::path::PathBuf>,
}

fn main() {
    env_logger::init();
    let args = Args::parse();

    let config = match &args.config {
        Some(p) => Config::load(p),
        None => Config::load_default(),
    };
    config.save_default().ok();

    let mut emu = Emulator::new(&args.image, config);
    let frames = emu.run(args.max_frames);
    log::info!("xHoly: rendered {} frames", frames);
}
