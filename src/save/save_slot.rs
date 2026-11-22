//! Save slots — metadata + fixed slot table.

/// Total user-visible slots (1–8) plus reserved quick/auto slots.
pub const MAX_SLOTS: u8 = 8;
/// Reserved slot id for quick-save (outside 1..=8).
pub const QUICK_SLOT: u8 = 0xFF;
/// Reserved slot id for auto-save.
pub const AUTO_SLOT: u8 = 0xFE;

/// Metadata stored alongside a save's payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SaveMeta {
    /// Display name (defaults to slot label).
    pub name: String,
    /// Game/title id this save belongs to.
    pub title_id: String,
    /// Free-form description.
    pub description: String,
    /// Play time in seconds when saved.
    pub playtime_secs: u64,
    /// Creation timestamp (RFC3339).
    pub created_at: String,
    /// Last write timestamp.
    pub updated_at: String,
    /// xHoly save-format version.
    pub format_version: u32,
}

impl Default for SaveMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            title_id: String::new(),
            description: String::new(),
            playtime_secs: 0,
            created_at: String::new(),
            updated_at: String::new(),
            format_version: super::save_format::FORMAT_VERSION,
        }
    }
}

/// A slot in the manager's table.
#[derive(Debug, Clone)]
pub struct SaveSlot {
    /// Slot id (1..=8, QUICK_SLOT, AUTO_SLOT).
    pub id: u8,
    /// Metadata (None = empty slot).
    pub meta: Option<SaveMeta>,
    /// File size in bytes when occupied.
    pub size_bytes: u64,
}

impl SaveSlot {
    /// Empty slot descriptor.
    pub fn empty(id: u8) -> Self {
        Self {
            id,
            meta: None,
            size_bytes: 0,
        }
    }

    /// Human label.
    pub fn label(&self) -> String {
        match self.id {
            QUICK_SLOT => "Quick".into(),
            AUTO_SLOT => "Auto".into(),
            n => format!("Slot {n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_labels() {
        assert_eq!(SaveSlot::empty(3).label(), "Slot 3");
        assert_eq!(SaveSlot::empty(QUICK_SLOT).label(), "Quick");
        assert_eq!(SaveSlot::empty(AUTO_SLOT).label(), "Auto");
    }

    #[test]
    fn meta_defaults() {
        let m = SaveMeta::default();
        assert_eq!(m.format_version, crate::save::save_format::FORMAT_VERSION);
    }
}
