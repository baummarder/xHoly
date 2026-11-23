//! UI widget system — Phase 9.
//!
//! A retained-mode widget tree layered on top of the immediate-mode
//! software compositor ([`crate::gui::windowed::ui`]). Widgets know
//! their rect, visual state (normal/hover/pressed/disabled) and z-order;
//! the tree resolves hover/click/drag against the mouse position and
//! emits draw commands that are rasterized into the RGBA framebuffer.
//!
//! This is intentionally CPU-side: the widgets produce *pixels*, the
//! Phase-9 post-FX chain (bloom/tonemap/FXAA) polishes them on GPU.

use crate::gui::windowed::ui::{draw_text, text_width, Framebuffer};

/// Integer rectangle (same layout as `ui::Rect`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Point-in-rect test.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// Center point.
    pub fn center(&self) -> (u32, u32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
}

/// Visual/interaction state of a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetState {
    Normal,
    Hover,
    Pressed,
    Disabled,
}

/// Widget kinds supported by the Phase-9 UI layer.
#[derive(Debug, Clone)]
pub enum Widget {
    /// Labeled button with hover/press feedback.
    Button {
        id: u32,
        label: String,
        rect: Rect,
        state: WidgetState,
        z: u32,
    },
    /// Container panel (background + optional border).
    Panel {
        id: u32,
        rect: Rect,
        z: u32,
        accent: bool,
    },
    /// Read-only label.
    Label {
        id: u32,
        text: String,
        pos: (u32, u32),
        scale: u32,
        z: u32,
    },
    /// Horizontal slider (visual only — value set programmatically).
    Slider {
        id: u32,
        rect: Rect,
        value: f32,
        z: u32,
    },
}

impl Widget {
    /// Stable id.
    pub fn id(&self) -> u32 {
        match self {
            Self::Button { id, .. }
            | Self::Panel { id, .. }
            | Self::Label { id, .. }
            | Self::Slider { id, .. } => *id,
        }
    }

    /// Z-order (higher draws on top).
    pub fn z(&self) -> u32 {
        match self {
            Self::Button { z, .. }
            | Self::Panel { z, .. }
            | Self::Label { z, .. }
            | Self::Slider { z, .. } => *z,
        }
    }

    /// Hit-test bounds (labels are point-sized).
    pub fn hit_rect(&self) -> Rect {
        match self {
            Self::Button { rect, .. } | Self::Panel { rect, .. } | Self::Slider { rect, .. } => {
                *rect
            }
            Self::Label {
                text, pos, scale, ..
            } => Rect {
                x: pos.0,
                y: pos.1,
                w: text_width(text, *scale),
                h: 7 * scale,
            },
        }
    }

    /// Whether the widget accepts pointer interaction.
    fn interactive(&self) -> bool {
        matches!(self, Self::Button { .. } | Self::Slider { .. })
    }
}

/// A click/drag result emitted by [`WidgetTree::update`].
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetEvent {
    /// Button pressed this frame.
    Clicked(u32),
    /// Slider dragged to a new value.
    Dragged { id: u32, value: f32 },
}

/// Retained widget tree with hover/press/drag tracking and z-ordering.
#[derive(Debug, Default)]
pub struct WidgetTree {
    widgets: Vec<Widget>,
    /// Widget currently under the mouse (topmost by z).
    hovered: Option<u32>,
    /// Widget currently being dragged/pressed.
    active: Option<u32>,
    /// Next auto id.
    next_id: u32,
}

impl WidgetTree {
    /// Empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a widget (id auto-assigned if 0).
    pub fn push(&mut self, mut w: Widget) {
        if w.id() == 0 {
            self.next_id += 1;
            let id = self.next_id;
            match &mut w {
                Widget::Button { id: i, .. }
                | Widget::Panel { id: i, .. }
                | Widget::Label { id: i, .. }
                | Widget::Slider { id: i, .. } => *i = id,
            }
        }
        self.widgets.push(w);
    }

    /// Convenience: button.
    pub fn button(&mut self, label: &str, rect: Rect, z: u32) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.push(Widget::Button {
            id,
            label: label.into(),
            rect,
            state: WidgetState::Normal,
            z,
        });
        id
    }

    /// Iterate widgets sorted by z (painter's order).
    pub fn sorted(&self) -> Vec<&Widget> {
        let mut v: Vec<&Widget> = self.widgets.iter().collect();
        v.sort_by_key(|w| w.z());
        v
    }

    /// Find topmost interactive widget at (x, y).
    pub fn hit_test(&self, x: u32, y: u32) -> Option<u32> {
        self.sorted()
            .iter()
            .rev()
            .find(|w| w.interactive() && w.hit_rect().contains(x, y))
            .map(|w| w.id())
    }

    /// Update hover/press state from mouse input.
    ///
    /// `mouse_down` is the *current* button state; `mouse_pressed` is a
    /// fresh click this frame. Returns events for the caller to act on.
    pub fn update(
        &mut self,
        mouse: (u32, u32),
        mouse_down: bool,
        mouse_pressed: bool,
    ) -> Vec<WidgetEvent> {
        let mut events = Vec::new();
        let hit = self.hit_test(mouse.0, mouse.1);
        self.hovered = hit;

        // Press: activate the widget under the cursor.
        if mouse_pressed {
            if let Some(id) = hit {
                self.active = Some(id);
            }
        }

        // Drag/click resolution.
        if let Some(id) = self.active {
            let Some(w) = self.widgets.iter_mut().find(|w| w.id() == id) else {
                self.active = None;
                return events;
            };
            match w {
                Widget::Button { state, .. } => {
                    if mouse_down {
                        *state = WidgetState::Pressed;
                    } else {
                        // Released: click fires only if still hovered.
                        if self.hovered == Some(id) {
                            events.push(WidgetEvent::Clicked(id));
                        }
                        *state = WidgetState::Normal;
                        self.active = None;
                    }
                }
                Widget::Slider { rect, value, .. } => {
                    if mouse_down {
                        let t = ((mouse.0 as i64 - rect.x as i64) as f32 / rect.w.max(1) as f32)
                            .clamp(0.0, 1.0);
                        if (t - *value).abs() > f32::EPSILON {
                            *value = t;
                            events.push(WidgetEvent::Dragged { id, value: t });
                        }
                    } else {
                        self.active = None;
                    }
                }
                _ => self.active = None,
            }
        }

        // Hover states for non-active widgets.
        for w in self.widgets.iter_mut() {
            if let Widget::Button { id, state, .. } = w {
                if self.active != Some(*id) {
                    *state = if self.hovered == Some(*id) {
                        WidgetState::Hover
                    } else {
                        WidgetState::Normal
                    };
                }
            }
        }
        events
    }

    /// Rasterize all widgets into the framebuffer (z-order respected).
    pub fn draw(&self, fb: &mut Framebuffer, fb_w: u32, fb_h: u32) {
        for w in self.sorted() {
            match w {
                Widget::Panel { rect, accent, .. } => {
                    crate::gui::windowed::ui::fill_rect_pub(
                        fb,
                        fb_w,
                        rect.x,
                        rect.y,
                        rect.w,
                        rect.h,
                        if *accent {
                            crate::theme::mix(
                                crate::theme::PANEL_ALT,
                                crate::theme::TRANS_BLUE,
                                0.30,
                            )
                        } else {
                            crate::theme::PANEL
                        },
                    );
                    if *accent {
                        crate::gui::windowed::ui::stroke_rect_pub(
                            fb,
                            fb_w,
                            rect.x,
                            rect.y,
                            rect.w,
                            rect.h,
                            crate::theme::TRANS_BLUE,
                        );
                    }
                }
                Widget::Button {
                    label, rect, state, ..
                } => draw_button(fb, fb_w, fb_h, rect, label, *state),
                Widget::Label {
                    text, pos, scale, ..
                } => draw_text(
                    fb,
                    fb_w,
                    fb_h,
                    pos.0,
                    pos.1,
                    text,
                    crate::theme::TEXT,
                    *scale,
                ),
                Widget::Slider { rect, value, .. } => draw_slider(fb, fb_w, fb_h, rect, *value),
            }
        }
    }
}

fn draw_button(
    fb: &mut Framebuffer,
    fb_w: u32,
    fb_h: u32,
    rect: &Rect,
    label: &str,
    state: WidgetState,
) {
    use crate::theme;
    let (bg, border, fg) = match state {
        WidgetState::Normal => (theme::PANEL_ALT, theme::TRANS_BLUE, theme::TRANS_WHITE),
        WidgetState::Hover => (
            theme::shade(theme::PANEL_ALT, 1.35),
            theme::TRANS_PINK,
            theme::TRANS_WHITE,
        ),
        WidgetState::Pressed => (
            theme::mix(theme::PANEL_ALT, theme::TRANS_PINK, 0.35),
            theme::TRANS_WHITE,
            theme::TRANS_WHITE,
        ),
        WidgetState::Disabled => (theme::INSET, theme::DIM, theme::MUTED),
    };
    crate::gui::windowed::ui::fill_rect_pub(fb, fb_w, rect.x, rect.y, rect.w, rect.h, bg);
    crate::gui::windowed::ui::stroke_rect_pub(fb, fb_w, rect.x, rect.y, rect.w, rect.h, border);
    let tw = text_width(label, 1);
    draw_text(
        fb,
        fb_w,
        fb_h,
        rect.x + rect.w.saturating_sub(tw) / 2,
        rect.y + rect.h.saturating_sub(7) / 2,
        label,
        fg,
        1,
    );
}

fn draw_slider(fb: &mut Framebuffer, fb_w: u32, fb_h: u32, rect: &Rect, value: f32) {
    crate::gui::windowed::ui::fill_rect_pub(
        fb,
        fb_w,
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        crate::theme::INSET,
    );
    let fill_w = (rect.w as f32 * value.clamp(0.0, 1.0)) as u32;
    crate::gui::windowed::ui::fill_rect_pub(
        fb,
        fb_w,
        rect.x,
        rect.y,
        fill_w,
        rect.h,
        crate::theme::trans_gradient(value.clamp(0.0, 1.0)),
    );
    crate::gui::windowed::ui::stroke_rect_pub(
        fb,
        fb_w,
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        crate::theme::DIM,
    );
    let _ = fb_h;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> WidgetTree {
        let mut t = WidgetTree::new();
        t.button(
            "A",
            Rect {
                x: 0,
                y: 0,
                w: 50,
                h: 20,
            },
            1,
        );
        t.button(
            "B",
            Rect {
                x: 100,
                y: 0,
                w: 50,
                h: 20,
            },
            2,
        ); // higher z
        t.push(Widget::Slider {
            id: 0,
            rect: Rect {
                x: 0,
                y: 40,
                w: 100,
                h: 10,
            },
            value: 0.5,
            z: 3,
        });
        t
    }

    #[test]
    fn hit_test_respects_z_order() {
        let _t = tree();
        // Overlapping buttons: put B over A.
        let mut t2 = WidgetTree::new();
        t2.button(
            "LOW",
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 30,
            },
            1,
        );
        t2.button(
            "HIGH",
            Rect {
                x: 10,
                y: 10,
                w: 50,
                h: 20,
            },
            5,
        );
        let hit = t2.hit_test(20, 15).unwrap();
        // Topmost = HIGH (z=5).
        assert_eq!(
            t2.widgets.iter().find(|w| w.id() == hit).map(|w| w.z()),
            Some(5)
        );
    }

    #[test]
    fn click_emits_event_and_hover_states() {
        let mut t = tree();
        // Hover over button A (id 1).
        let ev = t.update((10, 10), false, false);
        assert!(ev.is_empty());
        // Press + release → Clicked.
        t.update((10, 10), true, true);
        let ev = t.update((10, 10), false, false);
        assert_eq!(ev, vec![WidgetEvent::Clicked(1)]);
    }

    #[test]
    fn slider_drag_updates_value() {
        let mut t = tree();
        // Slider id was auto-assigned = 4 (3 buttons? no: 2 buttons + slider).
        let slider_id = t
            .widgets
            .iter()
            .find_map(|w| match w {
                Widget::Slider { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        t.update((50, 45), true, true);
        let ev = t.update((90, 45), true, false);
        assert!(
            matches!(ev.as_slice(), [WidgetEvent::Dragged { id, value }] if *id == slider_id && (*value - 0.9).abs() < 0.01)
        );
    }

    #[test]
    fn draw_produces_pixels() {
        let t = tree();
        let mut fb = vec![0u8; 200 * 100 * 4];
        t.draw(&mut fb, 200, 100);
        // Button A area should be non-black now.
        let i = (10 * 200 + 10) * 4;
        assert_ne!(&fb[i..i + 3], &[0, 0, 0]);
    }
}
