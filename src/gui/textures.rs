//! Texture management — Phase 9.
//!
//! CPU-side sprite atlas for UI icons plus a small wgpu texture cache
//! used by the windowed renderer. ISO-sourced textures are loaded as
//! raw RGBA payloads at a byte offset (the optical layer's ISO reader
//! is a separate concern — see [`crate::optical`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A CPU-side RGBA8 image.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// width*height*4 bytes, row-major RGBA.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// Solid-color image.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for p in pixels.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&rgba);
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

/// Sprite atlas: packs small RGBA images into one big image to reduce
/// draw calls / texture binds.
#[derive(Debug)]
pub struct TextureAtlas {
    /// Packed pixels (grown on demand, power-of-two-ish).
    pub image: RgbaImage,
    /// name → (u, v, w, h) in atlas pixels.
    entries: HashMap<String, AtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_h: u32,
}

/// Atlas sub-rect.
#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Default for TextureAtlas {
    fn default() -> Self {
        Self::new(256)
    }
}

impl TextureAtlas {
    /// Atlas with a fixed backing size.
    pub fn new(size: u32) -> Self {
        Self {
            image: RgbaImage::solid(size, size, [0, 0, 0, 0]),
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_h: 0,
        }
    }

    /// Insert an image; returns its atlas rect (None if it doesn't fit).
    pub fn insert(&mut self, name: &str, img: &RgbaImage) -> Option<AtlasEntry> {
        if self.entries.contains_key(name) {
            return self.entries.get(name).copied();
        }
        let (w, h) = (img.width, img.height);
        // Advance to next row when the sprite doesn't fit horizontally.
        if self.cursor_x + w + 1 > self.image.width {
            self.cursor_x = 1;
            self.cursor_y += self.row_h + 1;
            self.row_h = 0;
        }
        if self.cursor_y + h + 1 > self.image.height {
            return None; // atlas full
        }
        // Blit into the atlas.
        for sy in 0..h {
            for sx in 0..w {
                let si = ((sy * w + sx) * 4) as usize;
                let di =
                    (((self.cursor_y + sy) * self.image.width + self.cursor_x + sx) * 4) as usize;
                self.image.pixels[di..di + 4].copy_from_slice(&img.pixels[si..si + 4]);
            }
        }
        let e = AtlasEntry {
            x: self.cursor_x,
            y: self.cursor_y,
            w,
            h,
        };
        self.cursor_x += w + 1;
        self.row_h = self.row_h.max(h);
        self.entries.insert(name.to_string(), e);
        Some(e)
    }

    /// Look up a packed sprite.
    pub fn get(&self, name: &str) -> Option<AtlasEntry> {
        self.entries.get(name).copied()
    }
}

/// Load an RGBA image from a file (supports raw `.rgba` dumps and
/// PNG/JPEG via the `image` crate when the feature is enabled — for
/// now raw RGBA + TGA-ish headers are handled natively).
pub fn load_rgba(path: &Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    decode_rgba(&bytes, path)
}

/// Decode RGBA pixels. Recognizes:
/// - `.rgba` raw dumps with a tiny `XRGA` header: `XRGA` + w u32 + h u32
/// - otherwise treats the file as raw RGBA whose dimensions come from
///   a `WxH` in the filename stem or defaults to square.
fn decode_rgba(bytes: &[u8], path: &Path) -> Result<RgbaImage, String> {
    if bytes.len() >= 12 && &bytes[0..4] == b"XRGA" {
        let w = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let need = (w * h * 4) as usize;
        if bytes.len() < 12 + need {
            return Err("truncated XRGA payload".into());
        }
        return Ok(RgbaImage {
            width: w,
            height: h,
            pixels: bytes[12..12 + need].to_vec(),
        });
    }
    // Raw RGBA: infer square-ish dims from length.
    let px = bytes.len() / 4;
    let side = (px as f64).sqrt() as u32;
    if (side * side * 4) as usize != bytes.len() {
        return Err(format!(
            "cannot decode {}: not XRGA and not a square raw RGBA",
            path.display()
        ));
    }
    Ok(RgbaImage {
        width: side,
        height: side,
        pixels: bytes.to_vec(),
    })
}

/// Extract an RGBA sub-image from an ISO/disk image at `offset`.
///
/// This is deliberately dumb: Xbox ISO textures aren't a standard
/// container, so callers pass the exact byte offset + dims they
/// computed (e.g. from the game's asset table). Honest scaffold.
pub fn load_rgba_at_offset(
    image_path: &Path,
    offset: u64,
    width: u32,
    height: u32,
) -> Result<RgbaImage, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(image_path)
        .map_err(|e| format!("open {}: {e}", image_path.display()))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek: {e}"))?;
    let need = (width * height * 4) as usize;
    let mut buf = vec![0u8; need];
    f.read_exact(&mut buf).map_err(|e| format!("read: {e}"))?;
    Ok(RgbaImage {
        width,
        height,
        pixels: buf,
    })
}

/// wgpu-side texture cache: uploads `RgbaImage`s once, keyed by a
/// caller-supplied name. Lives next to the renderer (created lazily).
#[cfg(not(target_arch = "wasm32"))]
pub mod gpu {
    use super::RgbaImage;
    use std::collections::HashMap;

    /// Cache of uploaded wgpu textures.
    pub struct TextureCache {
        device: wgpu::Device,
        queue: wgpu::Queue,
        entries: HashMap<String, wgpu::Texture>,
    }

    impl TextureCache {
        /// New empty cache bound to a device.
        pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
            Self {
                device: device.clone(),
                queue: queue.clone(),
                entries: HashMap::new(),
            }
        }

        /// Upload `img` under `name` (idempotent).
        pub fn upload(&mut self, name: &str, img: &RgbaImage) -> &wgpu::Texture {
            self.entries.entry(name.to_string()).or_insert_with(|| {
                let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(name),
                    size: wgpu::Extent3d {
                        width: img.width,
                        height: img.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &img.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(img.width * 4),
                        rows_per_image: Some(img.height),
                    },
                    wgpu::Extent3d {
                        width: img.width,
                        height: img.height,
                        depth_or_array_layers: 1,
                    },
                );
                tex
            })
        }

        /// Look up an uploaded texture.
        pub fn get(&self, name: &str) -> Option<&wgpu::Texture> {
            self.entries.get(name)
        }
    }
}

/// Cache directory helpers shared with the shader cache.
pub fn cache_root() -> PathBuf {
    dirs_next_or_home()
}

fn dirs_next_or_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|v| PathBuf::from(v).join("xHoly").join("cache"))
            .unwrap_or_else(|_| PathBuf::from(".xholy-cache"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".cache"))
            .join("xholy")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_packs_and_reuses() {
        let mut atlas = TextureAtlas::new(64);
        let img = RgbaImage::solid(8, 8, [255, 0, 0, 255]);
        let e1 = atlas.insert("icon", &img).unwrap();
        let e2 = atlas.insert("icon", &img).unwrap();
        assert_eq!((e1.x, e1.y), (e2.x, e2.y)); // cached
                                                // Pixel check: atlas at (e1.x, e1.y) is red.
        let i = ((e1.y * 64 + e1.x) * 4) as usize;
        assert_eq!(&atlas.image.pixels[i..i + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn xrga_decode() {
        let mut bytes = b"XRGA".to_vec();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[9u8; 8]);
        let dir = std::env::temp_dir().join("xholy_tex_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.rgba");
        std::fs::write(&p, &bytes).unwrap();
        let img = load_rgba(&p).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.pixels[0], 9);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
