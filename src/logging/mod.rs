//! Central logging — Phase 11.5.
//!
//! Levelled logger with pluggable sinks (console/file/JSON), module
//! filters, and timing spans. Sits alongside `log`/`env_logger` (which
//! stays for third-party crates); this is the xHoly-internal channel.

pub mod filters;
pub mod rotating;
pub mod sinks;
pub mod span;

pub use filters::{Filter, LevelFilter};
pub use rotating::RotatingFile;
pub use sinks::{ConsoleSink, FileSink, JsonlSink, Sink};
pub use span::Span;

use std::sync::{Arc, Mutex};

/// Log severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
}

impl Level {
    /// Parse from name (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            "fatal" => Some(Self::Fatal),
            _ => None,
        }
    }
    /// Short tag for output.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
        }
    }
    /// ANSI color for console output.
    pub fn color(self) -> &'static str {
        match self {
            Self::Trace => "\x1b[90m",
            Self::Debug => "\x1b[36m",
            Self::Info => "\x1b[32m",
            Self::Warn => "\x1b[33m",
            Self::Error => "\x1b[31m",
            Self::Fatal => "\x1b[41;97m",
        }
    }
}

/// One log record.
#[derive(Debug, Clone)]
pub struct Record {
    pub level: Level,
    /// Module path (e.g. `core::kernel`).
    pub module: String,
    pub message: String,
    /// Optional span context.
    pub span: Option<String>,
}

/// The xHoly logger: level floor + sinks + filters.
pub struct Logger {
    min_level: Level,
    sinks: Vec<Arc<Mutex<dyn Sink>>>,
    filters: Vec<Filter>,
}

impl Logger {
    /// New logger writing Info+ to stderr.
    pub fn new() -> Self {
        Self {
            min_level: Level::Info,
            sinks: vec![Arc::new(Mutex::new(ConsoleSink::stderr()))],
            filters: Vec::new(),
        }
    }

    /// Builder-style level floor.
    pub fn with_min_level(mut self, l: Level) -> Self {
        self.min_level = l;
        self
    }

    /// Add a sink.
    pub fn add_sink(mut self, sink: Arc<Mutex<dyn Sink>>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Add a filter (all filters are OR-ed for *allowing*? No — a
    /// record passes if it passes the level floor AND no filter
    /// rejects it).
    pub fn add_filter(mut self, f: Filter) -> Self {
        self.filters.push(f);
        self
    }

    /// Emit a record if it passes level + filters.
    pub fn log(&self, level: Level, module: &str, msg: impl Into<String>) {
        if level < self.min_level {
            return;
        }
        let rec = Record {
            level,
            module: module.to_string(),
            message: msg.into(),
            span: None,
        };
        if self.filters.iter().any(|f| !f.accepts(&rec)) {
            return;
        }
        for s in &self.sinks {
            if let Ok(mut s) = s.lock() {
                s.write(&rec);
            }
        }
    }

    /// Convenience macros' backing methods.
    pub fn trace(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Trace, m, msg)
    }
    pub fn debug(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Debug, m, msg)
    }
    pub fn info(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Info, m, msg)
    }
    pub fn warn(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Warn, m, msg)
    }
    pub fn error(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Error, m, msg)
    }
    pub fn fatal(&self, m: &str, msg: impl Into<String>) {
        self.log(Level::Fatal, m, msg)
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-global logger (lazy, stderr console by default).
pub fn global() -> &'static Logger {
    static L: std::sync::OnceLock<Logger> = std::sync::OnceLock::new();
    L.get_or_init(Logger::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering_and_parse() {
        assert!(Level::Trace < Level::Info);
        assert!(Level::Info < Level::Fatal);
        assert_eq!(Level::parse("WARN"), Some(Level::Warn));
        assert_eq!(Level::parse("nope"), None);
    }

    #[test]
    fn level_floor_filters() {
        let l = Logger::new().with_min_level(Level::Warn);
        // No panic; records below floor are dropped before sinks.
        l.info("m", "hidden");
        l.error("m", "shown");
    }
}
