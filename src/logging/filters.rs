//! Log filters — level floor per module + global denylist.

use super::{Level, Record};

/// Per-module level override.
#[derive(Debug, Clone)]
pub struct ModuleFilter {
    /// Module path prefix (e.g. `core::gpu`).
    pub prefix: String,
    /// Minimum level for this module.
    pub min: Level,
}

/// A composed filter: global floor + per-module overrides +
/// substring denylist.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Modules whose messages containing any of these are dropped.
    pub deny_substrings: Vec<String>,
    /// Per-module minimums (longest-prefix match wins).
    pub modules: Vec<ModuleFilter>,
}

impl Filter {
    /// Accept/reject a record.
    pub fn accepts(&self, rec: &Record) -> bool {
        if self
            .deny_substrings
            .iter()
            .any(|d| rec.message.contains(d.as_str()))
        {
            return false;
        }
        // Longest matching prefix decides.
        let best = self
            .modules
            .iter()
            .filter(|m| rec.module.starts_with(&m.prefix))
            .max_by_key(|m| m.prefix.len());
        match best {
            Some(m) => rec.level >= m.min,
            None => true, // no module rule → logger floor applies
        }
    }
}

/// Simple level-floor filter (all modules same minimum).
#[derive(Debug, Clone, Copy)]
pub struct LevelFilter(pub Level);

impl LevelFilter {
    /// Accept records at or above the floor.
    pub fn accepts(&self, rec: &Record) -> bool {
        rec.level >= self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(level: Level, module: &str, msg: &str) -> Record {
        Record {
            level,
            module: module.into(),
            message: msg.into(),
            span: None,
        }
    }

    #[test]
    fn deny_substring() {
        let f = Filter {
            deny_substrings: vec!["secret".into()],
            modules: vec![],
        };
        assert!(!f.accepts(&rec(Level::Info, "m", "a secret thing")));
        assert!(f.accepts(&rec(Level::Info, "m", "public")));
    }

    #[test]
    fn module_prefix_levels() {
        let f = Filter {
            deny_substrings: vec![],
            modules: vec![
                ModuleFilter {
                    prefix: "core::gpu".into(),
                    min: Level::Warn,
                },
                ModuleFilter {
                    prefix: "core::gpu::verbose".into(),
                    min: Level::Trace,
                },
            ],
        };
        assert!(!f.accepts(&rec(Level::Info, "core::gpu", "x")));
        assert!(f.accepts(&rec(Level::Warn, "core::gpu", "x")));
        // Longer prefix wins.
        assert!(f.accepts(&rec(Level::Trace, "core::gpu::verbose", "x")));
        // Unrelated module unaffected.
        assert!(f.accepts(&rec(Level::Info, "gui", "x")));
    }
}
