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

/// xHoly texture container header (DXBC-style, Phase 12).
///
/// 17 bytes: magic `DXBC` + format u8 + flags u32 + width u32 +
/// height u32 (little-endian, unaligned).
/// Pixel payload follows immediately; its encoding depends on `format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxbcTextureHeader {
    /// [`TextureFormat`] discriminant.
    pub format: TextureFormat,
    /// Reserved flag bits.
    pub flags: u32,
    pub width: u32,
    pub height: u32,
}

/// Encoded texture formats understood by [`decode_dxbc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureFormat {
    /// Uncompressed RGBA8.
    Rgba8888 = 1,
    /// BC1/DXT1 (4bpp block compression) — decoded to RGBA8.
    Dxt1 = 2,
    /// BC7 — header parses but decoding is not implemented yet;
    /// [`decode_dxbc`] returns a dummy RGBA8 image honestly.
    Bc7 = 3,
}

impl TextureFormat {
    /// Bytes per pixel for uncompressed formats (BC formats use blocks).
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8888 => 4,
            Self::Dxt1 | Self::Bc7 => 0, // block-compressed
        }
    }
}

/// Parse a DXBC-style texture header. Returns `None` on bad magic,
/// unknown format, or truncated input.
pub fn parse_dxbc_header(bytes: &[u8]) -> Option<DxbcTextureHeader> {
    if bytes.len() < 17 || &bytes[0..4] != b"DXBC" {
        return None;
    }
    let format = match bytes[4] {
        1 => TextureFormat::Rgba8888,
        2 => TextureFormat::Dxt1,
        3 => TextureFormat::Bc7,
        _ => return None,
    };
    Some(DxbcTextureHeader {
        format,
        flags: u32::from_le_bytes(bytes[5..9].try_into().ok()?),
        width: u32::from_le_bytes(bytes[9..13].try_into().ok()?),
        height: u32::from_le_bytes(bytes[13..17].try_into().ok()?),
    })
}

/// Decode a DXBC-container image to RGBA8.
///
/// - `Rgba8888`: payload copied verbatim.
/// - `Dxt1`: real BC1 decode (4bpp blocks, 1/0-bit alpha).
/// - `Bc7`: **not implemented** — returns a magenta dummy so callers
///   get a visible placeholder instead of garbage (honest fallback).
///
/// Returns `Err` on truncated payloads or unknown formats.
pub fn decode_dxbc(bytes: &[u8]) -> Result<RgbaImage, String> {
    let h = parse_dxbc_header(bytes).ok_or_else(|| "bad DXBC header".to_string())?;
    let payload = &bytes[17..];
    match h.format {
        TextureFormat::Rgba8888 => {
            let need = (h.width * h.height * 4) as usize;
            if payload.len() < need {
                return Err("truncated RGBA8888 payload".into());
            }
            Ok(RgbaImage {
                width: h.width,
                height: h.height,
                pixels: payload[..need].to_vec(),
            })
        }
        TextureFormat::Dxt1 => decode_dxt1(payload, h.width, h.height),
        TextureFormat::Bc7 => {
            // BC7 decode is not implemented; emit a visible placeholder.
            log::warn!(
                "xHoly: BC7 texture {}x{} — dummy fallback",
                h.width,
                h.height
            );
            Ok(RgbaImage::solid(
                h.width.max(1),
                h.height.max(1),
                [255, 0, 255, 255],
            ))
        }
    }
}

/// Decode BC1/DXT1 (8-byte blocks: two RGB565 endpoints + 4×4 2-bit
/// indices) to RGBA8. 1-bit alpha via endpoint-0 > endpoint-1.
fn decode_dxt1(data: &[u8], width: u32, height: u32) -> Result<RgbaImage, String> {
    let bw = width.div_ceil(4);
    let bh = height.div_ceil(4);
    let need = (bw * bh * 8) as usize;
    if data.len() < need {
        return Err(format!("truncated DXT1 payload: {} < {}", data.len(), need));
    }
    let mut img = RgbaImage::solid(width, height, [0, 0, 0, 0]);
    for by in 0..bh {
        for bx in 0..bw {
            let off = ((by * bw + bx) * 8) as usize;
            let c0 = u16::from_le_bytes([data[off], data[off + 1]]);
            let c1 = u16::from_le_bytes([data[off + 2], data[off + 3]]);
            let bits =
                u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]);
            let (r0, g0, b0) = rgb565(c0);
            let (r1, g1, b1) = rgb565(c1);
            let has_alpha = c0 > c1;
            // 4-color palette; 3-color adds transparent index 3.
            let pal = if has_alpha {
                [
                    (r0, g0, b0, 255u8),
                    (r1, g1, b1, 255u8),
                    (
                        ((2 * r0 as u16 + r1 as u16) / 3) as u8,
                        ((2 * g0 as u16 + g1 as u16) / 3) as u8,
                        ((2 * b0 as u16 + b1 as u16) / 3) as u8,
                        255,
                    ),
                    (0, 0, 0, 0),
                ]
            } else {
                [
                    (r0, g0, b0, 255u8),
                    (r1, g1, b1, 255u8),
                    (
                        ((r0 as u16 + r1 as u16) / 2) as u8,
                        ((g0 as u16 + g1 as u16) / 2) as u8,
                        ((b0 as u16 + b1 as u16) / 2) as u8,
                        255,
                    ),
                    (
                        ((r0 as u16 + 2 * r1 as u16) / 3) as u8,
                        ((g0 as u16 + 2 * g1 as u16) / 3) as u8,
                        ((b0 as u16 + 2 * b1 as u16) / 3) as u8,
                        255,
                    ),
                ]
            };
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= width || y >= height {
                        continue;
                    }
                    let idx = ((bits >> ((py * 4 + px) * 2)) & 3) as usize;
                    let (r, g, b, a) = pal[idx];
                    let i = ((y * width + x) * 4) as usize;
                    img.pixels[i..i + 4].copy_from_slice(&[r, g, b, a]);
                }
            }
        }
    }
    Ok(img)
}

/// Expand RGB565 to (r,g,b) u8.
fn rgb565(c: u16) -> (u8, u8, u8) {
    let r = (c >> 11) & 0x1F;
    let g = (c >> 5) & 0x3F;
    let b = c & 0x1F;
    (
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    )
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
    fn dxbc_rgba8888_roundtrip() {
        let mut bytes = b"DXBC".to_vec();
        bytes.push(TextureFormat::Rgba8888 as u8);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&1u32.to_le_bytes()); // w
        bytes.extend_from_slice(&1u32.to_le_bytes()); // h
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        let img = decode_dxbc(&bytes).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.pixels, vec![1, 2, 3, 4]);
    }

    #[test]
    fn dxbc_dxt1_decodes_solid_block() {
        // 4x4 DXT1 block: c0=white(0xFFFF→actually 0xFFFF is white),
        // c1=black → indices 0 pick c0.
        let mut bytes = b"DXBC".to_vec();
        bytes.push(TextureFormat::Dxt1 as u8);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&0xFFFFu16.to_le_bytes()); // c0 white
        bytes.extend_from_slice(&0x0000u16.to_le_bytes()); // c1 black
        bytes.extend_from_slice(&0u32.to_le_bytes()); // all indices 0 → white
        let img = decode_dxbc(&bytes).unwrap();
        assert_eq!(img.pixels.len(), 4 * 4 * 4);
        assert_eq!(&img.pixels[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn dxbc_bc7_returns_dummy() {
        let mut bytes = b"DXBC".to_vec();
        bytes.push(TextureFormat::Bc7 as u8);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, 16));
        let img = decode_dxbc(&bytes).unwrap();
        assert_eq!((img.width, img.height), (4, 4));
        assert_eq!(img.pixels[0], 255); // magenta placeholder
    }

    #[test]
    fn dxbc_rejects_bad_magic_and_truncation() {
        assert!(parse_dxbc_header(b"NOPE").is_none());
        assert!(decode_dxbc(b"DXBC").is_err());
        let mut t = b"DXBC".to_vec();
        t.push(1u8);
        t.extend(0u32.to_le_bytes()); // flags
        t.extend(4u32.to_le_bytes()); // w
        t.extend(4u32.to_le_bytes()); // h
        t.extend(std::iter::repeat_n(0u8, 3)); // payload too short
        assert!(decode_dxbc(&t).is_err());
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
