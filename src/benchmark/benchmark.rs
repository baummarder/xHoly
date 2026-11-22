//! Benchmark runner — CPU/GPU/memory micro-benchmarks.

use std::time::{Duration, Instant};

use super::metrics::MetricStats;

/// What to benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkKind {
    /// Pure CPU loop (hash/sort workload).
    Cpu,
    /// Memory bandwidth (alloc + memset).
    Memory,
    /// "GPU" — wgpu pipeline creation+render timing (best effort).
    Gpu,
}

/// Result of one benchmark run.
#[derive(Debug, Clone)]
pub struct BenchResult {
    pub kind: BenchmarkKind,
    /// Iterations completed.
    pub iters: u64,
    /// Wall time.
    pub elapsed: Duration,
    /// Throughput metric (kind-specific; e.g. MB/s for memory).
    pub metric: f64,
    /// Per-iteration stats (min/avg/max ms).
    pub stats: MetricStats,
}

/// The runner.
pub struct BenchmarkRunner {
    /// Minimum run duration (keeps short benches meaningful).
    pub min_duration: Duration,
    /// Max iterations cap.
    pub max_iters: u64,
}

impl Default for BenchmarkRunner {
    fn default() -> Self {
        Self {
            min_duration: Duration::from_millis(500),
            max_iters: 1_000_000,
        }
    }
}

impl BenchmarkRunner {
    /// New runner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run a CPU benchmark: FNV-1a over a buffer, `iters` times.
    pub fn cpu(&self, buffer: &[u8], iters: u64) -> BenchResult {
        let iters = iters.min(self.max_iters).max(1);
        let mut times = Vec::with_capacity(iters as usize);
        let start = Instant::now();
        let mut acc = 0u64;
        for _ in 0..iters {
            let t = Instant::now();
            // Workload: hash the buffer a few times.
            for _ in 0..4 {
                acc = acc.wrapping_add(fnv1a(buffer));
            }
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let elapsed = start.elapsed();
        let _ = acc;
        BenchResult {
            kind: BenchmarkKind::Cpu,
            iters,
            elapsed,
            metric: iters as f64 / elapsed.as_secs_f64(), // iters/sec
            stats: stats_from(&times),
        }
    }

    /// Memory benchmark: allocate + memset `size` bytes per iter.
    pub fn memory(&self, size: usize, iters: u64) -> BenchResult {
        let iters = iters.min(self.max_iters).max(1);
        let mut times = Vec::with_capacity(iters as usize);
        let start = Instant::now();
        let mut sink = 0u8;
        for _ in 0..iters {
            let t = Instant::now();
            let mut v = vec![0u8; size];
            // Touch every page to force real work.
            for b in v.iter_mut() {
                *b = b.wrapping_add(1);
            }
            sink ^= v[0];
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let elapsed = start.elapsed();
        let total_mb = (size * iters as usize) as f64 / (1024.0 * 1024.0);
        let _ = sink;
        BenchResult {
            kind: BenchmarkKind::Memory,
            iters,
            elapsed,
            metric: total_mb / elapsed.as_secs_f64(), // MB/s
            stats: stats_from(&times),
        }
    }

    /// "GPU" benchmark stub: measures wgpu device creation time when
    /// an adapter exists; otherwise reports 0.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn gpu(&self) -> BenchResult {
        let t = Instant::now();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }));
        let elapsed = t.elapsed();
        BenchResult {
            kind: BenchmarkKind::Gpu,
            iters: 1,
            elapsed,
            metric: if adapter.is_some() { 1.0 } else { 0.0 },
            stats: stats_from(&[elapsed.as_secs_f64() * 1000.0]),
        }
    }

    /// wasm32 fallback for gpu().
    #[cfg(target_arch = "wasm32")]
    pub fn gpu(&self) -> BenchResult {
        BenchResult {
            kind: BenchmarkKind::Gpu,
            iters: 0,
            elapsed: Duration::ZERO,
            metric: 0.0,
            stats: MetricStats::default(),
        }
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn stats_from(times_ms: &[f64]) -> MetricStats {
    let mut r = super::metrics::StatsRing::new(times_ms.len().max(1));
    for &t in times_ms {
        r.push(t);
    }
    MetricStats::from_ring(&r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_bench_runs() {
        let r = BenchmarkRunner::new();
        let res = r.cpu(&[0xAB; 1024], 100);
        assert_eq!(res.kind, BenchmarkKind::Cpu);
        assert_eq!(res.iters, 100);
        assert!(res.elapsed > Duration::ZERO);
        assert!(res.metric > 0.0);
    }

    #[test]
    fn memory_bench_reports_mb_s() {
        let r = BenchmarkRunner::new();
        let res = r.memory(64 * 1024, 20);
        assert!(res.metric > 0.0);
        assert!(res.elapsed > Duration::ZERO);
    }

    #[test]
    fn max_iters_respected() {
        let mut r = BenchmarkRunner::new();
        r.max_iters = 5;
        let res = r.cpu(&[1u8; 8], 1_000_000);
        assert_eq!(res.iters, 5);
    }
}
