//! Cheat triggers — when a cheat fires.

/// When a cheat applies.
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerKind {
    /// Every update tick.
    Always,
    /// Every N ticks.
    Timer { every_ticks: u64 },
    /// On a key press (logical key name).
    Key { key: String },
    /// On a named event (e.g. "level_load").
    Event { name: String },
    /// When guest memory at `addr` equals `value` (watchpoint).
    MemoryWatch { addr: u64, value: u64 },
}

/// A trigger with bookkeeping.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// Ticks since last fire (for Timer).
    pub ticks_since: u64,
    /// Latched key state (edge detection).
    pub key_held: bool,
}

impl Trigger {
    /// New always-on trigger.
    pub fn always() -> Self {
        Self {
            kind: TriggerKind::Always,
            ticks_since: 0,
            key_held: false,
        }
    }

    /// Should this trigger fire on the current tick?
    /// `events` = events raised this tick; `key_down` = is the
    /// watched key held; `mem_at` = optional memory probe fn.
    pub fn poll(
        &mut self,
        events: &[String],
        key_down: impl Fn(&str) -> bool,
        mem_probe: Option<(u64, u64)>, // (addr→value) if watching
    ) -> bool {
        self.ticks_since += 1;
        match &self.kind {
            TriggerKind::Always => true,
            TriggerKind::Timer { every_ticks } => {
                if *every_ticks == 0 {
                    return false;
                }
                if self.ticks_since >= *every_ticks {
                    self.ticks_since = 0;
                    return true;
                }
                false
            }
            TriggerKind::Key { key } => {
                let down = key_down(key);
                let fired = down && !self.key_held;
                self.key_held = down;
                fired
            }
            TriggerKind::Event { name } => events.iter().any(|e| e == name),
            TriggerKind::MemoryWatch { addr, value } => mem_probe
                .map(|(a, v)| a == *addr && v == *value)
                .unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_fires_periodically() {
        let mut t = Trigger {
            kind: TriggerKind::Timer { every_ticks: 3 },
            ticks_since: 0,
            key_held: false,
        };
        let no_ev: &[String] = &[];
        assert!(!t.poll(no_ev, |_| false, None));
        assert!(!t.poll(no_ev, |_| false, None));
        assert!(t.poll(no_ev, |_| false, None)); // 3rd tick
        assert!(!t.poll(no_ev, |_| false, None));
    }

    #[test]
    fn key_edge_trigger() {
        let mut t = Trigger {
            kind: TriggerKind::Key { key: "F1".into() },
            ticks_since: 0,
            key_held: false,
        };
        let no_ev: &[String] = &[];
        // Rising edge fires once.
        assert!(t.poll(no_ev, |_| true, None));
        // Still held → no repeat.
        assert!(!t.poll(no_ev, |_| true, None));
        // Released → no fire.
        assert!(!t.poll(no_ev, |_| false, None));
        // Pressed again → fires again.
        assert!(t.poll(no_ev, |_| true, None));
    }

    #[test]
    fn event_trigger() {
        let mut t = Trigger {
            kind: TriggerKind::Event {
                name: "load".into(),
            },
            ticks_since: 0,
            key_held: false,
        };
        let evs = vec!["load".to_string()];
        assert!(t.poll(&evs, |_| false, None));
        let none: Vec<String> = vec![];
        assert!(!t.poll(&none, |_| false, None));
    }
}
