//! Telemetry events — anonymized, serde-friendly.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Event categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// App started.
    SessionStart,
    /// App exited cleanly.
    SessionEnd,
    /// Game launched.
    GameLaunch,
    /// Game exited.
    GameExit,
    /// Crash/panic reported.
    Crash,
    /// Performance sample (fps bucket).
    PerfSample,
    /// Feature used.
    FeatureUse,
    /// Custom.
    Custom,
}

/// One telemetry event (no PII fields by design).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    /// Kind.
    pub kind: EventKind,
    /// Unix timestamp secs.
    pub ts: u64,
    /// App version string.
    pub app_version: String,
    /// Platform tag ("windows"/"linux"/"macos"/"wasm").
    pub platform: String,
    /// Key/value attributes (caller must keep them anonymous).
    #[serde(default)]
    pub attrs: Vec<(String, String)>,
}

impl TelemetryEvent {
    /// New event of `kind`.
    pub fn new(kind: EventKind, app_version: &str, platform: &str) -> Self {
        Self {
            kind,
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            app_version: app_version.to_string(),
            platform: platform.to_string(),
            attrs: Vec::new(),
        }
    }
    /// Add attribute.
    pub fn attr(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attrs.push((k.into(), v.into()));
        self
    }
}

/// The collector: filter → batch → sender.
pub struct TelemetryCollector<F, S> {
    filter: F,
    sender: S,
    /// Pending batch.
    pending: Vec<TelemetryEvent>,
    /// Flush when pending reaches this size.
    pub batch_size: usize,
    /// Total events dropped by filter.
    pub dropped: u64,
    /// Total events sent.
    pub sent: u64,
}

impl<F, S> TelemetryCollector<F, S>
where
    F: Fn(&TelemetryEvent) -> bool,
    S: super::telemetry_sender::Sender,
{
    /// New collector.
    pub fn new(filter: F, sender: S) -> Self {
        Self {
            filter,
            sender,
            pending: Vec::new(),
            batch_size: 32,
            dropped: 0,
            sent: 0,
        }
    }

    /// Record an event (filtered + batched).
    pub fn record(&mut self, ev: TelemetryEvent) {
        if !(self.filter)(&ev) {
            self.dropped += 1;
            return;
        }
        self.pending.push(ev);
        if self.pending.len() >= self.batch_size {
            self.flush();
        }
    }

    /// Force flush pending batch.
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let n = batch.len() as u64;
        match self.sender.send(&batch) {
            Ok(()) => self.sent += n,
            Err(_) => {
                // Requeue on failure (bounded: drop oldest if huge).
                self.pending = batch;
                if self.pending.len() > self.batch_size * 4 {
                    let drain = self.pending.len() - self.batch_size * 4;
                    self.pending.drain(..drain);
                }
            }
        }
    }

    /// Pending count.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::telemetry_sender::MemorySender;

    #[test]
    fn event_attrs() {
        let e = TelemetryEvent::new(EventKind::GameLaunch, "0.1.0", "windows")
            .attr("title_id", "ABCD")
            .attr("fps_bucket", "55-60");
        assert_eq!(e.kind, EventKind::GameLaunch);
        assert_eq!(e.attrs.len(), 2);
        // serde roundtrip
        let s = serde_json::to_string(&e).unwrap();
        let back: TelemetryEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back.app_version, "0.1.0");
    }

    #[test]
    fn collector_batches() {
        let sender = MemorySender::new();
        let mut c = TelemetryCollector::new(|_| true, sender.clone());
        c.batch_size = 3;
        for _ in 0..3 {
            c.record(TelemetryEvent::new(EventKind::FeatureUse, "v", "p"));
        }
        // batch_size reached → flushed
        assert_eq!(c.sent, 3);
        assert_eq!(c.pending(), 0);
        let batches = sender.batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 3);
    }

    #[test]
    fn filter_drops() {
        let sender = MemorySender::new();
        let mut c = TelemetryCollector::new(|e| e.kind == EventKind::Crash, sender);
        c.record(TelemetryEvent::new(EventKind::FeatureUse, "v", "p"));
        c.record(TelemetryEvent::new(EventKind::Crash, "v", "p"));
        c.flush();
        assert_eq!(c.dropped, 1);
        assert_eq!(c.sent, 1);
    }
}
