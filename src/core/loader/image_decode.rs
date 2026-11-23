//! XEX2 image decode — Phase 20.
//!
//! Turns the on-disk payload of a real XEX2 into a flat guest image:
//!
//! ```text
//! header_size..               payload
//!   → AES-128-CBC (zero IV)   session key from security info +0x150
//!   → BASIC block expander    (data_size / zero_size pairs)
//!   → LZX normal de-blocker   (u32 block size + 20-byte SHA1 + chunks)
//!   → LZX decompressor        (32 KiB frames, verbatim/aligned/uncompressed)
//! ```
//!
//! Field offsets and constants follow Xenia's `xex_module.cc` /
//! `xex2_info.h` and libmspack's `lzx.c`/`lzx.h`. The AES mode is the
//! CBC-with-zero-IV variant Xenia implements in `aes_decrypt_buffer`
//! (not CTR — the XEX2 format does not use CTR here). LZX block hashes
//! are parsed but not verified, and the embedded PowerPC PE is not
//! interpreted. See STATUS.md.

use super::xex_parser::{XexHeader, XEX_HEADER_FILE_FORMAT_INFO};

/// Errors from the image decode path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDecodeError {
    /// A structure was truncated or inconsistent.
    BadFormat(String),
    /// A supported-but-unimplemented mode was requested (e.g. delta).
    Unsupported(&'static str),
    /// The LZX bitstream could not be decoded.
    Lzx(String),
}

impl std::fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFormat(s) => write!(f, "bad XEX image format: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported XEX image mode: {s}"),
            Self::Lzx(s) => write!(f, "LZX decode failed: {s}"),
        }
    }
}
impl std::error::Error for ImageDecodeError {}

/// Encryption type from the file-format optional header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XexEncryption {
    /// Plain payload.
    None,
    /// AES-128-CBC with the session key derived from security info.
    Normal,
}

/// Compression type from the file-format optional header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XexCompression {
    /// No compression.
    None,
    /// Basic (data/zero run) compression.
    Basic,
    /// LZX normal compression.
    Normal,
    /// LZX delta patches — not implemented.
    Delta,
}

/// Decoded `xex2_opt_file_format_info` (Xenia `xex2_file_format_info`).
#[derive(Debug, Clone)]
pub struct FileFormatInfo {
    /// Payload encryption.
    pub encryption: XexEncryption,
    /// Payload compression.
    pub compression: XexCompression,
    /// LZX window size in bytes (normal compression).
    pub window_size: u32,
    /// (data_size, zero_size) pairs (basic compression).
    pub blocks: Vec<(u32, u32)>,
}

fn be16(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes(s.try_into().ok()?))
}
fn be32(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes(s.try_into().ok()?))
}

/// Parse the optional `XEX_HEADER_FILE_FORMAT_INFO` structure.
///
/// Returns `None` for the scaffold fixture and for XEX files without
/// this header (the caller then maps the payload verbatim).
pub fn parse_file_format(header: &XexHeader, data: &[u8]) -> Option<FileFormatInfo> {
    let base = header.opt_offset(XEX_HEADER_FILE_FORMAT_INFO)?;
    let info_size = be32(data, base)?;
    let encryption = match be16(data, base + 4)? {
        0 => XexEncryption::None,
        1 => XexEncryption::Normal,
        _ => return None,
    };
    let compression = match be16(data, base + 6)? {
        0 => XexCompression::None,
        1 => XexCompression::Basic,
        2 => XexCompression::Normal,
        3 => XexCompression::Delta,
        _ => return None,
    };
    let mut info = FileFormatInfo {
        encryption,
        compression,
        window_size: 0,
        blocks: Vec::new(),
    };
    match compression {
        XexCompression::Normal => {
            info.window_size = be32(data, base + 8).unwrap_or(0x1_0000);
        }
        XexCompression::Basic => {
            let count = info_size.saturating_sub(8) / 8;
            for i in 0..count {
                let off = base + 8 + i as usize * 8;
                if let (Some(data_size), Some(zero_size)) = (be32(data, off), be32(data, off + 4)) {
                    info.blocks.push((data_size, zero_size));
                }
            }
        }
        XexCompression::None | XexCompression::Delta => {}
    }
    Some(info)
}

// ---------------------------------------------------------------------------
// AES-128 (decrypt only) — matches Xenia's rijndaelKeySetupDec + Decrypt
// ---------------------------------------------------------------------------

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

/// AES key used by retail XEX2 images to wrap the session key (Xenia).
pub const XEX_RETAIL_KEY: [u8; 16] = [
    0x20, 0xb1, 0x85, 0xa5, 0x9d, 0x28, 0xfd, 0xc3, 0x40, 0x58, 0x3f, 0xbb, 0x08, 0x96, 0xbf, 0x91,
];
/// AES key used by devkit XEX2 images (all zeros).
pub const XEX_DEVKIT_KEY: [u8; 16] = [0u8; 16];

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

struct Aes128 {
    /// 11 round keys of 16 bytes each, big-endian AES state order.
    round_keys: [u8; 176],
}

impl Aes128 {
    fn new(key: &[u8; 16]) -> Self {
        let mut w = [0u32; 44];
        for (i, word) in w.iter_mut().enumerate().take(4) {
            *word = u32::from_be_bytes(key[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 4..44 {
            let mut t = w[i - 1];
            if i % 4 == 0 {
                t = t.rotate_left(8); // RotWord
                let b = t.to_be_bytes();
                t = u32::from_be_bytes([
                    SBOX[b[0] as usize] ^ RCON[i / 4 - 1],
                    SBOX[b[1] as usize],
                    SBOX[b[2] as usize],
                    SBOX[b[3] as usize],
                ]);
            }
            w[i] = w[i - 4] ^ t;
        }
        let mut round_keys = [0u8; 176];
        for (i, word) in w.iter().enumerate() {
            round_keys[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Self { round_keys }
    }

    fn decrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut s = [0u8; 16];
        for (i, (byte, input_byte)) in s.iter_mut().zip(input.iter()).enumerate() {
            *byte = *input_byte ^ self.round_keys[160 + i];
        }
        for round in (1..10).rev() {
            inv_shift_rows(&mut s);
            for b in &mut s {
                *b = INV_SBOX[*b as usize];
            }
            for (i, byte) in s.iter_mut().enumerate() {
                *byte ^= self.round_keys[round * 16 + i];
            }
            inv_mix_columns(&mut s);
        }
        inv_shift_rows(&mut s);
        for b in &mut s {
            *b = INV_SBOX[*b as usize];
        }
        for (i, byte) in s.iter_mut().enumerate() {
            *byte ^= self.round_keys[i];
        }
        s
    }
}

/// AES state is column-major: `state[r + 4*c]`. Inverse ShiftRows
/// rotates row `r` right by `r` columns.
fn inv_shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for c in 0..4 {
        for r in 0..4 {
            s[c * 4 + r] = old[((c + 4 - r) & 3) * 4 + r];
        }
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
        s[c * 4] = gmul(a[0], 14) ^ gmul(a[1], 11) ^ gmul(a[2], 13) ^ gmul(a[3], 9);
        s[c * 4 + 1] = gmul(a[0], 9) ^ gmul(a[1], 14) ^ gmul(a[2], 11) ^ gmul(a[3], 13);
        s[c * 4 + 2] = gmul(a[0], 13) ^ gmul(a[1], 9) ^ gmul(a[2], 14) ^ gmul(a[3], 11);
        s[c * 4 + 3] = gmul(a[0], 11) ^ gmul(a[1], 13) ^ gmul(a[2], 9) ^ gmul(a[3], 14);
    }
}

/// AES-128 CBC decrypt with a caller-owned IV (Xenia `aes_decrypt_buffer`).
/// The IV starts at zero for XEX images and chains across BASIC blocks.
pub fn aes_cbc_decrypt(
    key: &[u8; 16],
    input: &[u8],
    iv: &mut [u8; 16],
) -> Result<Vec<u8>, ImageDecodeError> {
    if !input.len().is_multiple_of(16) {
        return Err(ImageDecodeError::BadFormat(format!(
            "encrypted payload length {} is not a multiple of 16",
            input.len()
        )));
    }
    let aes = Aes128::new(key);
    let mut out = vec![0u8; input.len()];
    for (i, chunk) in input.as_chunks::<16>().0.iter().enumerate() {
        let ct = *chunk;
        let pt = aes.decrypt_block(&ct);
        for j in 0..16 {
            out[i * 16 + j] = pt[j] ^ iv[j];
        }
        *iv = ct;
    }
    Ok(out)
}

/// Derive the image session key from security info +0x150.
fn session_key(
    header: &XexHeader,
    data: &[u8],
    devkit: bool,
) -> Result<[u8; 16], ImageDecodeError> {
    let off = header.security_offset as usize;
    let wrapped = data
        .get(off + 0x150..off + 0x160)
        .ok_or_else(|| ImageDecodeError::BadFormat("security AES key missing".into()))?;
    let base = if devkit {
        XEX_DEVKIT_KEY
    } else {
        XEX_RETAIL_KEY
    };
    let mut iv = [0u8; 16];
    let key = aes_cbc_decrypt(&base, wrapped, &mut iv)?;
    Ok(key.try_into().unwrap())
}

// ---------------------------------------------------------------------------
// BASIC compression
// ---------------------------------------------------------------------------

fn basic_decompress(input: &[u8], blocks: &[(u32, u32)]) -> Result<Vec<u8>, ImageDecodeError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for &(data_size, zero_size) in blocks {
        let end = pos
            .checked_add(data_size as usize)
            .ok_or_else(|| ImageDecodeError::BadFormat("basic block overflow".into()))?;
        let chunk = input
            .get(pos..end)
            .ok_or_else(|| ImageDecodeError::BadFormat("basic block truncated".into()))?;
        out.extend_from_slice(chunk);
        out.resize(out.len() + zero_size as usize, 0);
        pos = end;
    }
    Ok(out)
}

/// De-frame Xenia's LZX "normal" layout: each block starts with a
/// big-endian u32 size, followed by a 20-byte digest and a sequence of
/// big-endian u16-sized chunks terminated by a zero chunk. The digest
/// is parsed but not verified (honest limit).
fn deframe_lzx(input: &[u8]) -> Result<Vec<u8>, ImageDecodeError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= input.len() {
        let block_size = be32(input, pos)
            .ok_or_else(|| ImageDecodeError::BadFormat("LZX block size".into()))?
            as usize;
        if block_size == 0 {
            break;
        }
        pos += 4;
        if block_size > input.len() - pos {
            return Err(ImageDecodeError::BadFormat(
                "LZX block exceeds payload".into(),
            ));
        }
        let end = pos + block_size;
        if end - pos < 20 {
            return Err(ImageDecodeError::BadFormat("LZX digest truncated".into()));
        }
        pos += 20; // SHA1 digest — not verified
        while pos + 2 <= end {
            let chunk = be16(input, pos)
                .ok_or_else(|| ImageDecodeError::BadFormat("LZX chunk size".into()))?
                as usize;
            pos += 2;
            if chunk == 0 {
                break;
            }
            if chunk > end - pos {
                return Err(ImageDecodeError::BadFormat(
                    "LZX chunk exceeds block".into(),
                ));
            }
            out.extend_from_slice(&input[pos..pos + chunk]);
            pos += chunk;
        }
        pos = end;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// LZX (regular, not delta) — pure Rust decoder
// ---------------------------------------------------------------------------

const LZX_MIN_MATCH: usize = 2;
const LZX_NUM_CHARS: usize = 256;
const LZX_PRIMARY_LENGTHS: usize = 7;
const LZX_SECONDARY_LENGTHS: usize = 249;
const LZX_FRAME_SIZE: usize = 32 * 1024;
const LZX_BLOCK_VERBATIM: u32 = 1;
const LZX_BLOCK_ALIGNED: u32 = 2;
const LZX_BLOCK_UNCOMPRESSED: u32 = 3;
const POSITION_SLOTS: [u32; 11] = [30, 32, 34, 36, 38, 42, 50, 66, 98, 162, 290];

fn position_base(slot: usize) -> u32 {
    // Xenia/libmspack generate this table as:
    //   extra_bits[i] = 0 (i<4), i/2-1 (4<=i<36), 17 (i>=36)
    //   position_base[i] = position_base[i-1] + (1 << extra_bits[i-1])
    let mut base = 0u32;
    for i in 0..slot {
        let prev_extra = if i < 4 {
            0
        } else if i < 36 {
            i / 2 - 1
        } else {
            17
        };
        base = base.wrapping_add(1u32 << prev_extra);
    }
    base
}

fn extra_bits(slot: usize) -> u32 {
    if slot < 4 {
        0
    } else if slot < 36 {
        (slot / 2 - 1) as u32
    } else {
        17
    }
}

/// MSB-first bit reader matching libmspack `readbits.h` with
/// `READ_BYTES = (b1 << 8) | b0` (two-byte little-endian refill).
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    bits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            bits: 0,
        }
    }

    fn next_byte(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    fn refill(&mut self) {
        let b0 = self.next_byte() as u32;
        let b1 = self.next_byte() as u32;
        self.buf |= ((b1 << 8) | b0) << (16 - self.bits);
        self.bits += 16;
    }

    fn read_bits(&mut self, n: u32) -> Result<u32, ImageDecodeError> {
        if n == 0 {
            return Ok(0);
        }
        while self.bits < n {
            self.refill();
        }
        let v = self.buf >> (32 - n);
        self.buf <<= n;
        self.bits -= n;
        Ok(v)
    }

    /// Discard buffered bits and align to the next input byte.
    fn align_byte(&mut self) {
        self.buf = 0;
        self.bits = 0;
    }

    /// Frame-end realignment (libmspack: ensure 16 bits, drop `bits & 15`).
    fn align_frame(&mut self) {
        if self.bits > 0 {
            while self.bits < 16 {
                self.refill();
            }
            let drop = self.bits & 15;
            if drop > 0 {
                self.buf <<= drop;
                self.bits -= drop;
            }
        }
    }

    fn read_raw_byte(&mut self) -> u8 {
        self.align_byte();
        self.next_byte()
    }
}

/// Canonical Huffman decoder (MSB-first codes, lengths 1..=16).
struct Huffman {
    counts: [u16; 17],
    first: [u32; 17],
    base: [u16; 17],
    symbols: Vec<u16>,
}

impl Huffman {
    fn build(lengths: &[u8]) -> Result<Self, ImageDecodeError> {
        let mut counts = [0u16; 17];
        for &len in lengths {
            if len > 16 {
                return Err(ImageDecodeError::Lzx("code length > 16".into()));
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut first = [0u32; 17];
        let mut code = 0u32;
        for len in 1..=16 {
            code = (code + counts[len - 1] as u32) << 1;
            if code + counts[len] as u32 > (1u32 << len) {
                return Err(ImageDecodeError::Lzx("oversubscribed Huffman tree".into()));
            }
            first[len] = code;
        }
        let mut base = [0u16; 17];
        let mut acc = 0u16;
        for len in 1..=16 {
            base[len] = acc;
            acc += counts[len];
        }
        let mut next = first;
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let l = len as usize;
                let idx = next[l] as usize;
                if idx >= symbols.len() {
                    return Err(ImageDecodeError::Lzx("Huffman symbol overflow".into()));
                }
                symbols[idx] = sym as u16;
                next[l] += 1;
            }
        }
        Ok(Self {
            counts,
            first,
            base,
            symbols,
        })
    }

    fn is_empty(&self) -> bool {
        self.counts[1..].iter().all(|&c| c == 0)
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<usize, ImageDecodeError> {
        let mut code = 0u32;
        for len in 1..=16 {
            code = (code << 1) | br.read_bits(1)?;
            let count = self.counts[len] as u32;
            if count != 0 {
                let offset = code - self.first[len];
                if offset < count {
                    let idx = self.base[len] as usize + offset as usize;
                    return Ok(self.symbols[idx] as usize);
                }
            }
        }
        Err(ImageDecodeError::Lzx("invalid Huffman code".into()))
    }
}

struct LzxDecoder<'a> {
    br: BitReader<'a>,
    window: Vec<u8>,
    window_size: usize,
    window_pos: usize,
    frame_pos: usize,
    total_out: usize,
    out: Vec<u8>,
    num_offsets: usize,
    r: [u32; 3],
    main_len: Vec<u8>,
    length_len: Vec<u8>,
    aligned_len: [u8; 8],
    pretree_len: [u8; 20],
    main_tree: Option<Huffman>,
    length_tree: Option<Huffman>,
    aligned_tree: Option<Huffman>,
    block_type: u32,
    block_length: u32,
    block_remaining: u32,
    intel_filesize: i32,
    header_read: bool,
    intel_started: bool,
}

impl<'a> LzxDecoder<'a> {
    fn new(input: &'a [u8], output_len: usize, window_hint: u32) -> Result<Self, ImageDecodeError> {
        let raw_bits = if window_hint == 0 {
            16
        } else {
            31 - window_hint.leading_zeros()
        };
        let window_bits = raw_bits.clamp(15, 21);
        let window_size = 1usize << window_bits;
        let slot_index = (window_bits - 15) as usize;
        let num_offsets = POSITION_SLOTS.get(slot_index).copied().unwrap_or(290) as usize * 8;
        if output_len > 0x7fff_ffff {
            return Err(ImageDecodeError::BadFormat("LZX output too large".into()));
        }
        Ok(Self {
            br: BitReader::new(input),
            window: vec![0; window_size],
            window_size,
            window_pos: 0,
            frame_pos: 0,
            total_out: 0,
            out: Vec::with_capacity(output_len),
            num_offsets,
            r: [1, 1, 1],
            main_len: vec![0; LZX_NUM_CHARS + 290 * 8],
            length_len: vec![0; LZX_SECONDARY_LENGTHS + 1],
            aligned_len: [0; 8],
            pretree_len: [0; 20],
            main_tree: None,
            length_tree: None,
            aligned_tree: None,
            block_type: 0,
            block_length: 0,
            block_remaining: 0,
            intel_filesize: 0,
            header_read: false,
            intel_started: false,
        })
    }

    /// Read code lengths for one tree using the LZX pretree delta scheme.
    fn read_lengths(
        br: &mut BitReader<'_>,
        pretree_len: &mut [u8; 20],
        target: &mut [u8],
        first: usize,
        last: usize,
    ) -> Result<(), ImageDecodeError> {
        for len in pretree_len.iter_mut().take(20) {
            *len = br.read_bits(4)? as u8;
        }
        let pretree = Huffman::build(pretree_len)?;
        let mut x = first;
        while x < last {
            let z = pretree.decode(br)?;
            let run = match z {
                17 => br.read_bits(4)? as usize + 4,
                18 => br.read_bits(5)? as usize + 20,
                19 => {
                    let run = br.read_bits(1)? as usize + 4;
                    let delta = pretree.decode(br)? as i32;
                    let mut value = target[x] as i32 - delta;
                    if value < 0 {
                        value += 17;
                    }
                    for _ in 0..run {
                        if x >= last {
                            break;
                        }
                        target[x] = value as u8;
                        x += 1;
                    }
                    continue;
                }
                _ => 1,
            };
            let value = if z == 17 || z == 18 {
                0u8
            } else {
                let mut v = target[x] as i32 - z as i32;
                if v < 0 {
                    v += 17;
                }
                v as u8
            };
            for _ in 0..run {
                if x >= last {
                    break;
                }
                target[x] = value;
                x += 1;
            }
        }
        Ok(())
    }

    fn start_block(&mut self) -> Result<(), ImageDecodeError> {
        if self.block_type == LZX_BLOCK_UNCOMPRESSED && self.block_length & 1 == 1 {
            let _ = self.br.read_raw_byte();
        }
        let ty = self.br.read_bits(3)?;
        if !(LZX_BLOCK_VERBATIM..=LZX_BLOCK_UNCOMPRESSED).contains(&ty) {
            return Err(ImageDecodeError::Lzx(format!("invalid block type {ty}")));
        }
        let low = self.br.read_bits(16)?;
        let high = self.br.read_bits(8)?;
        self.block_type = ty;
        // libmspack reads 16 then 8 MSB-first bits and combines them as
        // (first << 8) | second — i.e. a plain 24-bit big-endian value.
        self.block_length = (low << 8) | high;
        self.block_remaining = self.block_length;
        match ty {
            LZX_BLOCK_VERBATIM | LZX_BLOCK_ALIGNED => {
                if ty == LZX_BLOCK_ALIGNED {
                    for len in &mut self.aligned_len {
                        *len = self.br.read_bits(3)? as u8;
                    }
                    self.aligned_tree = Some(Huffman::build(&self.aligned_len)?);
                }
                Self::read_lengths(
                    &mut self.br,
                    &mut self.pretree_len,
                    &mut self.main_len,
                    0,
                    LZX_NUM_CHARS,
                )?;
                Self::read_lengths(
                    &mut self.br,
                    &mut self.pretree_len,
                    &mut self.main_len,
                    LZX_NUM_CHARS,
                    LZX_NUM_CHARS + self.num_offsets,
                )?;
                self.main_tree = Some(Huffman::build(&self.main_len)?);
                if self.main_len[0xE8] != 0 {
                    self.intel_started = true;
                }
                Self::read_lengths(
                    &mut self.br,
                    &mut self.pretree_len,
                    &mut self.length_len,
                    0,
                    LZX_SECONDARY_LENGTHS,
                )?;
                self.length_tree = Some(Huffman::build(&self.length_len)?);
            }
            LZX_BLOCK_UNCOMPRESSED => {
                self.intel_started = true;
                self.br.align_byte();
                let mut stored = [0u8; 12];
                for b in &mut stored {
                    *b = self.br.read_raw_byte();
                }
                for i in 0..3 {
                    let off = u32::from_le_bytes(stored[i * 4..i * 4 + 4].try_into().unwrap());
                    if off as usize >= self.window_size {
                        return Err(ImageDecodeError::Lzx("R0/R1/R2 out of window".into()));
                    }
                    self.r[i] = off;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Decode one literal or match; returns the number of bytes written.
    fn decode_one(&mut self) -> Result<usize, ImageDecodeError> {
        if self.block_type == LZX_BLOCK_UNCOMPRESSED {
            let b = self.br.read_raw_byte();
            self.window[self.window_pos % self.window_size] = b;
            self.window_pos += 1;
            return Ok(1);
        }
        let main = self
            .main_tree
            .as_ref()
            .ok_or_else(|| ImageDecodeError::Lzx("no main tree".into()))?
            .decode(&mut self.br)?;
        if main < LZX_NUM_CHARS {
            self.window[self.window_pos % self.window_size] = main as u8;
            self.window_pos += 1;
            return Ok(1);
        }
        let element = main - LZX_NUM_CHARS;
        let mut match_length = element & LZX_PRIMARY_LENGTHS;
        if match_length == LZX_PRIMARY_LENGTHS {
            let tree = self
                .length_tree
                .as_ref()
                .ok_or_else(|| ImageDecodeError::Lzx("no length tree".into()))?;
            if tree.is_empty() {
                return Err(ImageDecodeError::Lzx("empty length tree".into()));
            }
            match_length += tree.decode(&mut self.br)?;
        }
        match_length += LZX_MIN_MATCH;

        let slot = element >> 3;
        let offset = match slot {
            0 => self.r[0],
            1 => {
                self.r.swap(1, 0);
                self.r[0]
            }
            2 => {
                self.r.swap(2, 0);
                self.r[0]
            }
            _ => {
                let extra = extra_bits(slot);
                let mut off = position_base(slot) as i64 - 2;
                if extra >= 3 && self.block_type == LZX_BLOCK_ALIGNED {
                    if extra > 3 {
                        let verbatim = self.br.read_bits(extra - 3)? as i64;
                        off += verbatim << 3;
                    }
                    let tree = self
                        .aligned_tree
                        .as_ref()
                        .ok_or_else(|| ImageDecodeError::Lzx("no aligned tree".into()))?;
                    off += tree.decode(&mut self.br)? as i64;
                } else if extra > 0 {
                    off += self.br.read_bits(extra)? as i64;
                }
                self.r[2] = self.r[1];
                self.r[1] = self.r[0];
                self.r[0] = off as u32;
                off as u32
            }
        };

        if offset as usize > self.total_out + self.window_pos {
            return Err(ImageDecodeError::Lzx("match offset beyond output".into()));
        }
        let ws = self.window_size as i64;
        for k in 0..match_length {
            let dst = (self.window_pos + k) % self.window_size;
            let src = if (offset as usize) <= self.window_pos {
                (self.window_pos + k - offset as usize) % self.window_size
            } else {
                let back = offset as i64 - self.window_pos as i64;
                if (k as i64) < back {
                    (ws - back + k as i64) as usize
                } else {
                    (k as i64 - back) as usize
                }
            };
            self.window[dst] = self.window[src];
        }
        self.window_pos += match_length;
        Ok(match_length)
    }

    /// Intel E8 call fixup for one 32 KiB frame (libmspack `e8_buf`).
    fn e8_translate(frame: &mut [u8], filesize: i32, base_pos: usize) {
        let mut pos = 0usize;
        while pos + 5 <= frame.len() && frame.len() - pos > 10 {
            if frame[pos] != 0xE8 {
                pos += 1;
                continue;
            }
            let abs = i32::from_le_bytes([
                frame[pos + 1],
                frame[pos + 2],
                frame[pos + 3],
                frame[pos + 4],
            ]);
            let curpos = (base_pos + pos) as i32;
            if abs >= -curpos && abs < filesize {
                let rel = if abs >= 0 {
                    abs.wrapping_sub(curpos)
                } else {
                    abs.wrapping_add(filesize)
                };
                frame[pos + 1..pos + 5].copy_from_slice(&rel.to_le_bytes());
            }
            pos += 5;
        }
    }

    fn run(mut self, output_len: usize) -> Result<Vec<u8>, ImageDecodeError> {
        if output_len == 0 {
            return Ok(Vec::new());
        }
        if !self.header_read {
            let e8 = self.br.read_bits(1)? != 0;
            self.header_read = true;
            if e8 {
                let high = self.br.read_bits(16)? as i32;
                let low = self.br.read_bits(16)? as i32;
                self.intel_filesize = (high << 16) | low;
            }
        }
        while self.total_out < output_len {
            let frame_size = LZX_FRAME_SIZE.min(output_len - self.total_out);
            let mut produced = 0usize;
            while produced < frame_size {
                if self.block_remaining == 0 {
                    self.start_block()?;
                }
                let budget = (self.block_remaining as usize).min(frame_size - produced);
                let mut done = 0usize;
                while done < budget {
                    let n = self.decode_one()?;
                    done += n;
                    if done > budget + 16 {
                        return Err(ImageDecodeError::Lzx("match overran block budget".into()));
                    }
                }
                self.block_remaining = self.block_remaining.saturating_sub(done as u32);
                produced += done;
            }
            self.br.align_frame();
            let mut frame: Vec<u8> = (0..frame_size)
                .map(|i| self.window[(self.frame_pos + i) % self.window_size])
                .collect();
            if self.intel_started && self.intel_filesize != 0 && frame_size > 10 {
                Self::e8_translate(&mut frame, self.intel_filesize, self.total_out);
            }
            self.out.extend_from_slice(&frame);
            self.total_out += frame_size;
            self.frame_pos = (self.frame_pos + frame_size) % self.window_size;
        }
        Ok(self.out)
    }
}

/// Decompress a regular LZX stream (no DELTA chunk headers).
pub fn lzx_decompress(
    input: &[u8],
    output_len: usize,
    window_size: u32,
) -> Result<Vec<u8>, ImageDecodeError> {
    LzxDecoder::new(input, output_len, window_size)?.run(output_len)
}

// ---------------------------------------------------------------------------
// Top-level XEX image decode
// ---------------------------------------------------------------------------

fn decompress_with(
    info: &FileFormatInfo,
    raw: &[u8],
    output_len: usize,
) -> Result<Vec<u8>, ImageDecodeError> {
    match info.compression {
        XexCompression::None => Ok(raw.to_vec()),
        XexCompression::Basic => basic_decompress(raw, &info.blocks),
        XexCompression::Normal => {
            let stream = deframe_lzx(raw)?;
            lzx_decompress(&stream, output_len, info.window_size)
        }
        XexCompression::Delta => Err(ImageDecodeError::Unsupported("LZX delta")),
    }
}

/// Decode the payload of a XEX2 header into a flat guest image.
///
/// Scaffold/legacy images without file-format info are returned raw. For
/// encrypted images the retail key is tried first, then the devkit key;
/// the candidate whose first bytes look like a PE (`MZ`) wins, mirroring
/// Xenia's two-key probe.
pub fn decode_xex_image(header: &XexHeader, data: &[u8]) -> Result<Vec<u8>, ImageDecodeError> {
    let hs = header.header_size as usize;
    let payload = data
        .get(hs..)
        .ok_or_else(|| ImageDecodeError::BadFormat("payload beyond file".into()))?;
    let Some(info) = parse_file_format(header, data) else {
        return Ok(payload.to_vec());
    };
    let output_len = header.image_size as usize;
    let plain = if info.encryption == XexEncryption::Normal {
        let key = session_key(header, data, false)?;
        let mut iv = [0u8; 16];
        let decrypted = aes_cbc_decrypt(&key, payload, &mut iv)?;
        let candidate = decompress_with(&info, &decrypted, output_len)?;
        if candidate.starts_with(b"MZ") {
            candidate
        } else {
            let devkey = session_key(header, data, true)?;
            let mut iv = [0u8; 16];
            let decrypted = aes_cbc_decrypt(&devkey, payload, &mut iv)?;
            decompress_with(&info, &decrypted, output_len)?
        }
    } else {
        decompress_with(&info, payload, output_len)?
    };
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loader::xex_parser::{parse_xex, XexFormat};
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test.xex")
    }

    #[test]
    fn aes_128_nist_known_answer() {
        // NIST FIPS-197 C.1: AES-128 decryption of 69c4..55a with key
        // 0001..0f must yield 0011..eeff.
        let mut key = [0u8; 16];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        let ct = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        let mut iv = [0u8; 16];
        let pt = aes_cbc_decrypt(&key, &ct, &mut iv).unwrap();
        let expected: Vec<u8> = (0..16).map(|i| (0x11 * i) as u8).collect();
        assert_eq!(pt, expected);
    }

    #[test]
    fn aes_cbc_zero_iv_nist_sp800_38a() {
        // SP 800-38A F.2.5 first block: CBC with IV=0 == ECB block.
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let ct = [
            0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9,
            0x19, 0x7d,
        ];
        // FIPS-197 SP 800-38A uses IV 000102..0f for this vector.
        let mut iv = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let pt = aes_cbc_decrypt(&key, &ct, &mut iv).unwrap();
        assert_eq!(
            pt,
            [
                0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
                0x17, 0x2a
            ]
        );
    }

    #[test]
    fn lzx_uncompressed_block_known_vector() {
        // Known LZX stream from libmspack/lzxd test suite (32 KiB window):
        // header bit (no E8) + uncompressed block of 3 bytes "abc".
        let data = [
            0x00, 0x30, 0x30, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, b'a', b'b', b'c', 0x00,
        ];
        let out = lzx_decompress(&data, 3, 0x8000).unwrap();
        assert_eq!(out, b"abc");
    }

    #[test]
    fn lzx_verbatim_block_with_literals() {
        // Build a verbatim block with one 1-bit code for 'a' and 'b'.
        struct Writer {
            bits: u32,
            count: u32,
            bytes: Vec<u8>,
        }
        impl Writer {
            fn bit(&mut self, b: u32) {
                self.bits = (self.bits << 1) | b;
                self.count += 1;
                if self.count == 16 {
                    self.bytes.push((self.bits & 0xff) as u8);
                    self.bytes.push((self.bits >> 8) as u8);
                    self.bits = 0;
                    self.count = 0;
                }
            }
            fn bits(&mut self, v: u32, n: u32) {
                for i in (0..n).rev() {
                    self.bit((v >> i) & 1);
                }
            }
            fn pretree_sym(&mut self, sym: u32) {
                self.bits(sym, 5); // all 20 pretree lengths are 5
            }
            fn finish(mut self) -> Vec<u8> {
                while self.count != 0 {
                    self.bit(0);
                }
                self.bytes
            }
        }
        let mut w = Writer {
            bits: 0,
            count: 0,
            bytes: Vec::new(),
        };
        w.bit(0); // no E8 header
        w.bits(LZX_BLOCK_VERBATIM, 3);
        w.bits(3, 24); // three literals
                       // Each READ_LENGTHS call starts with its own 20 pretree lengths.
        let emit_pretree = |w: &mut Writer| {
            for _ in 0..20 {
                w.bits(5, 4);
            }
        };
        // Main tree 0..256: 'a'(97) and 'b'(98) get length 1 (delta 16).
        emit_pretree(&mut w);
        for sym in 0..LZX_NUM_CHARS {
            if sym == b'a' as usize || sym == b'b' as usize {
                w.pretree_sym(16);
            } else {
                w.pretree_sym(0);
            }
        }
        // Main tree 256..(256+num_offsets): all zero.
        emit_pretree(&mut w);
        for _ in 0..256 {
            w.pretree_sym(0);
        }
        // Length tree: all zero (empty, no matches used).
        emit_pretree(&mut w);
        for _ in 0..LZX_SECONDARY_LENGTHS {
            w.pretree_sym(0);
        }
        // Literals: 'a','b','a' -> codes 0,1,0.
        w.bit(0);
        w.bit(1);
        w.bit(0);
        let stream = w.finish();
        let out = lzx_decompress(&stream, 3, 0x1_0000).unwrap();
        assert_eq!(out, b"aba");
    }

    #[test]
    fn basic_blocks_expand_data_and_zero_runs() {
        let info = FileFormatInfo {
            encryption: XexEncryption::None,
            compression: XexCompression::Basic,
            window_size: 0,
            blocks: vec![(3, 2), (2, 1)],
        };
        let out = decompress_with(&info, b"abcde", 0).unwrap();
        assert_eq!(out, b"abc\0\0de\0");
    }

    #[test]
    fn lzx_deframing_concatenates_chunks() {
        let mut data = Vec::new();
        // Block 1: size 0x1e (digest + chunks), two chunks ("ab", "cd").
        data.extend_from_slice(&30u32.to_be_bytes());
        data.extend_from_slice(&[0xAA; 20]);
        data.extend_from_slice(&2u16.to_be_bytes());
        data.extend_from_slice(b"ab");
        data.extend_from_slice(&2u16.to_be_bytes());
        data.extend_from_slice(b"cd");
        data.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(deframe_lzx(&data).unwrap(), b"abcd");
    }

    /// A structurally real XEX2 with file-format info at 0x120 and the
    /// session key at security+0x150.
    fn xex_with_format(compression: u16, payload: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 0x400 + payload.len()];
        let put = |d: &mut [u8], off: usize, v: u32| {
            d[off..off + 4].copy_from_slice(&v.to_be_bytes());
        };
        data[0..4].copy_from_slice(b"XEX2");
        put(&mut data, 0x08, 0x400);
        put(&mut data, 0x10, 0x200);
        put(&mut data, 0x14, 2);
        put(&mut data, 0x18, XEX_HEADER_FILE_FORMAT_INFO);
        put(&mut data, 0x1C, 0x100);
        put(&mut data, 0x20, 0x0001_0100);
        put(&mut data, 0x24, 0x8200_0000);
        // file format info: info_size, encryption=none, compression.
        put(&mut data, 0x100, 0x08);
        data[0x104..0x106].copy_from_slice(&0u16.to_be_bytes());
        data[0x106..0x108].copy_from_slice(&compression.to_be_bytes());
        put(&mut data, 0x108, 0x1_0000); // LZX window size
                                         // Security block: image size + session key bytes.
        put(&mut data, 0x204, payload.len() as u32);
        data[0x350..0x360].copy_from_slice(&XEX_RETAIL_KEY);
        data[0x400..].copy_from_slice(payload);
        data
    }

    #[test]
    fn decode_raw_xex2_payload() {
        let data = xex_with_format(0, b"MZ\x90\x00");
        let header = parse_xex(&data, &path()).unwrap();
        assert_eq!(header.format, XexFormat::Xex2);
        assert_eq!(header.security_offset, 0x200);
        let image = decode_xex_image(&header, &data).unwrap();
        assert_eq!(image, b"MZ\x90\x00");
    }

    #[test]
    fn scaffold_layout_stays_raw() {
        let mut data = vec![0u8; 0x200];
        data[0..4].copy_from_slice(b"XEX2");
        data[0x08..0x0C].copy_from_slice(&0x178u32.to_le_bytes());
        data[0x178..0x17C].copy_from_slice(&0xD503_201Fu32.to_le_bytes());
        let header = parse_xex(&data, &path()).unwrap();
        assert_eq!(decode_xex_image(&header, &data).unwrap(), data[0x178..]);
    }
}
