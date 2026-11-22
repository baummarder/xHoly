//! Telemetry filter — rule-based event filtering.
//!
//! Keeps telemetry anonymized: rules can drop events containing
//! suspicious attribute keys (paths, usernames, serials) or limit
//! which kinds are sent.

use super::telemetry::TelemetryEvent;

/// A single filter rule.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterRule {
    /// Only allow these event kinds.
    AllowKinds(Vec<crate::telemetry::telemetry::EventKind>),
    /// Drop events that have any of these attribute keys.
    DenyAttrKeys(Vec<String>),
    /// Drop events with any attribute *value* matching a substring.
    DenyAttrValues(Vec<String>),
    /// Sample rate (0.0..=1.0) — deterministic per event via hash.
    SampleRate(f64),
}

/// The filter: conjunction of rules (all must pass).
#[derive(Debug, Clone, Default)]
pub struct TelemetryFilter {
    rules: Vec<FilterRule>,
}

impl TelemetryFilter {
    /// New empty filter (allow all).
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a rule.
    pub fn add(&mut self, r: FilterRule) -> &mut Self {
        self.rules.push(r);
        self
    }
    /// Convenience: allow only certain kinds.
    pub fn allow_kinds(&mut self, kinds: &[crate::telemetry::telemetry::EventKind]) -> &mut Self {
        self.add(FilterRule::AllowKinds(kinds.to_vec()));
        self
    }
    /// Convenience: deny attribute keys (PII guard).
    pub fn deny_keys(&mut self, keys: &[&str]) -> &mut Self {
        self.add(FilterRule::DenyAttrKeys(
            keys.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Should this event pass?
    pub fn allows(&self, ev: &TelemetryEvent) -> bool {
        for r in &self.rules {
            match r {
                FilterRule::AllowKinds(kinds) => {
                    if !kinds.contains(&ev.kind) {
                        return false;
                    }
                }
                FilterRule::DenyAttrKeys(keys) => {
                    for (k, _) in &ev.attrs {
                        if keys.iter().any(|d| k == d) {
                            return false;
                        }
                    }
                }
                FilterRule::DenyAttrValues(subs) => {
                    for (_, v) in &ev.attrs {
                        if subs.iter().any(|s| v.contains(s.as_str())) {
                            return false;
                        }
                    }
                }
                FilterRule::SampleRate(rate) => {
                    if !sample_pass(ev, *rate) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// As a closure for `TelemetryCollector`.
    pub fn predicate(&self) -> impl Fn(&TelemetryEvent) -> bool + '_ {
        |ev| self.allows(ev)
    }
}

/// Deterministic sampling: hash event → [0,1) < rate.
fn sample_pass(ev: &TelemetryEvent, rate: f64) -> bool {
    if rate >= 1.0 {
        return true;
    }
    if rate <= 0.0 {
        return false;
    }
    // FNV over kind+ts+version.
    let mut h = 0xcbf29ce484222325u64;
    for b in ev
        .app_version
        .bytes()
        .chain(ev.platform.bytes())
        .chain(ev.ts.to_le_bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let norm = (h >> 11) as f64 / (1u64 << 53) as f64;
    norm < rate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::telemetry::EventKind;

    #[test]
    fn deny_keys_blocks_pii() {
        let mut f = TelemetryFilter::new();
        f.deny_keys(&["username", "path", "serial"]);
        let clean =
            TelemetryEvent::new(EventKind::FeatureUse, "v", "p").attr("feature", "shader_cache");
        let dirty = TelemetryEvent::new(EventKind::FeatureUse, "v", "p").attr("username", "admin");
        assert!(f.allows(&clean));
        assert!(!f.allows(&dirty));
    }

    #[test]
    fn allow_kinds_whitelist() {
        let mut f = TelemetryFilter::new();
        f.allow_kinds(&[EventKind::Crash, EventKind::SessionStart]);
        assert!(f.allows(&TelemetryEvent::new(EventKind::Crash, "v", "p")));
        assert!(!f.allows(&TelemetryEvent::new(EventKind::FeatureUse, "v", "p")));
    }

    #[test]
    fn sample_rate_bounds() {
        let mut f = TelemetryFilter::new();
        f.add(FilterRule::SampleRate(0.0));
        let ev = TelemetryEvent::new(EventKind::FeatureUse, "v", "p");
        assert!(!f.allows(&ev));
        let mut f2 = TelemetryFilter::new();
        f2.add(FilterRule::SampleRate(1.0));
        assert!(f2.allows(&ev));
    }
}
