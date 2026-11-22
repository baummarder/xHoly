//! Save compression — dependency-free RLE + optional "store" mode.
//!
//! Honest scope: this is run-length encoding for byte runs, not gzip.
//! It shrinks zero-heavy emulator state well; for real compression
//! swap in `zstd`/`flate2` behind the same API.

/// Compression tag stored in the save flags byte.
pub const TAG_STORE: u8 = 0;
pub const TAG_RLE: u8 = 1;

/// Compress `data`; returns (tag, bytes). Falls back to store when
/// RLE would expand the data.
pub fn compress(data: &[u8]) -> (u8, Vec<u8>) {
    let rle = rle_encode(data);
    if rle.len() < data.len() {
        (TAG_RLE, rle)
    } else {
        (TAG_STORE, data.to_vec())
    }
}

/// Decompress `(tag, bytes)` back to the original.
pub fn decompress(tag: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    match tag {
        TAG_STORE => Ok(data.to_vec()),
        TAG_RLE => rle_decode(data).ok_or_else(|| "corrupt RLE stream".into()),
        other => Err(format!("unknown compression tag {other}")),
    }
}

/// RLE format: literal runs as `[len<=127][bytes...]`, repeated runs
/// as `[0x80|count_hi][count_lo][byte]` for runs ≥ 3.
fn rle_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        // Measure run length at i.
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == data[i] && run < 0x8000 {
            run += 1;
        }
        if run >= 3 {
            // Repeat marker: 0x80 | high6, then low8, then byte.
            out.push(0x80 | ((run >> 8) as u8 & 0x7F));
            out.push((run & 0xFF) as u8);
            out.push(data[i]);
            i += run;
        } else {
            // Literal: gather until next run ≥3 or 127 bytes.
            let start = i;
            i += run; // skip the 1-2 equal bytes as literals
            while i < data.len() && out.len() < usize::MAX {
                let mut r = 1usize;
                while i + r < data.len() && data[i + r] == data[i] && r < 3 {
                    r += 1;
                }
                if r >= 3 || i - start >= 127 {
                    break;
                }
                i += 1;
            }
            let n = i - start;
            out.push(n as u8);
            out.extend_from_slice(&data[start..i]);
        }
    }
    out
}

fn rle_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let tag = data[i];
        i += 1;
        if tag & 0x80 != 0 {
            if i + 1 >= data.len() {
                return None;
            }
            let count = (((tag & 0x7F) as usize) << 8) | data[i] as usize;
            i += 1;
            let b = *data.get(i)?;
            i += 1;
            out.extend(std::iter::repeat_n(b, count));
        } else {
            let n = tag as usize;
            if i + n > data.len() {
                return None;
            }
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_roundtrip_runs() {
        let data = vec![0u8; 1000];
        let (tag, enc) = compress(&data);
        assert_eq!(tag, TAG_RLE);
        assert!(enc.len() < data.len() / 10);
        assert_eq!(decompress(tag, &enc).unwrap(), data);
    }

    #[test]
    fn random_data_stays_store() {
        // xorshift PRNG — deterministic "random" bytes.
        let mut x = 0x1234_5678u32;
        let data: Vec<u8> = (0..500)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x & 0xFF) as u8
            })
            .collect();
        let (tag, enc) = compress(&data);
        assert_eq!(tag, TAG_STORE);
        assert_eq!(decompress(tag, &enc).unwrap(), data);
    }

    #[test]
    fn mixed_content() {
        let mut data = vec![7u8; 50];
        data.extend(0u8..100);
        data.extend(vec![9u8; 200]);
        let (tag, enc) = compress(&data);
        assert_eq!(decompress(tag, &enc).unwrap(), data);
    }

    #[test]
    fn corrupt_rle_rejected() {
        // Literal tag claims 5 bytes but only 2 follow.
        assert!(rle_decode(&[5, 1, 2]).is_none());
        assert!(decompress(99, b"x").is_err());
    }
}
