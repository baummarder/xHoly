//! General utilities — hashing, strings, time, files.

use std::time::{SystemTime, UNIX_EPOCH};

/// FNV-1a 64-bit hash.
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Current unix timestamp secs.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current unix timestamp millis.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Human-readable byte count.
pub fn human_bytes(b: u64) -> String {
    HumanBytes(b).to_string()
}

/// Wrapper for Display impl.
pub struct HumanBytes(pub u64);

impl std::fmt::Display for HumanBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut v = self.0 as f64;
        let mut i = 0;
        while v >= 1024.0 && i < UNITS.len() - 1 {
            v /= 1024.0;
            i += 1;
        }
        if i == 0 {
            write!(f, "{} {}", self.0, UNITS[0])
        } else {
            write!(f, "{:.1} {}", v, UNITS[i])
        }
    }
}

/// Sanitize a string for use as filename (ASCII-safe).
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Slugify: lowercase, non-alnum → '-', collapse repeats.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true; // trim leading
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Read a file to string, trimming trailing whitespace.
pub fn read_text(path: &std::path::Path) -> std::io::Result<String> {
    Ok(std::fs::read_to_string(path)?.trim_end().to_string())
}

/// Ensure parent dirs exist then write file.
pub fn write_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, data)
}

/// Format duration millis → "1.23 s" / "456 ms".
pub fn human_duration_ms(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_deterministic() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"hello"), fnv1a(b"hello"));
        assert_ne!(fnv1a(b"a"), fnv1a(b"b"));
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn sanitize_and_slug() {
        assert_eq!(sanitize_filename("a/b\\c:d*e?f"), "a_b_c_d_e_f");
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("  --Test--  "), "test");
    }

    #[test]
    fn duration_format() {
        assert_eq!(human_duration_ms(500), "500 ms");
        assert_eq!(human_duration_ms(2500), "2.50 s");
    }

    #[test]
    fn write_creates_parents() {
        let dir = std::env::temp_dir().join("xholy_utils_test/nested");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("xholy_utils_test"));
        write_file(&dir.join("f.txt"), b"data").unwrap();
        assert_eq!(read_text(&dir.join("f.txt")).unwrap(), "data");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("xholy_utils_test"));
    }
}
