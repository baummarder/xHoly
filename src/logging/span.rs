//! Trace spans — named timing scopes for profiling.

use std::time::Instant;

/// A started span; dropping it is a no-op (use [`Span::finish`]).
pub struct Span {
    name: String,
    start: Instant,
    finished: bool,
}

impl Span {
    /// Begin a named span.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start: Instant::now(),
            finished: false,
        }
    }

    /// Elapsed without finishing.
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    /// Finish and return (name, duration).
    pub fn finish(mut self) -> (String, std::time::Duration) {
        self.finished = true;
        (self.name.clone(), self.start.elapsed())
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.finished {
            // Log unfinished spans at debug level so leaks are visible.
            crate::logging::global()
                .debug("span", format!("span '{}' dropped unfinished", self.name));
        }
    }
}

/// Convenience: time a closure, returning (result, duration).
pub fn timeit<T>(name: &str, f: impl FnOnce() -> T) -> (T, std::time::Duration) {
    let s = Span::new(name);
    let out = f();
    let (_, d) = s.finish();
    (out, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_reports_duration() {
        let s = Span::new("t");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (name, d) = s.finish();
        assert_eq!(name, "t");
        assert!(d >= std::time::Duration::from_millis(4));
    }

    #[test]
    fn timeit_wraps() {
        let (v, d) = timeit("add", || 2 + 2);
        assert_eq!(v, 4);
        assert!(d < std::time::Duration::from_secs(1));
    }
}
