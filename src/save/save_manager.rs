//! Save manager — slot table, quick/auto/manual saves, auto-save policy.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::save_format::{self, SavePayload};
use super::save_slot::{SaveSlot, AUTO_SLOT, MAX_SLOTS, QUICK_SLOT};

/// Result of a save/load call.
#[derive(Debug)]
pub enum SaveOutcome {
    /// Success; slot id written/read.
    Ok(u8),
    /// Slot doesn't exist / empty.
    Empty(u8),
    /// Underlying format error.
    Err(String),
}

/// Auto-save policy.
#[derive(Debug, Clone)]
pub struct AutoSavePolicy {
    /// Enabled?
    pub enabled: bool,
    /// Interval seconds.
    pub interval_secs: u64,
}

impl Default for AutoSavePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 300,
        }
    }
}

/// In-memory + filesystem-backed save manager.
///
/// Slots live under `dir/<title_id>/slot_<n>.xhs`. The manager keeps
/// an in-memory slot table for fast UI listing; disk is source of
/// truth on load.
pub struct SaveManager {
    /// Save root directory.
    pub dir: PathBuf,
    /// Slot table (id → meta summary).
    slots: BTreeMap<u8, SaveSlot>,
    /// Auto-save policy.
    pub auto: AutoSavePolicy,
    /// Seconds since last auto-save tick.
    pub since_auto: u64,
}

impl SaveManager {
    /// New manager rooted at `dir`.
    pub fn new(dir: PathBuf) -> Self {
        let mut slots = BTreeMap::new();
        for i in 1..=MAX_SLOTS {
            slots.insert(i, SaveSlot::empty(i));
        }
        slots.insert(QUICK_SLOT, SaveSlot::empty(QUICK_SLOT));
        slots.insert(AUTO_SLOT, SaveSlot::empty(AUTO_SLOT));
        Self {
            dir,
            slots,
            auto: AutoSavePolicy::default(),
            since_auto: 0,
        }
    }

    /// Slot listing (sorted by id).
    pub fn slots(&self) -> Vec<&SaveSlot> {
        self.slots.values().collect()
    }

    /// Path for a slot of `title_id`.
    fn slot_path(&self, title_id: &str, slot: u8) -> PathBuf {
        self.dir.join(title_id).join(format!("slot_{slot}.xhs"))
    }

    /// Write `payload` to `slot` for `title_id`.
    pub fn save(&mut self, title_id: &str, slot: u8, payload: &SavePayload) -> SaveOutcome {
        let path = self.slot_path(title_id, slot);
        if let Some(p) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(p) {
                return SaveOutcome::Err(e.to_string());
            }
        }
        // Apply compression to the state blob before framing.
        let (tag, enc) = super::save_compression::compress(&payload.state);
        let framed = SavePayload {
            meta: payload.meta.clone(),
            state: {
                // Prefix tag byte so decode knows how to decompress.
                let mut v = vec![tag];
                v.extend_from_slice(&enc);
                v
            },
        };
        let bytes = save_format::encode(&framed);
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                let meta = payload.meta.clone();
                let size = bytes.len() as u64;
                self.slots.insert(
                    slot,
                    SaveSlot {
                        id: slot,
                        meta: Some(meta),
                        size_bytes: size,
                    },
                );
                SaveOutcome::Ok(slot)
            }
            Err(e) => SaveOutcome::Err(e.to_string()),
        }
    }

    /// Read a slot back (validates existence + integrity).
    /// Use [`load_payload`](Self::load_payload) for the actual data.
    pub fn load(&self, title_id: &str, slot: u8) -> SaveOutcome {
        let path = self.slot_path(title_id, slot);
        if !path.exists() {
            return SaveOutcome::Empty(slot);
        }
        match self.load_payload(title_id, slot) {
            Ok(_) => SaveOutcome::Ok(slot),
            Err(e) => SaveOutcome::Err(e),
        }
    }

    /// Full load returning the payload (preferred API).
    pub fn load_payload(&self, title_id: &str, slot: u8) -> Result<SavePayload, String> {
        let path = self.slot_path(title_id, slot);
        let bytes = std::fs::read(&path).map_err(|_| format!("slot {slot} empty"))?;
        let p = save_format::decode(&bytes).map_err(|e| e.to_string())?;
        if p.state.is_empty() {
            return Err("empty state".into());
        }
        let tag = p.state[0];
        let state = super::save_compression::decompress(tag, &p.state[1..])?;
        Ok(SavePayload {
            meta: p.meta,
            state,
        })
    }

    /// Delete a slot.
    pub fn delete(&mut self, title_id: &str, slot: u8) -> bool {
        let path = self.slot_path(title_id, slot);
        let ok = std::fs::remove_file(&path).is_ok();
        if ok {
            self.slots.insert(slot, SaveSlot::empty(slot));
        }
        ok
    }

    /// Refresh slot table from disk for `title_id`.
    pub fn scan(&mut self, title_id: &str) {
        for slot in self.slots.keys().copied().collect::<Vec<_>>() {
            let path = self.slot_path(title_id, slot);
            match std::fs::metadata(&path) {
                Ok(m) => {
                    // Read meta only (cheap: decode header+json).
                    if let Ok(bytes) = std::fs::read(&path) {
                        if let Ok(p) = save_format::decode(&bytes) {
                            self.slots.insert(
                                slot,
                                SaveSlot {
                                    id: slot,
                                    meta: Some(p.meta),
                                    size_bytes: m.len(),
                                },
                            );
                            continue;
                        }
                    }
                    self.slots.insert(
                        slot,
                        SaveSlot {
                            id: slot,
                            meta: None,
                            size_bytes: m.len(),
                        },
                    );
                }
                Err(_) => {
                    self.slots.insert(slot, SaveSlot::empty(slot));
                }
            }
        }
    }

    /// Advance auto-save timer; returns true when a tick is due.
    pub fn auto_tick(&mut self, elapsed_secs: u64) -> bool {
        if !self.auto.enabled {
            return false;
        }
        self.since_auto += elapsed_secs;
        if self.since_auto >= self.auto.interval_secs {
            self.since_auto = 0;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::save_slot::SaveMeta;

    fn tmp_mgr(name: &str) -> SaveManager {
        let dir = std::env::temp_dir().join(format!("xholy_savemgr_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        SaveManager::new(dir)
    }

    #[test]
    fn save_load_roundtrip() {
        let mut m = tmp_mgr("rt");
        let p = SavePayload {
            meta: SaveMeta {
                name: "s1".into(),
                ..Default::default()
            },
            state: vec![0u8; 500], // compressible
        };
        assert!(matches!(m.save("GAME", 1, &p), SaveOutcome::Ok(1)));
        let back = m.load_payload("GAME", 1).unwrap();
        assert_eq!(back.state, p.state);
        assert_eq!(back.meta.name, "s1");
    }

    #[test]
    fn empty_slot_load_fails() {
        let m = tmp_mgr("empty");
        assert!(matches!(m.load("GAME", 3), SaveOutcome::Empty(3)));
        assert!(m.load_payload("GAME", 3).is_err());
    }

    #[test]
    fn slots_isolated_per_title() {
        let mut m = tmp_mgr("iso");
        let p = SavePayload {
            meta: SaveMeta::default(),
            state: vec![1],
        };
        m.save("A", 1, &p);
        m.save("B", 1, &p);
        assert!(m.load_payload("A", 1).is_ok());
        assert!(m.load_payload("B", 1).is_ok());
        m.delete("A", 1);
        assert!(m.load_payload("A", 1).is_err());
        assert!(m.load_payload("B", 1).is_ok());
    }

    #[test]
    fn auto_tick_interval() {
        let mut m = tmp_mgr("auto");
        m.auto.interval_secs = 10;
        assert!(!m.auto_tick(5));
        assert!(m.auto_tick(5));
        assert!(!m.auto_tick(1));
    }

    #[test]
    fn stress_ten_slots() {
        let mut m = tmp_mgr("stress");
        for s in 1..=8u8 {
            let p = SavePayload {
                meta: SaveMeta {
                    name: format!("s{s}"),
                    ..Default::default()
                },
                state: vec![s; 64],
            };
            m.save("G", s, &p);
        }
        m.save(
            "G",
            QUICK_SLOT,
            &SavePayload {
                meta: SaveMeta::default(),
                state: vec![0],
            },
        );
        for s in 1..=8u8 {
            assert!(m.load_payload("G", s).is_ok());
        }
        assert!(m.load_payload("G", QUICK_SLOT).is_ok());
    }
}
