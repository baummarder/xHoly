//! Bridge to the Scala/JVM runtime.
//!
//! The Scala side (`scala/src/main/scala/runtime/GameLoader.scala`) hosts
//! JVM-based game content and a Scala.js bundle for web deployment. This
//! module manages the subprocess/JNI-style bridge: it locates the Scala
//! runtime jar, launches it, and exchanges JSON messages over stdio.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// A message exchanged with the Scala runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BridgeMessage {
    /// Request the Scala runtime to load a game module.
    LoadGame { title_id: String },
    /// Scala runtime reports progress.
    Progress { percent: f32, message: String },
    /// Scala runtime reports an error.
    Error { message: String },
    /// Graceful shutdown.
    Shutdown,
}

/// Handle to a running Scala runtime subprocess.
pub struct ScalaBridge {
    child: Child,
    /// Path to the Scala runtime jar/bundle.
    pub runtime_path: PathBuf,
}

impl ScalaBridge {
    /// Locate the Scala runtime artifact (`xholy-runtime.jar` or the
    /// Scala.js bundle under `target/`).
    pub fn find_runtime() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from("target/scala-3.3.4/xholy-runtime.jar"),
            PathBuf::from("scala/target/scala-3.3.4/xholy-runtime.jar"),
            PathBuf::from("web/target/scala-3.3.4/xholy-web-opt.js"),
        ];
        candidates.into_iter().find(|p| p.exists())
    }

    /// Spawn the Scala runtime for `title_id`.
    pub fn spawn(title_id: &str) -> std::io::Result<Self> {
        let runtime_path = Self::find_runtime().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Scala runtime not found; run `sbt assembly` first",
            )
        })?;
        let child = Command::new("java")
            .arg("-jar")
            .arg(&runtime_path)
            .arg(title_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self {
            child,
            runtime_path,
        })
    }

    /// Send a message to the Scala runtime (newline-delimited JSON).
    pub fn send(&mut self, msg: &BridgeMessage) -> std::io::Result<()> {
        use std::io::Write;
        let stdin =
            self.child.stdin.as_mut().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin closed")
            })?;
        let line = serde_json::to_string(msg).unwrap_or_default();
        writeln!(stdin, "{}", line)
    }

    /// Shut down the runtime gracefully.
    pub fn shutdown(mut self) {
        let _ = self.send(&BridgeMessage::Shutdown);
        let _ = self.child.wait();
    }
}

impl Drop for ScalaBridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
