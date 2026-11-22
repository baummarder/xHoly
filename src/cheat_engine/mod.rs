//! Cheat engine — Phase 11.5. (Single-player cheats; no anti-cheat.)

pub mod cheat_compiler;
pub mod cheat_list;
pub mod cheat_manager;
pub mod cheat_trigger;
pub mod cheat_value;

pub use cheat_compiler::{compile, CheatPatch, CompileError};
pub use cheat_list::{CheatBook, CheatEntry};
pub use cheat_manager::{CheatManager, CheatStatus};
pub use cheat_trigger::{Trigger, TriggerKind};
pub use cheat_value::{CheatValue, MemoryRegion};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::CheatPatch {
            address: 0,
            bytes: vec![],
        };
    }
}
