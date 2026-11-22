//! Debug tools — memory dump + shader inspection helpers.

/// Dump errors.
#[derive(Debug)]
pub enum DumpError {
    /// Address out of range.
    OutOfRange { addr: u64, size: usize, limit: u64 },
}

impl std::fmt::Display for DumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange { addr, size, limit } => {
                write!(f, "dump {addr:#x}+{size} exceeds limit {limit:#x}")
            }
        }
    }
}
impl std::error::Error for DumpError {}

/// Hex-dump `data` starting at `base_addr` into lines of `width` bytes.
pub fn dump_region(base_addr: u64, data: &[u8], width: usize) -> String {
    let width = width.max(1);
    let mut out = String::new();
    for (i, chunk) in data.chunks(width).enumerate() {
        let addr = base_addr + (i * width) as u64;
        out.push_str(&format!("{addr:016x}  "));
        for b in chunk {
            out.push_str(&format!("{b:02x} "));
        }
        out.push_str(" |");
        for &b in chunk {
            let c = if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            };
            out.push(c);
        }
        out.push_str("|\n");
    }
    out
}

/// Validated dump: checks bounds then formats.
pub fn dump_checked(
    base_addr: u64,
    data: &[u8],
    max_addr: u64,
    width: usize,
) -> Result<String, DumpError> {
    let end = base_addr.saturating_add(data.len() as u64);
    if end > max_addr {
        return Err(DumpError::OutOfRange {
            addr: base_addr,
            size: data.len(),
            limit: max_addr,
        });
    }
    Ok(dump_region(base_addr, data, width))
}

/// Shader inspection info.
#[derive(Debug, Clone, Default)]
pub struct ShaderInfo {
    /// Entry point name.
    pub entry_point: String,
    /// Stage (vs/fs/cs).
    pub stage: String,
    /// Resource count.
    pub resources: usize,
    /// Source size in bytes.
    pub source_len: usize,
    /// Cache hit?
    pub cache_hit: bool,
}

/// Inspect a shader source string (lightweight: counts bindings).
pub fn inspect_shader(entry: &str, stage: &str, source: &str) -> ShaderInfo {
    let resources = source.matches("@binding").count() + source.matches("register(").count();
    ShaderInfo {
        entry_point: entry.to_string(),
        stage: stage.to_string(),
        resources,
        source_len: source.len(),
        cache_hit: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_format() {
        let d = dump_region(0x1000, &[0x48, 0x65, 0x6c, 0x6c, 0x6f], 16);
        assert!(d.contains("0000000000001000"));
        assert!(d.contains("Hello"));
        assert!(d.contains("48 65 6c 6c 6f"));
    }

    #[test]
    fn dump_checked_bounds() {
        assert!(dump_checked(0x1000, &[0; 10], 0x2000, 16).is_ok());
        assert!(dump_checked(0x1F00, &[0; 0x200], 0x2000, 16).is_err());
    }

    #[test]
    fn inspect_counts_bindings() {
        let src = "@binding(0) var<uniform> a: f32; @binding(1) var t: texture_2d<f32>;";
        let info = inspect_shader("main", "fs", src);
        assert_eq!(info.resources, 2);
        assert_eq!(info.entry_point, "main");
    }
}
