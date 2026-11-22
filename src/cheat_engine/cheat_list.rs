//! Cheat list — named, persistable cheat collections.

use serde::{Deserialize, Serialize};

use super::cheat_trigger::TriggerKind;
use super::cheat_value::CheatValue;

/// One cheat entry (UI-facing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheatEntry {
    /// Display name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Guest address.
    pub address: u64,
    /// Region (default Ram).
    #[serde(default)]
    pub region: MemoryRegionSerde,
    /// Value to write.
    pub value: CheatValueSerde,
    /// Trigger (default Always).
    #[serde(default)]
    pub trigger: TriggerKindSerde,
    /// Enabled flag.
    #[serde(default)]
    pub enabled: bool,
    /// Hotkey (logical key name, optional).
    #[serde(default)]
    pub hotkey: Option<String>,
}

/// Serde-friendly region (CheatValue already isn't Serialize — keep
/// cheat types plain-data for the book file).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MemoryRegionSerde {
    #[default]
    Ram,
    Vram,
    Kernel,
}

/// Serde-friendly value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CheatValueSerde {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    Bytes(Vec<u8>),
}

impl From<&CheatValueSerde> for CheatValue {
    fn from(v: &CheatValueSerde) -> Self {
        match v {
            CheatValueSerde::U8(x) => Self::U8(*x),
            CheatValueSerde::U16(x) => Self::U16(*x),
            CheatValueSerde::U32(x) => Self::U32(*x),
            CheatValueSerde::U64(x) => Self::U64(*x),
            CheatValueSerde::F32(x) => Self::F32(*x),
            CheatValueSerde::Bytes(b) => Self::Bytes(b.clone()),
        }
    }
}

/// Serde-friendly trigger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TriggerKindSerde {
    #[default]
    Always,
    Timer {
        every_ticks: u64,
    },
    Key {
        key: String,
    },
    Event {
        name: String,
    },
    MemoryWatch {
        addr: u64,
        value: u64,
    },
}

impl From<&TriggerKindSerde> for TriggerKind {
    fn from(t: &TriggerKindSerde) -> Self {
        match t {
            TriggerKindSerde::Always => Self::Always,
            TriggerKindSerde::Timer { every_ticks } => Self::Timer {
                every_ticks: *every_ticks,
            },
            TriggerKindSerde::Key { key } => Self::Key { key: key.clone() },
            TriggerKindSerde::Event { name } => Self::Event { name: name.clone() },
            TriggerKindSerde::MemoryWatch { addr, value } => Self::MemoryWatch {
                addr: *addr,
                value: *value,
            },
        }
    }
}

/// A cheat book (one per game).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheatBook {
    /// Game/title id.
    pub title_id: String,
    /// Cheats.
    #[serde(default)]
    pub cheats: Vec<CheatEntry>,
}

impl CheatBook {
    /// New book for a title.
    pub fn new(title_id: impl Into<String>) -> Self {
        Self {
            title_id: title_id.into(),
            cheats: Vec::new(),
        }
    }

    /// Add an entry.
    pub fn add(&mut self, e: CheatEntry) {
        self.cheats.push(e);
    }

    /// Remove by index.
    pub fn remove(&mut self, idx: usize) -> Option<CheatEntry> {
        if idx < self.cheats.len() {
            Some(self.cheats.remove(idx))
        } else {
            None
        }
    }

    /// Save to JSON text.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Load from JSON text.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_json_roundtrip() {
        let mut b = CheatBook::new("GAME1");
        b.add(CheatEntry {
            name: "god mode".into(),
            description: "999 hp".into(),
            address: 0x1234,
            region: MemoryRegionSerde::Ram,
            value: CheatValueSerde::U32(999),
            trigger: TriggerKindSerde::Key { key: "F2".into() },
            enabled: true,
            hotkey: Some("F2".into()),
        });
        let s = b.to_json().unwrap();
        let back = CheatBook::from_json(&s).unwrap();
        assert_eq!(back.cheats.len(), 1);
        assert_eq!(back.cheats[0].name, "god mode");
    }
}
