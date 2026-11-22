//! Debug menu — toggleable overlay menu tree (F1-style).

/// A menu entry (leaf or submenu).
#[derive(Debug, Clone)]
pub enum MenuItem {
    /// Action with label.
    Action { label: String, id: String },
    /// Toggleable bool setting.
    Toggle {
        label: String,
        id: String,
        value: bool,
    },
    /// Submenu.
    Submenu { label: String, items: Vec<MenuItem> },
    /// Separator.
    Separator,
}

/// The debug menu state.
#[derive(Debug, Default)]
pub struct DebugMenu {
    /// Root items.
    pub items: Vec<MenuItem>,
    /// Whether the menu is visible.
    pub visible: bool,
    /// Selected path (indices into the tree) for keyboard nav.
    pub cursor: Vec<usize>,
}

impl DebugMenu {
    /// New menu with default items.
    pub fn new() -> Self {
        Self {
            items: default_items(),
            visible: false,
            cursor: vec![0],
        }
    }

    /// Toggle visibility (F1 hotkey hook).
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Find an item by id (depth-first).
    pub fn find_mut(&mut self, id: &str) -> Option<&mut MenuItem> {
        fn walk<'a>(items: &'a mut [MenuItem], id: &str) -> Option<&'a mut MenuItem> {
            for it in items.iter_mut() {
                match it {
                    MenuItem::Action { id: i, .. } | MenuItem::Toggle { id: i, .. } => {
                        if i == id {
                            return Some(it);
                        }
                    }
                    MenuItem::Submenu { items: sub, .. } => {
                        if let Some(f) = walk(sub, id) {
                            return Some(f);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        walk(&mut self.items, id)
    }

    /// Activate the item under the cursor; returns its id if Action/Toggle.
    pub fn activate_cursor(&mut self) -> Option<String> {
        // Navigate to the parent list of the cursor's last index.
        let mut items = &mut self.items;
        for &idx in self.cursor.iter().take(self.cursor.len().saturating_sub(1)) {
            match items.get_mut(idx)? {
                MenuItem::Submenu { items: sub, .. } => items = sub,
                _ => return None,
            }
        }
        let last = *self.cursor.last()?;
        match items.get_mut(last)? {
            MenuItem::Action { id, .. } => Some(id.clone()),
            MenuItem::Toggle { id, value, .. } => {
                *value = !*value;
                Some(id.clone())
            }
            _ => None,
        }
    }
}

fn default_items() -> Vec<MenuItem> {
    vec![
        MenuItem::Toggle {
            label: "FPS overlay".into(),
            id: "fps".into(),
            value: true,
        },
        MenuItem::Toggle {
            label: "Wireframe".into(),
            id: "wireframe".into(),
            value: false,
        },
        MenuItem::Action {
            label: "Dump memory".into(),
            id: "memdump".into(),
        },
        MenuItem::Separator,
        MenuItem::Submenu {
            label: "GPU".into(),
            items: vec![MenuItem::Action {
                label: "Inspect shaders".into(),
                id: "shader_inspect".into(),
            }],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_visibility() {
        let mut m = DebugMenu::new();
        assert!(!m.visible);
        m.toggle();
        assert!(m.visible);
        m.toggle();
        assert!(!m.visible);
    }

    #[test]
    fn find_and_toggle_item() {
        let mut m = DebugMenu::new();
        let id = m.activate_cursor(); // cursor at [0] → fps toggle
        assert_eq!(id.as_deref(), Some("fps"));
        // fps was true → now false
        if let Some(MenuItem::Toggle { value, .. }) = m.find_mut("fps") {
            assert!(!*value);
        } else {
            panic!("fps not found");
        }
    }
}
