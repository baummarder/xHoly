//! Log sinks — where records go.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

use super::{Level, Record};

/// A log destination.
pub trait Sink: Send {
    /// Write one record (implementations handle their own locking
    /// only if shared; `Logger` already serializes per-sink).
    fn write(&mut self, rec: &Record);
    /// Optional flush (file sinks).
    fn flush(&mut self) {}
}

/// stderr/stdout console sink with ANSI colors.
pub struct ConsoleSink {
    use_color: bool,
    to_stderr: bool,
}

impl ConsoleSink {
    /// Colored stderr sink.
    pub fn stderr() -> Self {
        Self {
            use_color: true,
            to_stderr: true,
        }
    }
    /// Plain stdout sink (tests/CI).
    pub fn stdout_plain() -> Self {
        Self {
            use_color: false,
            to_stderr: false,
        }
    }
}

impl Sink for ConsoleSink {
    fn write(&mut self, rec: &Record) {
        let line = if self.use_color {
            format!(
                "{}{}[{}] {}:{}\x1b[0m",
                rec.level.color(),
                if rec.level >= Level::Warn {
                    "\x1b[1m"
                } else {
                    ""
                },
                rec.level.tag(),
                rec.module,
                rec.message
            )
        } else {
            format!("[{}] {}: {}", rec.level.tag(), rec.module, rec.message)
        };
        if self.to_stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
}

/// Append-to-file sink (one line per record, plain text).
pub struct FileSink {
    file: Mutex<std::fs::File>,
}

impl FileSink {
    /// Open (create+append) `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Sink for FileSink {
    fn write(&mut self, rec: &Record) {
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "[{}] {}: {}", rec.level.tag(), rec.module, rec.message);
        }
    }
    fn flush(&mut self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// JSON-lines sink — one JSON object per record (for log shippers).
pub struct JsonlSink {
    file: Mutex<std::fs::File>,
}

impl JsonlSink {
    /// Open (create+append) `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Sink for JsonlSink {
    fn write(&mut self, rec: &Record) {
        let obj = serde_json::json!({
            "level": rec.level.tag(),
            "module": rec.module,
            "message": rec.message,
            "span": rec.span,
            "ts": chrono::Utc::now().to_rfc3339(),
        });
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{obj}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_sink_writes_lines() {
        let dir = std::env::temp_dir().join("xholy_logsink");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("l.log");
        let mut s = FileSink::open(&p).unwrap();
        s.write(&Record {
            level: Level::Info,
            module: "m".into(),
            message: "hello".into(),
            span: None,
        });
        s.flush();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("INFO") && text.contains("hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jsonl_sink_emits_json() {
        let dir = std::env::temp_dir().join("xholy_jsonlsink");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("l.jsonl");
        let mut s = JsonlSink::open(&p).unwrap();
        s.write(&Record {
            level: Level::Error,
            module: "core".into(),
            message: "boom".into(),
            span: None,
        });
        s.flush();
        let text = std::fs::read_to_string(&p).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["level"], "ERROR");
        assert_eq!(v["message"], "boom");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
