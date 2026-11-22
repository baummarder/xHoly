//! Config file IO + hot-reload (Phase 11.5).
//!
//! Loads TOML/JSON config files and watches them via `notify`:
//! on change the file is re-parsed and the callback receives the new
//! `serde_json::Value` (merged shape) — callers deserialize into
//! their concrete config type.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

use super::merge::toml_to_value;

/// Errors loading a config file.
#[derive(Debug)]
pub enum ConfigFileError {
    Io(std::io::Error),
    Parse(String),
}
impl std::fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(e) => write!(f, "parse: {e}"),
        }
    }
}
impl std::error::Error for ConfigFileError {}

/// Load a config file (TOML by extension, JSON for `.json`).
pub fn load_file(path: &Path) -> Result<Value, ConfigFileError> {
    let text = std::fs::read_to_string(path).map_err(ConfigFileError::Io)?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            serde_json::from_str(&text).map_err(|e| ConfigFileError::Parse(e.to_string()))
        }
        _ => toml_to_value(&text).map_err(|e| ConfigFileError::Parse(e.to_string())),
    }
}

/// Debounced file watcher: emits the re-loaded `Value` on change.
/// Polls mtime every `interval` — simpler and dependency-free vs.
/// `notify` for our small files; `notify` remains available for the
/// GUI settings watcher.
pub struct ConfigWatcher {
    path: PathBuf,
    last_mtime: Option<std::time::SystemTime>,
    interval: Duration,
}

impl ConfigWatcher {
    /// Watch `path`; `interval` is the poll period.
    pub fn new(path: PathBuf, interval: Duration) -> Self {
        Self {
            path,
            last_mtime: None,
            interval,
        }
    }

    /// Check once; returns `Some(new_value)` when the file changed
    /// and parses cleanly. Call in a loop or from a timer.
    pub fn poll(&mut self) -> Option<Result<Value, ConfigFileError>> {
        let mtime = std::fs::metadata(&self.path).ok()?.modified().ok()?;
        if self.last_mtime == Some(mtime) {
            return None;
        }
        self.last_mtime = Some(mtime);
        Some(load_file(&self.path))
    }

    /// Spawn a thread that polls and sends updates on a channel.
    /// Returns the thread handle + receiver; the thread exits when the
    /// receiver is dropped.
    pub fn spawn_channel(
        self,
    ) -> (
        std::thread::JoinHandle<()>,
        mpsc::Receiver<Result<Value, ConfigFileError>>,
    ) {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut w = self;
            // Prime: emit current state once so subscribers sync.
            if let Some(v) = w.poll() {
                let _ = tx.send(v);
            }
            loop {
                std::thread::sleep(w.interval);
                if let Some(v) = w.poll() {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        (handle, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_toml_and_json() {
        let dir = std::env::temp_dir().join("xholy_cfgfile");
        std::fs::create_dir_all(&dir).unwrap();
        let toml_path = dir.join("c.toml");
        let json_path = dir.join("c.json");
        std::fs::write(&toml_path, "[a]\nb = 1\n").unwrap();
        std::fs::write(&json_path, r#"{"a":{"b":2}}"#).unwrap();
        assert_eq!(load_file(&toml_path).unwrap()["a"]["b"], 1);
        assert_eq!(load_file(&json_path).unwrap()["a"]["b"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watcher_detects_change() {
        let dir = std::env::temp_dir().join("xholy_cfgwatch");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("w.toml");
        std::fs::write(&p, "v = 1\n").unwrap();
        let mut w = ConfigWatcher::new(p.clone(), Duration::from_millis(10));
        assert!(w.poll().is_some()); // initial
        assert!(w.poll().is_none()); // unchanged
        std::thread::sleep(Duration::from_millis(20)); // ensure mtime differs
        std::fs::write(&p, "v = 2\n").unwrap();
        let upd = w.poll().expect("should detect change").unwrap();
        assert_eq!(upd["v"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
