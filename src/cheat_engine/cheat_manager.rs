//! Cheat manager — enable/disable cheats, apply patches, hotkeys.

use std::collections::HashMap;

use super::cheat_list::CheatEntry;
use super::cheat_trigger::{Trigger, TriggerKind};
use super::cheat_value::{CheatValue, MemoryRegion};

/// Status of one managed cheat.
#[derive(Debug, Clone, PartialEq)]
pub enum CheatStatus {
    /// Registered but disabled.
    Disabled,
    /// Enabled, waiting for trigger.
    Armed,
    /// Fired this tick.
    Fired,
}

/// A live cheat instance (compiled from an entry).
struct LiveCheat {
    entry: CheatEntry,
    trigger: Trigger,
    status: CheatStatus,
}

/// The cheat manager: owns live cheats + a simulated guest memory.
pub struct CheatManager {
    /// title_id → live cheats.
    cheats: HashMap<String, Vec<LiveCheat>>,
    /// Simulated guest RAM for patching (tests/real use via `mem`).
    pub ram: Vec<u8>,
    /// Guest base address `ram` maps to.
    pub ram_base: u64,
    /// Events raised this tick (consumed by triggers).
    pub events: Vec<String>,
}

impl CheatManager {
    /// New manager with a `size`-byte RAM at `ram_base`.
    pub fn new(ram_base: u64, ram_size: usize) -> Self {
        Self {
            cheats: HashMap::new(),
            ram: vec![0u8; ram_size],
            ram_base,
            events: Vec::new(),
        }
    }

    /// Install cheats for a title (replaces existing).
    pub fn install(&mut self, title_id: &str, entries: Vec<CheatEntry>) {
        let live = entries
            .into_iter()
            .map(|e| LiveCheat {
                trigger: Trigger {
                    kind: TriggerKind::from(&e.trigger),
                    ticks_since: 0,
                    key_held: false,
                },
                entry: e,
                status: CheatStatus::Disabled,
            })
            .collect();
        self.cheats.insert(title_id.to_string(), live);
    }

    /// Enable a cheat by index.
    pub fn enable(&mut self, title_id: &str, idx: usize) -> bool {
        if let Some(c) = self.cheats.get_mut(title_id).and_then(|v| v.get_mut(idx)) {
            c.status = CheatStatus::Armed;
            true
        } else {
            false
        }
    }

    /// Disable a cheat by index.
    pub fn disable(&mut self, title_id: &str, idx: usize) -> bool {
        if let Some(c) = self.cheats.get_mut(title_id).and_then(|v| v.get_mut(idx)) {
            c.status = CheatStatus::Disabled;
            true
        } else {
            false
        }
    }

    /// Toggle.
    pub fn toggle(&mut self, title_id: &str, idx: usize) -> bool {
        if let Some(c) = self.cheats.get_mut(title_id).and_then(|v| v.get_mut(idx)) {
            c.status = if c.status == CheatStatus::Disabled {
                CheatStatus::Armed
            } else {
                CheatStatus::Disabled
            };
            return true;
        }
        false
    }

    /// Read a cheat's current guest value.
    pub fn read_value(&self, region: MemoryRegion, addr: u64) -> Option<CheatValue> {
        match region {
            MemoryRegion::Ram => {
                let off = addr.checked_sub(self.ram_base)? as usize;
                if off + 4 <= self.ram.len() {
                    Some(CheatValue::U32(u32::from_le_bytes(
                        self.ram[off..off + 4].try_into().ok()?,
                    )))
                } else {
                    None
                }
            }
            _ => None, // Vram/Kernel not simulated here
        }
    }

    /// Write bytes into simulated RAM (if in range).
    pub fn write_ram(&mut self, addr: u64, bytes: &[u8]) -> bool {
        let Some(off) = addr.checked_sub(self.ram_base) else {
            return false;
        };
        let off = off as usize;
        if off + bytes.len() > self.ram.len() {
            return false;
        }
        self.ram[off..off + bytes.len()].copy_from_slice(bytes);
        true
    }

    /// Poll all cheats for `title_id`; fires triggered ones.
    /// `key_down` reports held state for Key triggers.
    /// `mem_probe` supplies (addr,value) for MemoryWatch triggers.
    pub fn tick(
        &mut self,
        title_id: &str,
        key_down: impl Fn(&str) -> bool,
        mem_probe: Option<(u64, u64)>,
    ) -> usize {
        let Some(list) = self.cheats.get_mut(title_id) else {
            return 0;
        };
        // Collect patches first (immutable view), then apply writes.
        struct Patch {
            addr: u64,
            bytes: Vec<u8>,
        }
        let mut patches = Vec::new();
        let mut fired = 0;
        for c in list.iter_mut() {
            if c.status == CheatStatus::Disabled {
                continue;
            }
            if !c.trigger.poll(&self.events, &key_down, mem_probe) {
                continue;
            }
            let region = match c.entry.region {
                super::cheat_list::MemoryRegionSerde::Ram => MemoryRegion::Ram,
                super::cheat_list::MemoryRegionSerde::Vram => MemoryRegion::Vram,
                super::cheat_list::MemoryRegionSerde::Kernel => MemoryRegion::Kernel,
            };
            if region == MemoryRegion::Ram {
                patches.push(Patch {
                    addr: c.entry.address,
                    bytes: CheatValue::from(&c.entry.value).to_bytes(),
                });
            }
            c.status = CheatStatus::Fired;
            fired += 1;
        }
        // `list` borrow ends here (NLL); apply writes.
        for p in patches {
            let _ = self.write_ram(p.addr, &p.bytes);
        }
        self.events.clear();
        fired
    }

    /// Cheat statuses for UI.
    pub fn statuses(&self, title_id: &str) -> Vec<CheatStatus> {
        self.cheats
            .get(title_id)
            .map(|v| v.iter().map(|c| c.status.clone()).collect())
            .unwrap_or_default()
    }

    /// Number of cheats for a title.
    pub fn count(&self, title_id: &str) -> usize {
        self.cheats.get(title_id).map(|v| v.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cheat_engine::cheat_list::{CheatValueSerde, MemoryRegionSerde, TriggerKindSerde};

    fn entry(addr: u64, val: u32, trig: TriggerKindSerde) -> CheatEntry {
        CheatEntry {
            name: "c".into(),
            description: String::new(),
            address: addr,
            region: MemoryRegionSerde::Ram,
            value: CheatValueSerde::U32(val),
            trigger: trig,
            enabled: true,
            hotkey: None,
        }
    }

    #[test]
    fn always_trigger_writes_ram() {
        let mut m = CheatManager::new(0x1000, 256);
        m.install("G", vec![entry(0x1010, 0xABCD, TriggerKindSerde::Always)]);
        m.enable("G", 0);
        let fired = m.tick("G", |_| false, None);
        assert_eq!(fired, 1);
        let got = m.read_value(MemoryRegion::Ram, 0x1010).unwrap();
        assert_eq!(got, CheatValue::U32(0xABCD));
    }

    #[test]
    fn disabled_cheat_does_not_fire() {
        let mut m = CheatManager::new(0x1000, 256);
        m.install("G", vec![entry(0x1010, 1, TriggerKindSerde::Always)]);
        // stays Disabled
        assert_eq!(m.tick("G", |_| false, None), 0);
    }

    #[test]
    fn timer_trigger_fires_on_schedule() {
        let mut m = CheatManager::new(0x1000, 256);
        m.install(
            "G",
            vec![entry(0x1010, 7, TriggerKindSerde::Timer { every_ticks: 2 })],
        );
        m.enable("G", 0);
        assert_eq!(m.tick("G", |_| false, None), 0);
        assert_eq!(m.tick("G", |_| false, None), 1);
        assert_eq!(m.tick("G", |_| false, None), 0);
    }

    #[test]
    fn hotkey_toggle() {
        let mut m = CheatManager::new(0x1000, 256);
        m.install("G", vec![entry(0x1010, 1, TriggerKindSerde::Always)]);
        assert!(m.toggle("G", 0));
        assert_eq!(m.statuses("G")[0], CheatStatus::Armed);
        assert!(m.toggle("G", 0));
        assert_eq!(m.statuses("G")[0], CheatStatus::Disabled);
    }
}
