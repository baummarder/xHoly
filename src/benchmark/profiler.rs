//! Simple profiler — named counters + sampling timestamps.
//!
//! Not a statistical profiler; it's a manual instrumentation layer:
//! code marks scopes, the profiler aggregates count/total/max per
//! name. Good enough for "which subsystem is hot" debugging.

use std::collections::HashMap;
use std::time::Instant;

/// One recorded sample.
#[derive(Debug, Clone)]
pub struct ProfileSample {
    /// Scope name.
    pub name: String,
    /// Duration in microseconds.
    pub micros: u64,
}

/// Per-scope aggregate.
#[derive(Debug, Clone, Default)]
pub struct ScopeStats {
    pub count: u64,
    pub total_micros: u64,
    pub max_micros: u64,
}

/// The profiler.
#[derive(Debug, Default)]
pub struct Profiler {
    scopes: HashMap<String, ScopeStats>,
    /// Ring of recent raw samples (bounded).
    pub recent: Vec<ProfileSample>,
    /// Max recent samples kept.
    pub max_recent: usize,
}

impl Profiler {
    /// New profiler.
    pub fn new() -> Self {
        Self {
            max_recent: 1024,
            ..Default::default()
        }
    }

    /// Time a closure under `name`.
    pub fn time<T>(&mut self, name: &str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.record(name, t.elapsed());
        out
    }

    /// Record a measured duration.
    pub fn record(&mut self, name: &str, d: std::time::Duration) {
        let micros = d.as_micros() as u64;
        let s = self.scopes.entry(name.to_string()).or_default();
        s.count += 1;
        s.total_micros += micros;
        s.max_micros = s.max_micros.max(micros);
        if self.recent.len() >= self.max_recent {
            self.recent.remove(0);
        }
        self.recent.push(ProfileSample {
            name: name.to_string(),
            micros,
        });
    }

    /// All scope stats (sorted by total time desc).
    pub fn scopes_sorted(&self) -> Vec<(&str, &ScopeStats)> {
        let mut v: Vec<_> = self.scopes.iter().map(|(k, v)| (k.as_str(), v)).collect();
        v.sort_by_key(|(_, s)| std::cmp::Reverse(s.total_micros));
        v
    }

    /// Average micros per call for a scope.
    pub fn avg(&self, name: &str) -> Option<u64> {
        let s = self.scopes.get(name)?;
        s.total_micros.checked_div(s.count)
    }

    /// Reset all counters.
    pub fn reset(&mut self) {
        self.scopes.clear();
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_scopes() {
        let mut p = Profiler::new();
        p.time("a", || {
            std::thread::sleep(std::time::Duration::from_millis(1))
        });
        p.time("a", || {
            std::thread::sleep(std::time::Duration::from_millis(1))
        });
        p.time("b", || {});
        let scopes = p.scopes_sorted();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].0, "a"); // more total time
        let sa = p.scopes.get("a").unwrap();
        assert_eq!(sa.count, 2);
        assert!(sa.total_micros >= 1500);
    }

    #[test]
    fn recent_bounded() {
        let mut p = Profiler::new();
        p.max_recent = 4;
        for _ in 0..10 {
            p.record("x", std::time::Duration::from_micros(1));
        }
        assert_eq!(p.recent.len(), 4);
    }
}
