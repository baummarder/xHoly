//! Metric statistics — FPS-style ring buffers with percentiles.

/// Fixed-capacity ring of f64 samples.
#[derive(Debug, Clone)]
pub struct StatsRing {
    buf: Vec<f64>,
    cap: usize,
    pos: usize,
    len: usize,
}

impl StatsRing {
    /// New ring with capacity.
    pub fn new(cap: usize) -> Self {
        Self {
            buf: vec![0.0; cap.max(1)],
            cap: cap.max(1),
            pos: 0,
            len: 0,
        }
    }

    /// Push a sample.
    pub fn push(&mut self, v: f64) {
        self.buf[self.pos] = v;
        self.pos = (self.pos + 1) % self.cap;
        self.len = (self.len + 1).min(self.cap);
    }

    /// Sorted copy of samples.
    fn sorted(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.buf[..self.len].to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    /// Count.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Mean.
    pub fn avg(&self) -> f64 {
        if self.len == 0 {
            return 0.0;
        }
        self.buf[..self.len].iter().sum::<f64>() / self.len as f64
    }

    /// Min.
    pub fn min(&self) -> f64 {
        self.sorted().first().copied().unwrap_or(0.0)
    }

    /// Max.
    pub fn max(&self) -> f64 {
        self.sorted().last().copied().unwrap_or(0.0)
    }

    /// Percentile (0..=100).
    pub fn percentile(&self, p: f64) -> f64 {
        let s = self.sorted();
        if s.is_empty() {
            return 0.0;
        }
        let idx = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
        s[idx.min(s.len() - 1)]
    }

    /// P50/P95/P99 convenience.
    pub fn p50(&self) -> f64 {
        self.percentile(50.0)
    }
    pub fn p95(&self) -> f64 {
        self.percentile(95.0)
    }
    pub fn p99(&self) -> f64 {
        self.percentile(99.0)
    }
}

/// Aggregated stats snapshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricStats {
    pub count: usize,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

impl MetricStats {
    /// Compute from a ring.
    pub fn from_ring(r: &StatsRing) -> Self {
        Self {
            count: r.len(),
            avg: r.avg(),
            min: r.min(),
            max: r.max(),
            p50: r.p50(),
            p95: r.p95(),
            p99: r.p99(),
        }
    }
}

/// FPS-specific ring (frame times → fps).
#[derive(Debug, Clone)]
pub struct FrameStats {
    /// Frame-time ring (ms).
    pub frame_ms: StatsRing,
}

impl FrameStats {
    /// New with capacity (e.g. 240).
    pub fn new(cap: usize) -> Self {
        Self {
            frame_ms: StatsRing::new(cap),
        }
    }

    /// Record a frame duration.
    pub fn push_frame(&mut self, dt_ms: f64) {
        self.frame_ms.push(dt_ms);
    }

    /// Current FPS estimate (1 / avg frame time).
    pub fn fps(&self) -> f64 {
        let avg = self.frame_ms.avg();
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }

    /// FPS variance proxy (stdev of frame times → fps jitter).
    pub fn fps_jitter(&self) -> f64 {
        let s = self.frame_ms.sorted();
        if s.len() < 2 {
            return 0.0;
        }
        let mean = s.iter().sum::<f64>() / s.len() as f64;
        let var = s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / s.len() as f64;
        var.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_stats() {
        let mut r = StatsRing::new(100);
        for i in 1..=100 {
            r.push(i as f64);
        }
        assert_eq!(r.len(), 100);
        assert_eq!(r.min(), 1.0);
        assert_eq!(r.max(), 100.0);
        assert!((r.avg() - 50.5).abs() < 0.01);
        assert!(r.p50() >= 50.0 && r.p50() <= 51.0);
        assert!(r.p95() >= 90.0);
        assert!(r.p99() >= 98.0);
    }

    #[test]
    fn frame_stats_fps() {
        let mut f = FrameStats::new(60);
        for _ in 0..60 {
            f.push_frame(16.67);
        }
        let fps = f.fps();
        assert!((fps - 60.0).abs() < 1.0);
    }

    #[test]
    fn ring_wraps() {
        let mut r = StatsRing::new(4);
        for i in 0..10 {
            r.push(i as f64);
        }
        assert_eq!(r.len(), 4);
        assert_eq!(r.min(), 6.0);
        assert_eq!(r.max(), 9.0);
    }
}
