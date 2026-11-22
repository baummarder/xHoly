//! Telemetry sender — batched delivery abstraction.

use super::telemetry::TelemetryEvent;

/// A sender backend.
pub trait Sender {
    /// Send a batch; return Err to requeue.
    fn send(&mut self, batch: &[TelemetryEvent]) -> Result<(), String>;
}

/// In-memory sender for tests: stores batches (shared across clones
/// via Arc so a collector and its inspector see the same data).
#[derive(Debug, Default, Clone)]
pub struct MemorySender {
    batches: std::sync::Arc<std::sync::Mutex<Vec<Vec<TelemetryEvent>>>>,
    /// If >0, fail this many sends.
    pub fail_next: usize,
}

impl MemorySender {
    /// New sender.
    pub fn new() -> Self {
        Self::default()
    }
    /// All batches received.
    pub fn batches(&self) -> Vec<Vec<TelemetryEvent>> {
        self.batches.lock().unwrap().clone()
    }
    /// Total events sent.
    pub fn total_events(&self) -> usize {
        self.batches.lock().unwrap().iter().map(|b| b.len()).sum()
    }
}

impl Sender for MemorySender {
    fn send(&mut self, batch: &[TelemetryEvent]) -> Result<(), String> {
        if self.fail_next > 0 {
            self.fail_next -= 1;
            return Err("injected failure".into());
        }
        self.batches.lock().unwrap().push(batch.to_vec());
        Ok(())
    }
}

/// Batch sender wrapper: accumulates and flushes on interval/size.
/// Delegates actual IO to an inner `Sender` (e.g. HTTP transport).
pub struct BatchSender<S: Sender> {
    inner: S,
    /// Pending events.
    pending: Vec<TelemetryEvent>,
    /// Flush threshold.
    pub max_batch: usize,
    /// Max pending before forced drop-oldest.
    pub max_pending: usize,
}

impl<S: Sender> BatchSender<S> {
    /// New batch sender.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            max_batch: 32,
            max_pending: 1024,
        }
    }

    /// Queue an event; flushes when batch full.
    pub fn push(&mut self, ev: TelemetryEvent) {
        self.pending.push(ev);
        if self.pending.len() >= self.max_pending {
            // Drop oldest to bound memory.
            let drain = self.pending.len() - self.max_pending / 2;
            self.pending.drain(..drain);
        }
        if self.pending.len() >= self.max_batch {
            self.flush();
        }
    }

    /// Flush pending via inner sender; on error requeue.
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        if self.inner.send(&batch).is_err() {
            self.pending = batch;
        }
    }

    /// Pending count.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Inner sender access.
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::telemetry::{EventKind, TelemetryEvent};

    #[test]
    fn batch_sender_flushes_on_size() {
        let mem = MemorySender::new();
        let mut b = BatchSender::new(mem);
        b.max_batch = 2;
        for _ in 0..2 {
            b.push(TelemetryEvent::new(EventKind::FeatureUse, "v", "p"));
        }
        assert_eq!(b.pending(), 0);
        assert_eq!(b.inner().batches().len(), 1);
    }

    #[test]
    fn failed_send_requeues() {
        let mut mem = MemorySender::new();
        mem.fail_next = 1;
        let mut b = BatchSender::new(mem);
        b.max_batch = 1;
        b.push(TelemetryEvent::new(EventKind::Crash, "v", "p"));
        // flush attempted, failed → pending has 1
        assert_eq!(b.pending(), 1);
    }
}
