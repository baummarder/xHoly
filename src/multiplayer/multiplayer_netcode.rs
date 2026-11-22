//! Netcode stub — interpolation + prediction concepts.
//!
//! Models the *math* of client-side prediction and entity
//! interpolation without any transport. Useful for testing game
//! netcode logic deterministically.

use std::collections::VecDeque;

/// Netcode tuning.
#[derive(Debug, Clone)]
pub struct NetcodeConfig {
    /// Snapshot interpolation delay (ticks).
    pub interp_delay_ticks: u64,
    /// Max buffered snapshots per entity.
    pub max_snapshots: usize,
    /// Reconciliation threshold (units) before hard snap.
    pub reconcile_threshold: f32,
}

impl Default for NetcodeConfig {
    fn default() -> Self {
        Self {
            interp_delay_ticks: 2,
            max_snapshots: 32,
            reconcile_threshold: 0.5,
        }
    }
}

/// One received snapshot for an entity.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub tick: u64,
    pub x: f32,
    pub y: f32,
}

/// Interpolator: renders entity state `interp_delay` ticks in the
/// past by lerping between surrounding snapshots.
#[derive(Debug)]
pub struct Interpolator {
    /// entity_id → snapshot ring.
    buffers: std::collections::HashMap<u64, VecDeque<Snapshot>>,
    cfg: NetcodeConfig,
}

impl Interpolator {
    /// New interpolator.
    pub fn new(cfg: NetcodeConfig) -> Self {
        Self {
            buffers: std::collections::HashMap::new(),
            cfg,
        }
    }

    /// Push a snapshot for an entity.
    pub fn push(&mut self, entity: u64, snap: Snapshot) {
        let buf = self.buffers.entry(entity).or_default();
        buf.push_back(snap);
        while buf.len() > self.cfg.max_snapshots {
            buf.pop_front();
        }
    }

    /// Interpolated position at render `tick` (typically
    /// authoritative_tick - interp_delay).
    pub fn sample(&self, entity: u64, render_tick: u64) -> Option<(f32, f32)> {
        let buf = self.buffers.get(&entity)?;
        if buf.is_empty() {
            return None;
        }
        if buf.len() == 1 {
            let s = buf[0];
            return Some((s.x, s.y));
        }
        // Find surrounding snapshots (VecDeque has no windows()).
        let snaps: Vec<Snapshot> = buf.iter().copied().collect();
        let mut prev = snaps[0];
        let mut next = *snaps.last().unwrap();
        for w in snaps.windows(2) {
            if w[0].tick <= render_tick && render_tick <= w[1].tick {
                prev = w[0];
                next = w[1];
                break;
            }
        }
        if next.tick == prev.tick {
            return Some((prev.x, prev.y));
        }
        let t = (render_tick.saturating_sub(prev.tick)) as f32 / (next.tick - prev.tick) as f32;
        Some((
            prev.x + (next.x - prev.x) * t,
            prev.y + (next.y - prev.y) * t,
        ))
    }

    /// Buffered count for entity.
    pub fn buffered(&self, entity: u64) -> usize {
        self.buffers.get(&entity).map(|b| b.len()).unwrap_or(0)
    }
}

/// Client-side predictor: applies inputs locally, reconciles against
/// authoritative snapshots.
#[derive(Debug)]
pub struct Predictor {
    /// Unacked inputs (input → predicted deltas).
    pending: VecDeque<(u64, f32, f32)>, // (tick, dx, dy)
    /// Current predicted position.
    pub pos: (f32, f32),
    /// Last authoritative pos+tick.
    pub last_authoritative: Option<(u64, f32, f32)>,
    cfg: NetcodeConfig,
}

impl Predictor {
    /// New predictor at origin.
    pub fn new(cfg: NetcodeConfig) -> Self {
        Self {
            pending: VecDeque::new(),
            pos: (0.0, 0.0),
            last_authoritative: None,
            cfg,
        }
    }

    /// Apply a local input (predicts movement).
    pub fn apply_input(&mut self, tick: u64, dx: f32, dy: f32) {
        self.pos.0 += dx;
        self.pos.1 += dy;
        self.pending.push_back((tick, dx, dy));
    }

    /// Reconcile with an authoritative snapshot: rewind pending
    /// inputs and re-apply, or hard-snap if error too large.
    pub fn reconcile(&mut self, auth_tick: u64, auth_pos: (f32, f32)) -> bool {
        let err = ((self.pos.0 - auth_pos.0).powi(2) + (self.pos.1 - auth_pos.1).powi(2)).sqrt();
        if err > self.cfg.reconcile_threshold {
            // Hard snap: drop pending, trust server.
            self.pos = auth_pos;
            self.pending.clear();
            self.last_authoritative = Some((auth_tick, auth_pos.0, auth_pos.1));
            return false; // snapped
        }
        // Soft reconcile: rewind unacked inputs from auth pos.
        let mut pos = auth_pos;
        let mut kept = VecDeque::new();
        for (t, dx, dy) in self.pending.drain(..) {
            if t > auth_tick {
                pos.0 += dx;
                pos.1 += dy;
                kept.push_back((t, dx, dy));
            }
        }
        self.pending = kept;
        self.pos = pos;
        self.last_authoritative = Some((auth_tick, auth_pos.0, auth_pos.1));
        true
    }

    /// Pending input count.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_between_snapshots() {
        let mut it = Interpolator::new(NetcodeConfig::default());
        it.push(
            1,
            Snapshot {
                tick: 0,
                x: 0.0,
                y: 0.0,
            },
        );
        it.push(
            1,
            Snapshot {
                tick: 10,
                x: 10.0,
                y: 0.0,
            },
        );
        let (x, _) = it.sample(1, 5).unwrap();
        assert!((x - 5.0).abs() < 0.01);
    }

    #[test]
    fn prediction_reconciles_small_error() {
        let cfg = NetcodeConfig {
            reconcile_threshold: 1.0,
            ..Default::default()
        };
        let mut p = Predictor::new(cfg);
        p.apply_input(1, 1.0, 0.0);
        p.apply_input(2, 1.0, 0.0);
        // pos = (2,0). Server says (1.9, 0) at tick 2 → err .1 < 1 → soft.
        let soft = p.reconcile(2, (1.9, 0.0));
        assert!(soft);
        // pending inputs after tick 2 replayed → pos ≈ 1.9 + nothing? tick>2 none → pos=1.9
        assert!((p.pos.0 - 1.9).abs() < 0.01);
    }

    #[test]
    fn large_error_hard_snaps() {
        let cfg = NetcodeConfig {
            reconcile_threshold: 0.1,
            ..Default::default()
        };
        let mut p = Predictor::new(cfg);
        p.apply_input(1, 5.0, 0.0);
        let soft = p.reconcile(1, (0.0, 0.0));
        assert!(!soft);
        assert_eq!(p.pos, (0.0, 0.0));
        assert_eq!(p.pending(), 0);
    }
}
