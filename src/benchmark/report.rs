//! Benchmark report generation — Markdown + HTML.

use super::benchmark::BenchResult;

/// A full report: metadata + results.
#[derive(Debug, Clone)]
pub struct Report {
    /// Report title.
    pub title: String,
    /// Free-form environment notes (GPU name, CPU, etc.).
    pub environment: Vec<String>,
    /// Benchmark results.
    pub results: Vec<BenchResult>,
}

impl Report {
    /// New report.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            environment: Vec::new(),
            results: Vec::new(),
        }
    }

    /// Add a result.
    pub fn add(&mut self, r: BenchResult) {
        self.results.push(r);
    }
}

/// Render as Markdown.
pub fn render_markdown(r: &Report) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", r.title));
    if !r.environment.is_empty() {
        s.push_str("## Environment\n\n");
        for e in &r.environment {
            s.push_str(&format!("- {e}\n"));
        }
        s.push('\n');
    }
    s.push_str("## Results\n\n");
    s.push_str("| Benchmark | Iters | Elapsed (ms) | Metric | Avg (ms) | P95 (ms) | Max (ms) |\n");
    s.push_str("|-----------|-------|--------------|--------|----------|----------|----------|\n");
    for b in &r.results {
        s.push_str(&format!(
            "| {:?} | {} | {:.1} | {:.2} | {:.3} | {:.3} | {:.3} |\n",
            b.kind,
            b.iters,
            b.elapsed.as_secs_f64() * 1000.0,
            b.metric,
            b.stats.avg,
            b.stats.p95,
            b.stats.max,
        ));
    }
    s
}

/// Render as a minimal standalone HTML page.
pub fn render_html(r: &Report) -> String {
    let mut s = String::new();
    s.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><title>");
    s.push_str(&html_escape(&r.title));
    s.push_str("</title><style>body{font-family:sans-serif;margin:2em}table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:4px 8px}</style></head><body>");
    s.push_str(&format!("<h1>{}</h1>", html_escape(&r.title)));
    if !r.environment.is_empty() {
        s.push_str("<h2>Environment</h2><ul>");
        for e in &r.environment {
            s.push_str(&format!("<li>{}</li>", html_escape(e)));
        }
        s.push_str("</ul>");
    }
    s.push_str("<h2>Results</h2><table><tr><th>Benchmark</th><th>Iters</th><th>Elapsed (ms)</th><th>Metric</th><th>Avg (ms)</th><th>P95 (ms)</th><th>Max (ms)</th></tr>");
    for b in &r.results {
        s.push_str(&format!(
            "<tr><td>{:?}</td><td>{}</td><td>{:.1}</td><td>{:.2}</td><td>{:.3}</td><td>{:.3}</td><td>{:.3}</td></tr>",
            b.kind,
            b.iters,
            b.elapsed.as_secs_f64() * 1000.0,
            b.metric,
            b.stats.avg,
            b.stats.p95,
            b.stats.max,
        ));
    }
    s.push_str("</table></body></html>");
    s
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Write report files to `dir` (markdown + html).
pub fn write_report(r: &Report, dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let slug: String = r
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    std::fs::write(dir.join(format!("{slug}.md")), render_markdown(r))?;
    std::fs::write(dir.join(format!("{slug}.html")), render_html(r))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::benchmark::{BenchResult, BenchmarkKind};
    use crate::benchmark::MetricStats;
    use std::time::Duration;

    fn sample() -> Report {
        let mut r = Report::new("Test Bench");
        r.environment.push("test-env".into());
        r.add(BenchResult {
            kind: BenchmarkKind::Cpu,
            iters: 10,
            elapsed: Duration::from_millis(100),
            metric: 100.0,
            stats: MetricStats {
                count: 10,
                avg: 10.0,
                min: 5.0,
                max: 20.0,
                p50: 10.0,
                p95: 18.0,
                p99: 20.0,
            },
        });
        r
    }

    #[test]
    fn markdown_has_table() {
        let md = render_markdown(&sample());
        assert!(md.contains("# Test Bench"));
        assert!(md.contains("| Cpu |"));
        assert!(md.contains("P95"));
    }

    #[test]
    fn html_is_valid_skeleton() {
        let h = render_html(&sample());
        assert!(h.contains("<!doctype html>"));
        assert!(h.contains("<table>"));
        assert!(h.contains("Cpu"));
    }

    #[test]
    fn write_report_creates_files() {
        let dir = std::env::temp_dir().join("xholy_benchrep");
        let _ = std::fs::remove_dir_all(&dir);
        write_report(&sample(), &dir).unwrap();
        assert!(dir.join("Test_Bench.md").exists());
        assert!(dir.join("Test_Bench.html").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
