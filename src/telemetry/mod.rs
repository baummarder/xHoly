//! Telemetry — Phase 11.5.
//!
//! Anonymized, filtered, batched telemetry. Events carry no PII:
//! caller supplies a `TelemetryFilter` predicate and the collector
//! drops everything that doesn't pass. Batches flush on size or
//! interval via an injectable sender.

#[allow(clippy::module_inception)]
pub mod telemetry;
pub mod telemetry_filter;
pub mod telemetry_sender;

pub use telemetry::{EventKind, TelemetryCollector, TelemetryEvent};
pub use telemetry_filter::{FilterRule, TelemetryFilter};
pub use telemetry_sender::{BatchSender, MemorySender, Sender};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::EventKind::SessionStart;
    }
}
