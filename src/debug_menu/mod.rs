//! Debug menu / overlay tooling — Phase 11.5.
//!
//! Headless-friendly debug surfaces: FPS/memory/CPU/GPU panels,
//! render-target inspection state, wireframe flag, memory dump +
//! shader-inspector helpers. The TUI/GUI binds to these; nothing
//! here requires a window.

#[allow(clippy::module_inception)]
pub mod debug_menu;
pub mod debug_overlay;
pub mod debug_panel;
pub mod debug_tools;

pub use debug_menu::{DebugMenu, MenuItem};
pub use debug_overlay::{DebugOverlay, OverlayFlags};
pub use debug_panel::{PanelData, PanelKind};
pub use debug_tools::{dump_region, inspect_shader, DumpError, ShaderInfo};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::PanelKind::Fps;
    }
}
