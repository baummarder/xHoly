//! Mod executor — plugin lifecycle state machine.
//!
//! Mods register Rust callbacks (`ModHooks`); the executor drives
//! them through Init → Enable → Update → Disable → Unload. Hot-reload
//! re-inits enabled mods whose manifest changed.

use std::collections::HashMap;

use super::mod_manifest::ModManifest;

/// Lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    /// Registered, never initialized.
    Registered,
    /// Initialized (entry ran).
    Initialized,
    /// Enabled (update loop active).
    Enabled,
    /// Disabled but still loaded.
    Disabled,
    /// Unloaded — terminal.
    Unloaded,
}

/// Hook fn types (aliases keep `ModHooks` readable).
pub type InitHook = Box<dyn FnMut(&ModManifest) + Send>;
/// No-arg hook (enable/disable/unload).
pub type SimpleHook = Box<dyn FnMut() + Send>;
/// Per-tick hook (dt seconds).
pub type UpdateHook = Box<dyn FnMut(f64) + Send>;

/// Callbacks a mod provides. All optional; missing hooks are no-ops.
#[derive(Default)]
pub struct ModHooks {
    /// Called once at Init.
    pub on_init: Option<InitHook>,
    /// Called on Enable.
    pub on_enable: Option<SimpleHook>,
    /// Called every frame/update tick.
    pub on_update: Option<UpdateHook>,
    /// Called on Disable.
    pub on_disable: Option<SimpleHook>,
    /// Called at Unload (cleanup).
    pub on_unload: Option<SimpleHook>,
}

/// A mod instance under management.
pub struct ManagedMod {
    pub manifest: ModManifest,
    pub state: LifecycleState,
    /// Monotonic update counter (for tests/diagnostics).
    pub updates: u64,
    hooks: Option<ModHooks>,
}

impl ManagedMod {
    /// Attach hooks (call before init).
    pub fn set_hooks(&mut self, h: ModHooks) {
        self.hooks = Some(h);
    }
}

/// The executor: owns all managed mods.
pub struct ModExecutor {
    mods: HashMap<String, ManagedMod>,
    /// Mods enabled at boot (auto-enable on register).
    pub autoload: Vec<String>,
}

impl ModExecutor {
    /// New executor.
    pub fn new() -> Self {
        Self {
            mods: HashMap::new(),
            autoload: Vec::new(),
        }
    }

    /// Register a mod (state=Registered; auto-init if in autoload).
    pub fn register(&mut self, manifest: ModManifest) -> Result<(), String> {
        let id = manifest.id.clone();
        if self.mods.contains_key(&id) {
            return Err(format!("duplicate mod {id}"));
        }
        self.mods.insert(
            id.clone(),
            ManagedMod {
                manifest,
                state: LifecycleState::Registered,
                updates: 0,
                hooks: None,
            },
        );
        if self.autoload.contains(&id) {
            self.init(&id)?;
        }
        Ok(())
    }

    /// Attach hooks to a registered mod.
    pub fn attach_hooks(&mut self, id: &str, hooks: ModHooks) -> Result<(), String> {
        self.mods
            .get_mut(id)
            .ok_or_else(|| format!("unknown mod {id}"))?
            .set_hooks(hooks);
        Ok(())
    }

    /// Init: Registered → Initialized (runs on_init).
    pub fn init(&mut self, id: &str) -> Result<(), String> {
        let m = self
            .mods
            .get_mut(id)
            .ok_or_else(|| format!("unknown mod {id}"))?;
        if m.state != LifecycleState::Registered {
            return Err(format!("mod {id} not in Registered state"));
        }
        m.state = LifecycleState::Initialized;
        if let Some(h) = m.hooks.as_mut() {
            if let Some(f) = h.on_init.as_mut() {
                let man = m.manifest.clone();
                f(&man);
            }
        }
        Ok(())
    }

    /// Enable: Initialized/Disabled → Enabled (runs on_enable).
    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        let m = self
            .mods
            .get_mut(id)
            .ok_or_else(|| format!("unknown mod {id}"))?;
        match m.state {
            LifecycleState::Initialized | LifecycleState::Disabled => {}
            _ => return Err(format!("mod {id} cannot enable from {:?}", m.state)),
        }
        m.state = LifecycleState::Enabled;
        if let Some(h) = m.hooks.as_mut() {
            if let Some(f) = h.on_enable.as_mut() {
                f();
            }
        }
        Ok(())
    }

    /// Update tick for all enabled mods.
    pub fn update_all(&mut self, dt: f64) {
        for m in self.mods.values_mut() {
            if m.state != LifecycleState::Enabled {
                continue;
            }
            m.updates += 1;
            if let Some(h) = m.hooks.as_mut() {
                if let Some(f) = h.on_update.as_mut() {
                    f(dt);
                }
            }
        }
    }

    /// Disable: Enabled → Disabled (runs on_disable).
    pub fn disable(&mut self, id: &str) -> Result<(), String> {
        let m = self
            .mods
            .get_mut(id)
            .ok_or_else(|| format!("unknown mod {id}"))?;
        if m.state != LifecycleState::Enabled {
            return Err(format!("mod {id} not enabled"));
        }
        m.state = LifecycleState::Disabled;
        if let Some(h) = m.hooks.as_mut() {
            if let Some(f) = h.on_disable.as_mut() {
                f();
            }
        }
        Ok(())
    }

    /// Unload: any non-Unloaded → Unloaded (runs on_unload, drops hooks).
    pub fn unload(&mut self, id: &str) -> Result<(), String> {
        let m = self
            .mods
            .get_mut(id)
            .ok_or_else(|| format!("unknown mod {id}"))?;
        if m.state == LifecycleState::Unloaded {
            return Ok(());
        }
        if let Some(h) = m.hooks.as_mut() {
            if let Some(f) = h.on_unload.as_mut() {
                f();
            }
        }
        m.hooks = None;
        m.state = LifecycleState::Unloaded;
        Ok(())
    }

    /// Hot-reload: swap manifest on a live mod; enabled mods are
    /// disabled then re-enabled (hooks persist).
    pub fn reload(&mut self, id: &str, new_manifest: ModManifest) -> Result<(), String> {
        if !self.mods.contains_key(id) {
            return Err(format!("unknown mod {id}"));
        }
        let was_enabled = self.mods.get(id).map(|m| m.state) == Some(LifecycleState::Enabled);
        if was_enabled {
            self.disable(id)?;
        }
        self.mods.get_mut(id).unwrap().manifest = new_manifest;
        if was_enabled {
            self.enable(id)?;
        }
        Ok(())
    }

    /// Update counter for a mod (diagnostics/tests).
    pub fn update_count(&self, id: &str) -> u64 {
        self.mods.get(id).map(|m| m.updates).unwrap_or(0)
    }

    /// State query.
    pub fn state(&self, id: &str) -> Option<LifecycleState> {
        self.mods.get(id).map(|m| m.state)
    }

    /// All mod ids.
    pub fn ids(&self) -> Vec<&str> {
        self.mods.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ModExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_system::mod_manifest::Version;
    use std::sync::{Arc, Mutex};

    fn man(id: &str) -> ModManifest {
        ModManifest {
            id: id.into(),
            name: id.into(),
            version: Version::new(1, 0, 0),
            authors: vec![],
            description: String::new(),
            dependencies: vec![],
            entry: None,
        }
    }

    #[test]
    fn lifecycle_walkthrough() {
        let mut ex = ModExecutor::new();
        ex.register(man("m")).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        ex.attach_hooks(
            "m",
            ModHooks {
                on_init: Some(Box::new(move |_| l2.lock().unwrap().push("init"))),
                on_enable: Some(Box::new({
                    let l = log.clone();
                    move || l.lock().unwrap().push("enable")
                })),
                on_update: Some(Box::new({
                    let l = log.clone();
                    move |_| l.lock().unwrap().push("update")
                })),
                on_disable: Some(Box::new({
                    let l = log.clone();
                    move || l.lock().unwrap().push("disable")
                })),
                on_unload: Some(Box::new({
                    let l = log.clone();
                    move || l.lock().unwrap().push("unload")
                })),
            },
        )
        .unwrap();
        ex.init("m").unwrap();
        ex.enable("m").unwrap();
        ex.update_all(0.016);
        ex.update_all(0.016);
        ex.disable("m").unwrap();
        ex.unload("m").unwrap();
        let l = log.lock().unwrap();
        assert_eq!(
            l.as_slice(),
            &["init", "enable", "update", "update", "disable", "unload"]
        );
    }

    #[test]
    fn invalid_transitions_rejected() {
        let mut ex = ModExecutor::new();
        ex.register(man("m")).unwrap();
        assert!(ex.enable("m").is_err()); // not initialized
        ex.init("m").unwrap();
        assert!(ex.disable("m").is_err()); // not enabled
        ex.enable("m").unwrap();
        assert_eq!(ex.state("m"), Some(LifecycleState::Enabled));
    }

    #[test]
    fn autoload_inits_on_register() {
        let mut ex = ModExecutor::new();
        ex.autoload.push("m".into());
        ex.register(man("m")).unwrap();
        assert_eq!(ex.state("m"), Some(LifecycleState::Initialized));
    }

    #[test]
    fn reload_enabled_mod() {
        let mut ex = ModExecutor::new();
        ex.register(man("m")).unwrap();
        ex.init("m").unwrap();
        ex.enable("m").unwrap();
        let mut m2 = man("m");
        m2.version = Version::new(2, 0, 0);
        ex.reload("m", m2).unwrap();
        assert_eq!(ex.state("m"), Some(LifecycleState::Enabled));
    }
}
