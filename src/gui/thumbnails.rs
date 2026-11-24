//! Automatic game cover / thumbnail discovery (Phase 26).
//!
//! Only the game's own directory and the sub-folders `covers`,
//! `thumbnails`, `images`, `media` are inspected — never a whole drive.
//! Files are validated by actually decoding them. Loading happens on a
//! worker thread; the UI thread only polls a cache.

use crate::gui::textures::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// Image formats accepted by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverFormat {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// WebP.
    WebP,
    /// BMP.
    Bmp,
}

impl CoverFormat {
    /// All supported extensions, in priority order.
    pub const EXTENSIONS: [&'static str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

    /// Detect a format from file magic bytes.
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Self::WebP)
        } else if bytes.starts_with(b"BM") {
            Some(Self::Bmp)
        } else {
            None
        }
    }
}

/// Preferred cover names in a game directory.
pub const COVER_NAMES: [&str; 4] = ["cover", "thumbnail", "boxart", "folder"];
/// Relevant sub-directories (no recursive drive scan).
pub const COVER_DIRS: [&str; 4] = ["covers", "thumbnails", "images", "media"];

/// Find a cover candidate without decoding it yet.
///
/// Priority: same stem as the dump → common cover names → relevant
/// sub-folders. Only the supplied directory and those sub-folders are read.
pub fn discover(game_dir: &Path, dump_stem: &str) -> Option<PathBuf> {
    for ext in CoverFormat::EXTENSIONS {
        let candidate = game_dir.join(format!("{dump_stem}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for name in COVER_NAMES {
        for ext in CoverFormat::EXTENSIONS {
            let candidate = game_dir.join(format!("{name}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for dir in COVER_DIRS {
        let sub = game_dir.join(dir);
        if !sub.is_dir() {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&sub)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        CoverFormat::EXTENSIONS
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(e))
                    })
            })
            .collect();
        files.sort();
        if let Some(first) = files.first() {
            return Some(first.clone());
        }
    }
    None
}

/// Validate and decode a cover file into RGBA8 pixels.
pub fn decode_cover(path: &Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let format = CoverFormat::detect(&bytes)
        .ok_or_else(|| format!("{} is not a supported image", path.display()))?;
    let _ = format; // magic already validated; image crate does full decode
    let decoded = image::load_from_memory(&bytes)
        .map_err(|e| format!("decode {}: {e}", path.display()))?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("{} has zero dimensions", path.display()));
    }
    Ok(RgbaImage {
        width,
        height,
        pixels: decoded.into_raw(),
    })
}

/// Uniform placeholder used when no valid cover exists.
pub fn placeholder(width: u32, height: u32) -> RgbaImage {
    use crate::theme::{self, Rgba};
    let w = width.max(1);
    let h = height.max(1);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize * 4;
            // Graphite field with a subtle trans-blue → pink diagonal.
            let t = (x as f32 / w as f32 + y as f32 / h as f32) * 0.5;
            let color: Rgba = theme::trans_gradient(t);
            let mix: Rgba = theme::mix(theme::PANEL, color, 0.28);
            pixels[i..i + 4].copy_from_slice(&mix);
        }
    }
    RgbaImage {
        width: w,
        height: h,
        pixels,
    }
}

/// Scale a decoded cover to a target size, preserving aspect ratio by
/// centre-cropping (cover fit — never stretches the image).
pub fn cover_fit(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let (w, h) = (width.max(1), height.max(1));
    let src_ratio = image.width as f32 / image.height.max(1) as f32;
    let dst_ratio = w as f32 / h as f32;
    let (crop_w, crop_h) = if src_ratio > dst_ratio {
        (image.height as f32 * dst_ratio, image.height as f32)
    } else {
        (image.width as f32, image.width as f32 / dst_ratio)
    };
    let ox = (image.width as f32 - crop_w) / 2.0;
    let oy = (image.height as f32 - crop_h) / 2.0;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = (ox + x as f32 * crop_w / w as f32) as usize;
            let sy = (oy + y as f32 * crop_h / h as f32) as usize;
            let si = (sy * image.width as usize + sx) * 4;
            let di = (y * w + x) as usize * 4;
            if si + 4 <= image.pixels.len() {
                out[di..di + 4].copy_from_slice(&image.pixels[si..si + 4]);
            }
        }
    }
    RgbaImage {
        width: w,
        height: h,
        pixels: out,
    }
}

/// Cache of decoded covers, keyed by absolute path.
#[derive(Debug, Default, Clone)]
pub struct CoverCache {
    entries: Arc<Mutex<HashMap<PathBuf, Option<RgbaImage>>>>,
}

impl CoverCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached cover, if it was already loaded (including failed loads).
    pub fn get(&self, path: &Path) -> Option<Option<RgbaImage>> {
        self.entries.lock().unwrap().get(path).cloned()
    }

    /// Number of cached entries (hits and misses).
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decode and cache a cover (used by the worker thread).
    pub fn load(&self, path: PathBuf) {
        let result = decode_cover(&path).ok();
        self.entries.lock().unwrap().insert(path, result);
    }

    /// Remember a failed discovery so the UI can show the placeholder.
    pub fn remember_missing(&self, path: PathBuf) {
        self.entries.lock().unwrap().insert(path, None);
    }
}

/// Non-blocking thumbnail loader.
pub struct ThumbnailLoader {
    tx: SyncSender<PathBuf>,
    cache: CoverCache,
}

impl ThumbnailLoader {
    /// Spawn the loader thread.
    pub fn new() -> Self {
        let (tx, rx): (SyncSender<PathBuf>, Receiver<PathBuf>) = mpsc::sync_channel(32);
        let cache = CoverCache::new();
        let worker_cache = cache.clone();
        std::thread::spawn(move || {
            while let Ok(path) = rx.recv() {
                worker_cache.load(path);
            }
        });
        Self { tx, cache }
    }

    /// Shared cache for UI polling.
    pub fn cache(&self) -> &CoverCache {
        &self.cache
    }

    /// Queue a cover for decoding; never blocks on file IO.
    pub fn queue(&self, path: PathBuf) {
        match self.tx.try_send(path) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

/// Process-wide loader shared by all UI views.
static LOADER: std::sync::OnceLock<ThumbnailLoader> = std::sync::OnceLock::new();

/// Shared thumbnail loader used by the UI.
pub fn loader() -> &'static ThumbnailLoader {
    LOADER.get_or_init(ThumbnailLoader::new)
}

impl Default for ThumbnailLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xholy_thumbs_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path) {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            32,
            image::Rgba([10, 20, 30, 255]),
        ));
        img.save(path).unwrap();
    }

    #[test]
    fn magic_detection_covers_all_supported_formats() {
        assert_eq!(
            CoverFormat::detect(b"\x89PNG\r\n\x1a\n"),
            Some(CoverFormat::Png)
        );
        assert_eq!(
            CoverFormat::detect(b"\xff\xd8\xffx"),
            Some(CoverFormat::Jpeg)
        );
        assert_eq!(
            CoverFormat::detect(b"RIFFxxxxWEBP"),
            Some(CoverFormat::WebP)
        );
        assert_eq!(CoverFormat::detect(b"BMxxxx"), Some(CoverFormat::Bmp));
        assert_eq!(CoverFormat::detect(b"nope"), None);
    }

    #[test]
    fn discovery_priority_same_stem_then_cover_name_then_subfolder() {
        let dir = temp_dir("priority");
        write_png(&dir.join("cover.png"));
        assert_eq!(discover(&dir, "game"), Some(dir.join("cover.png")));
        write_png(&dir.join("game.webp"));
        assert_eq!(discover(&dir, "game"), Some(dir.join("game.webp")));
        let sub = dir.join("covers");
        std::fs::create_dir_all(&sub).unwrap();
        write_png(&sub.join("art.png"));
        let dir2 = temp_dir("priority2");
        let sub2 = dir2.join("thumbnails");
        std::fs::create_dir_all(&sub2).unwrap();
        write_png(&sub2.join("art.png"));
        assert_eq!(discover(&dir2, "game"), Some(sub2.join("art.png")));
    }

    #[test]
    fn invalid_and_missing_images_are_handled() {
        let dir = temp_dir("invalid");
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        assert!(decode_cover(&bad).is_err());
        assert!(decode_cover(&dir.join("missing.png")).is_err());
        assert_eq!(discover(&dir, "nothing"), None);
    }

    #[test]
    fn decoded_cover_keeps_pixels_and_placeholder_is_uniform_size() {
        let dir = temp_dir("decode");
        let file = dir.join("cover.png");
        write_png(&file);
        let img = decode_cover(&file).unwrap();
        assert_eq!((img.width, img.height), (64, 32));
        let fitted = cover_fit(&img, 128, 128);
        assert_eq!((fitted.width, fitted.height), (128, 128));
        let ph = placeholder(100, 50);
        assert_eq!((ph.width, ph.height), (100, 50));
        assert_eq!(ph.pixels.len(), 100 * 50 * 4);
    }

    #[test]
    fn cache_records_hits_and_misses() {
        let dir = temp_dir("cache");
        let file = dir.join("cover.png");
        write_png(&file);
        let cache = CoverCache::new();
        cache.load(file.clone());
        assert!(cache.get(&file).unwrap().is_some());
        cache.remember_missing(dir.join("none.png"));
        assert!(cache.get(&dir.join("none.png")).unwrap().is_none());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn loader_is_non_blocking_and_eventually_fills_cache() {
        let dir = temp_dir("loader");
        let file = dir.join("cover.png");
        write_png(&file);
        let loader = ThumbnailLoader::new();
        loader.queue(file.clone());
        for _ in 0..50 {
            if loader.cache().get(&file).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(loader.cache().get(&file).unwrap().is_some());
    }
}
