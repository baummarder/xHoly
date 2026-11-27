//! Transparent roadmap progress model (Phase 27).
//!
//! All percentages are derived from `progress.json` task data. They
//! measure roadmap completion — **not** game compatibility, production
//! readiness, or retail boot success.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Raw progress file parsed from `progress.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressFile {
    /// Project name.
    pub project: String,
    /// Schema version.
    pub schema_version: u32,
    /// Human description.
    pub description: String,
    /// Roadmap categories.
    pub categories: Vec<Category>,
    /// Milestones/tasks.
    pub tasks: Vec<Task>,
    /// Date of the last verification run.
    pub last_verification_date: String,
}

/// One weighted category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Weight in percent of the whole project.
    pub weight: f64,
}

/// Task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// No work started.
    Planned,
    /// Work started with measurable implementation.
    InProgress,
    /// Some documented functionality exists.
    PartiallyImplemented,
    /// Implemented but not separately verified.
    Implemented,
    /// Verified by tests or reproducible checks.
    Verified,
    /// Blocked by an external dependency or missing evidence.
    Blocked,
}

impl TaskStatus {
    /// Fraction of the task's weight that may be credited.
    pub fn credit(self, has_evidence: bool) -> f64 {
        match self {
            Self::Planned | Self::Blocked => 0.0,
            Self::InProgress => {
                if has_evidence {
                    0.25
                } else {
                    0.0
                }
            }
            Self::PartiallyImplemented => 0.5,
            Self::Implemented | Self::Verified => 1.0,
        }
    }

    /// Stable display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "Planned",
            Self::InProgress => "In Progress",
            Self::PartiallyImplemented => "Partially Implemented",
            Self::Implemented => "Implemented",
            Self::Verified => "Verified",
            Self::Blocked => "Blocked",
        }
    }
}

/// One measurable milestone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Owning category id.
    pub category: String,
    /// Weight within the category (percent).
    pub weight: f64,
    /// Current status.
    pub status: TaskStatus,
    /// Short description.
    pub description: String,
    /// Reproducible evidence (test names or implementation references).
    pub evidence: String,
    /// Limitations or blockers.
    pub notes: String,
}

/// Derived category result.
#[derive(Debug, Clone)]
pub struct CategoryProgress {
    /// Category metadata.
    pub category: Category,
    /// Derived completion from its tasks.
    pub completion: f64,
}

/// A validated, derived progress report.
#[derive(Debug, Clone)]
pub struct ProgressReport {
    /// Parsed source file.
    pub file: ProgressFile,
    /// Derived category results.
    pub categories: Vec<CategoryProgress>,
    /// Derived overall completion.
    pub overall: f64,
}

const PROGRESS_JSON: &str = include_str!("../progress.json");
const EPSILON: f64 = 0.001;
const CALCULATION_NOTE: &str = "Calculation: each task adds `weight x credit` to its category, where credit is 0 for Planned and Blocked, 0.5 for Partially Implemented, and 1 for Implemented and Verified. Task weights total 100 within each category, so the category sum is its completion percentage. Overall completion is the category completion weighted by the category weights. Everything is derived from `progress.json` by `src/progress.rs`. This measures roadmap progress only, not game compatibility, production readiness or retail boot success.";

impl ProgressReport {
    /// Parse and derive progress from the embedded JSON file.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: ProgressFile =
            serde_json::from_str(text).map_err(|e| format!("invalid progress.json: {e}"))?;
        let report = Self::derive(file);
        report.validate()?;
        Ok(report)
    }

    fn derive(file: ProgressFile) -> Self {
        let categories: Vec<CategoryProgress> = file
            .categories
            .iter()
            .map(|category| {
                // Task weights are percentages of their category (validated to
                // total 100), so the credited sum is already a category percentage.
                let completion: f64 = file
                    .tasks
                    .iter()
                    .filter(|task| task.category == category.id)
                    .map(|task| task.weight * task.status.credit(!task.evidence.trim().is_empty()))
                    .sum();
                CategoryProgress {
                    category: category.clone(),
                    completion,
                }
            })
            .collect();
        let overall = categories
            .iter()
            .map(|c| c.completion * c.category.weight / 100.0)
            .sum();
        Self {
            file,
            categories,
            overall,
        }
    }

    /// Validate schema invariants. Percentages are derived, never stored.
    pub fn validate(&self) -> Result<(), String> {
        let file = &self.file;
        if file.schema_version != 1 {
            return Err(format!(
                "unsupported progress schema {}",
                file.schema_version
            ));
        }
        if file.last_verification_date.trim().is_empty() {
            return Err("last_verification_date is empty".into());
        }
        let category_weight: f64 = file.categories.iter().map(|c| c.weight).sum();
        if (category_weight - 100.0).abs() > EPSILON {
            return Err(format!(
                "category weights sum to {category_weight}, expected 100"
            ));
        }
        let mut category_ids = HashSet::new();
        for category in &file.categories {
            if category.weight <= 0.0 || !category_ids.insert(category.id.as_str()) {
                return Err(format!("invalid or duplicate category id {}", category.id));
            }
        }
        let mut ids = HashSet::new();
        for task in &file.tasks {
            if !ids.insert(task.id.as_str()) {
                return Err(format!("duplicate task id {}", task.id));
            }
            if task.weight <= 0.0 || task.weight > 100.0 {
                return Err(format!("invalid weight for task {}", task.id));
            }
            if !category_ids.contains(task.category.as_str()) {
                return Err(format!(
                    "task {} references unknown category {}",
                    task.id, task.category
                ));
            }
            let has_evidence = !task.evidence.trim().is_empty();
            if matches!(task.status, TaskStatus::Implemented | TaskStatus::Verified)
                && !has_evidence
            {
                return Err(format!("task {} lacks implementation evidence", task.id));
            }
            if matches!(task.status, TaskStatus::Verified)
                && !task.evidence.to_ascii_lowercase().contains("test")
            {
                return Err(format!("verified task {} lacks test evidence", task.id));
            }
            if matches!(task.status, TaskStatus::Blocked) && task.notes.trim().is_empty() {
                return Err(format!("blocked task {} needs a reason", task.id));
            }
        }
        for category in &file.categories {
            let task_weight: f64 = file
                .tasks
                .iter()
                .filter(|t| t.category == category.id)
                .map(|t| t.weight)
                .sum();
            if (task_weight - 100.0).abs() > EPSILON {
                return Err(format!(
                    "task weights for {} sum to {task_weight}, expected 100",
                    category.id
                ));
            }
        }
        Ok(())
    }

    /// Markdown block for README.md. A test checks that the README contains it.
    pub fn render_markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let _ = writeln!(o, "**Overall roadmap completion: {:.1}%**", self.overall);
        let _ = writeln!(o);
        let _ = writeln!(o, "`{}`", progress_bar(self.overall));
        let _ = writeln!(o);
        let _ = writeln!(o, "| Category | Weight | Completion |");
        let _ = writeln!(o, "|---|---:|---:|");
        for c in &self.categories {
            let _ = writeln!(
                o,
                "| {} | {:.0}% | {:.1}% |",
                c.category.name, c.category.weight, c.completion
            );
        }
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "Last verification: {}.",
            self.file.last_verification_date
        );
        let _ = writeln!(o);
        let _ = writeln!(o, "{CALCULATION_NOTE}");
        o
    }

    /// Render a compact CLI report.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{} — Project Development Progress\n",
            self.file.project
        ));
        out.push_str(&format!(
            "Overall roadmap completion: {:.1}%\n",
            self.overall
        ));
        out.push_str(&format!("{}\n", progress_bar(self.overall)));
        out.push_str("Categories (derived from verified tasks):\n");
        for c in &self.categories {
            out.push_str(&format!(
                "  - {:<28} {:>5.1}%  (weight {:.0}%)\n",
                c.category.name, c.completion, c.category.weight
            ));
        }
        out.push_str("Incomplete or blocked milestones:\n");
        for task in &self.file.tasks {
            if task.status != TaskStatus::Verified && task.status != TaskStatus::Implemented {
                out.push_str(&format!(
                    "  - [{}] {}: {}\n",
                    task.status.label(),
                    task.name,
                    task.notes
                ));
            }
        }
        out.push_str(&format!(
            "Last verification: {}\n",
            self.file.last_verification_date
        ));
        out.push_str("Roadmap progress only — not game compatibility or production readiness.\n");
        out
    }
}

/// Twenty-cell progress bar for terminal/README display.
pub fn progress_bar(percent: f64) -> String {
    const CELLS: usize = 20;
    let filled = ((percent / 100.0 * CELLS as f64).round() as usize).clamp(0, CELLS);
    format!(
        "[{}{}] {:.1}%",
        "▓".repeat(filled),
        "░".repeat(CELLS - filled),
        percent
    )
}

static REPORT: OnceLock<ProgressReport> = OnceLock::new();

/// Validated embedded progress report.
pub fn report() -> Result<&'static ProgressReport, String> {
    if let Some(report) = REPORT.get() {
        return Ok(report);
    }
    let parsed = ProgressReport::from_json(PROGRESS_JSON)?;
    Ok(REPORT.get_or_init(|| parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(tasks: &str, category_weight: f64) -> String {
        format!(
            r#"{{"project":"t","schema_version":1,"description":"d","last_verification_date":"2026-01-01","categories":[{{"id":"a","name":"A","weight":{category_weight}}}],"tasks":[{tasks}]}}"#
        )
    }

    #[test]
    fn hand_calculated_example_matches() {
        // Verified 50% of the category plus Planned 50%: category 50%, overall 50%.
        let json = tiny(
            r#"{"id":"x","name":"X","category":"a","weight":50.0,"status":"Verified","description":"d","evidence":"unit test x","notes":""},{"id":"y","name":"Y","category":"a","weight":50.0,"status":"Planned","description":"d","evidence":"","notes":""}"#,
            100.0,
        );
        let r = ProgressReport::from_json(&json).unwrap();
        assert!((r.categories[0].completion - 50.0).abs() < EPSILON);
        assert!((r.overall - 50.0).abs() < EPSILON);
    }

    #[test]
    fn embedded_progress_file_is_valid_and_reproducible() {
        let report = report().expect("progress.json must validate");
        assert_eq!(report.file.project, "xHoly");
        let second = ProgressReport::from_json(PROGRESS_JSON).unwrap();
        assert!((second.overall - report.overall).abs() < EPSILON);
        assert!((0.0..=100.0).contains(&report.overall));
    }

    #[test]
    fn category_weights_must_total_100() {
        let mut file: ProgressFile = serde_json::from_str(PROGRESS_JSON).unwrap();
        file.categories[0].weight = 19.0;
        let text = serde_json::to_string(&file).unwrap();
        assert!(ProgressReport::from_json(&text)
            .unwrap_err()
            .contains("category weights"));
    }

    #[test]
    fn duplicate_task_id_is_rejected() {
        let mut file: ProgressFile = serde_json::from_str(PROGRESS_JSON).unwrap();
        file.tasks[0].id = file.tasks[1].id.clone();
        let text = serde_json::to_string(&file).unwrap();
        assert!(ProgressReport::from_json(&text)
            .unwrap_err()
            .contains("duplicate task id"));
    }

    #[test]
    fn invalid_status_is_rejected_with_a_clear_error() {
        let broken = PROGRESS_JSON.replacen("\"status\": \"Verified\"", "\"status\": \"Bogus\"", 1);
        let err = ProgressReport::from_json(&broken).unwrap_err();
        assert!(err.contains("invalid progress.json"), "{err}");
    }

    #[test]
    fn unknown_category_is_rejected() {
        let json = tiny(
            r#"{"id":"x","name":"X","category":"nope","weight":100.0,"status":"Planned","description":"d","evidence":"","notes":""}"#,
            100.0,
        );
        assert!(ProgressReport::from_json(&json)
            .unwrap_err()
            .contains("unknown category"));
    }

    #[test]
    fn verified_without_test_evidence_is_rejected() {
        let json = tiny(
            r#"{"id":"x","name":"X","category":"a","weight":100.0,"status":"Verified","description":"d","evidence":"a file","notes":""}"#,
            100.0,
        );
        assert!(ProgressReport::from_json(&json)
            .unwrap_err()
            .contains("lacks test evidence"));
    }

    #[test]
    fn blocked_without_reason_is_rejected() {
        let json = tiny(
            r#"{"id":"x","name":"X","category":"a","weight":100.0,"status":"Blocked","description":"d","evidence":"","notes":""}"#,
            100.0,
        );
        assert!(ProgressReport::from_json(&json)
            .unwrap_err()
            .contains("needs a reason"));
    }

    #[test]
    fn blocked_and_planned_tasks_earn_no_credit() {
        assert_eq!(TaskStatus::Blocked.credit(true), 0.0);
        assert_eq!(TaskStatus::Planned.credit(true), 0.0);
        assert_eq!(TaskStatus::PartiallyImplemented.credit(true), 0.5);
    }

    #[test]
    fn progress_bar_is_bounded() {
        // Out-of-range values clamp the bar; the printed value is not clamped.
        assert!(progress_bar(-5.0).starts_with("[░░░░"));
        assert!(progress_bar(150.0).starts_with("[▓▓▓▓"));
        assert_eq!(progress_bar(50.0).matches('▓').count(), 10);
    }

    #[test]
    fn report_text_has_no_compatibility_percentage() {
        let text = report().unwrap().render_text();
        assert!(text.contains("Roadmap progress only"));
        assert!(text.contains("Last verification: 2026-11-29"));
    }

    #[test]
    fn readme_contains_the_generated_progress_section() {
        // Normalise line endings: README files are CRLF on Windows, the
        // generated block always uses LF.
        let readme = include_str!("../README.md").replace("\r\n", "\n");
        let expected = report().unwrap().render_markdown();
        assert!(
            readme.contains(&expected),
            "README progress section is stale. Expected block:\n{expected}"
        );
    }
}
