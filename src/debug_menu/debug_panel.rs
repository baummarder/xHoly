//! Debug panel data — structured stats a UI panel can render.

/// Which panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelKind {
    /// FPS + frametime.
    #[default]
    Fps,
    /// Memory usage.
    Memory,
    /// CPU stats.
    Cpu,
    /// GPU stats.
    Gpu,
}

/// One panel's data snapshot.
#[derive(Debug, Clone, Default)]
pub struct PanelData {
    /// Panel kind.
    pub kind: PanelKind,
    /// Title line.
    pub title: String,
    /// Key/value rows.
    pub rows: Vec<(String, String)>,
}

/// Format a byte count as human-readable.
pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", b, UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// Build an FPS panel from frame stats.
pub fn fps_panel(fps: f64, frame_ms: f64, p95_ms: f64) -> PanelData {
    PanelData {
        kind: PanelKind::Fps,
        title: "Performance".into(),
        rows: vec![
            ("FPS".into(), format!("{fps:.1}")),
            ("Frame".into(), format!("{frame_ms:.2} ms")),
            ("P95".into(), format!("{p95_ms:.2} ms")),
        ],
    }
}

/// Build a memory panel.
pub fn memory_panel(used: u64, total: u64) -> PanelData {
    PanelData {
        kind: PanelKind::Memory,
        title: "Memory".into(),
        rows: vec![
            ("Used".into(), human_bytes(used)),
            ("Total".into(), human_bytes(total)),
            ("Free".into(), human_bytes(total.saturating_sub(used))),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn panels_render_rows() {
        let p = fps_panel(60.0, 16.7, 20.0);
        assert_eq!(p.rows.len(), 3);
        assert!(p.rows[0].1.contains("60"));
    }
}
