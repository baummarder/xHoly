//! Benchmarking system — Phase 11.5.

#[allow(clippy::module_inception)]
pub mod benchmark;
pub mod metrics;
pub mod profiler;
pub mod report;

pub use benchmark::{BenchResult, BenchmarkKind, BenchmarkRunner};
pub use metrics::{FrameStats, MetricStats, StatsRing};
pub use profiler::{ProfileSample, Profiler};
pub use report::{render_html, render_markdown, Report};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::BenchmarkKind::Cpu;
    }
}
