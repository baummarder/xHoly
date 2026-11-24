//! Writes the synthetic PE32+ test image to the given path (default:
//! `synthetic.exe`). The image is generated from source; no third-party
//! binaries are used. Useful for `xholy --analyze` examples.

use xholy::core::loader::pe64::synthetic_pe;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "synthetic.exe".into());
    if let Err(e) = std::fs::write(&path, synthetic_pe()) {
        eprintln!("cannot write {path}: {e}");
        std::process::exit(1);
    }
    println!("wrote {path}");
}
