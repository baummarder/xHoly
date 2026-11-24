//! Discord Rich Presence integration (shared interface + controller).
//!
//! The emulator core and GUI only depend on the backend-agnostic
//! [`PresenceSink`] trait. The Discord IPC client lives in [`discord`] and
//! is never called from the emulation thread: [`Presence::update`] and
//! [`Presence::reset`] push commands into a bounded channel and return
//! immediately. RPC connection, retries and activity updates run on a
//! dedicated worker thread.
//!
//! No Discord Application ID is hardcoded. An empty `application_id`
//! disables the connection entirely; see `docs/discord-rich-presence.md`.

pub mod discord;

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// Application name shown by Discord (must match the Developer Portal app).
pub const APPLICATION_NAME: &str = "xHoly";
/// Main-menu detail text.
pub const MAIN_MENU_DETAILS: &str = "Chilling in Main Menu";
/// Maximum number of queued presence commands.
const QUEUE_CAPACITY: usize = 16;
/// Console generation displayed in the presence `state` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleGeneration {
    /// Xbox One (Jaguar).
    XboxOne,
    /// Xbox Series X|S (Zen 2).
    XboxSeriesXS,
}

impl ConsoleGeneration {
    /// Human-readable label sent to Discord.
    pub fn label(self) -> &'static str {
        match self {
            Self::XboxOne => "Xbox One",
            Self::XboxSeriesXS => "Xbox Series X|S",
        }
    }
}

/// A presence state reported by the GUI or emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceState {
    /// No game active.
    MainMenu,
    /// A game image is being loaded.
    Loading { game: String },
    /// A game is running.
    Emulating {
        game: String,
        console: ConsoleGeneration,
    },
}

/// Discord presence settings (persisted as the `[discord]` config section).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordSettings {
    /// Master switch. Defaults to `true`; an empty ID still prevents any
    /// RPC connection.
    pub enabled: bool,
    /// Discord Application ID from the Developer Portal. Empty = placeholder.
    pub application_id: String,
    /// Whether the display name of the current game is included.
    pub show_game_name: bool,
    /// Whether the console generation is included.
    pub show_generation: bool,
}

impl Default for DiscordSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            application_id: String::new(),
            show_game_name: true,
            show_generation: true,
        }
    }
}

impl DiscordSettings {
    /// Whether an RPC connection may be attempted.
    ///
    /// Disabled mode or a missing Application ID means **no connection at
    /// all** — no socket, no IPC handshake, no activity payload.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.application_id.trim().is_empty()
    }

    /// Reduce a game title to a safe display name.
    ///
    /// Local paths, separators and user-specific components are stripped;
    /// only the visible title is forwarded to Discord.
    pub fn display_name(&self, raw: &str, fallback: &str) -> String {
        if !self.show_game_name {
            return fallback.to_string();
        }
        let trimmed = raw.trim();
        let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
        let name = if name.is_empty() { fallback } else { name };
        name.chars().take(80).collect()
    }
}

/// Backend-agnostic presence sink implemented by both the GUI and core.
pub trait PresenceSink: Send + Sync + std::fmt::Debug {
    /// Report a state change. Duplicate states are ignored by implementations.
    fn update(&self, state: PresenceState);
    /// Clear the activity and return to the main-menu state.
    fn reset(&self);
}

/// No-op sink (used when the integration is disabled or in tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopPresence;

impl PresenceSink for NoopPresence {
    fn update(&self, _state: PresenceState) {}
    fn reset(&self) {}
}

/// Commands sent to the RPC worker thread.
#[derive(Debug, Clone)]
enum PresenceCommand {
    Update(PresenceState),
    Reset,
}

/// A non-blocking presence controller.
///
/// Cloning shares the same command queue and dedupe state. Dropping the
/// last handle asks the worker to clear the activity and shut down.
pub struct Presence {
    tx: SyncSender<PresenceCommand>,
    last: Arc<Mutex<Option<PresenceState>>>,
    settings: DiscordSettings,
    stop: Arc<AtomicBool>,
    worker: Option<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl std::fmt::Debug for Presence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Presence")
            .field("settings", &self.settings)
            .field("active", &self.settings.is_active())
            .finish_non_exhaustive()
    }
}

impl Presence {
    /// Create a controller. With no usable Application ID this builds a
    /// sink that never opens an IPC connection.
    pub fn new(settings: DiscordSettings) -> Self {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = if settings.is_active() {
            let worker_stop = stop.clone();
            let worker_settings = settings.clone();
            Some(Mutex::new(Some(std::thread::spawn(move || {
                discord::run_worker(rx, worker_settings, worker_stop);
            }))))
        } else {
            None
        };
        Self {
            tx,
            last: Arc::new(Mutex::new(None)),
            settings,
            stop,
            worker,
        }
    }

    /// Whether this controller may talk to Discord.
    pub fn is_active(&self) -> bool {
        self.settings.is_active()
    }

    /// Report a state without ever blocking on RPC.
    ///
    /// Repeated identical states are dropped before entering the queue,
    /// so `tick_guest()` can call this every frame safely.
    pub fn update(&self, state: PresenceState) {
        {
            let mut last = self.last.lock().expect("presence dedupe lock poisoned");
            if last.as_ref() == Some(&state) {
                return;
            }
            *last = Some(state.clone());
        }
        self.send(PresenceCommand::Update(state));
    }

    /// Clear the Discord activity (e.g. on game end).
    pub fn reset(&self) {
        self.last
            .lock()
            .expect("presence dedupe lock poisoned")
            .take();
        self.send(PresenceCommand::Reset);
    }

    fn send(&self, command: PresenceCommand) {
        match self.tx.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                log::debug!("xHoly: presence queue full, dropping update");
            }
            Err(TrySendError::Disconnected(_)) => {
                log::debug!("xHoly: presence worker gone");
            }
        }
    }
}

impl PresenceSink for Presence {
    fn update(&self, state: PresenceState) {
        Presence::update(self, state);
    }

    fn reset(&self) {
        Presence::reset(self);
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.try_send(PresenceCommand::Reset);
        if let Some(worker) = &self.worker {
            if let Some(handle) = worker.lock().unwrap().take() {
                // The worker polls WORKER_TICK, so this join is bounded.
                let _ = handle.join();
            }
        }
    }
}

/// Load the `[discord]` section of the usual xHoly config file.
///
/// Missing files, malformed TOML and unknown fields all fall back to the
/// documented defaults — configuration must never break startup.
pub fn load_settings(path: Option<&Path>) -> DiscordSettings {
    let path = path
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(crate::config::Config::default_path);
    let Ok(value) = crate::config::load_file(&path) else {
        return DiscordSettings::default();
    };
    let section = value.get("discord").cloned().unwrap_or_default();
    serde_json::from_value(section).unwrap_or_default()
}

impl PresenceState {
    /// Detail text shown in Discord.
    pub fn details(&self, settings: &DiscordSettings) -> String {
        match self {
            Self::MainMenu => MAIN_MENU_DETAILS.to_string(),
            Self::Loading { game } => {
                format!("Loading {}", settings.display_name(game, "a game"))
            }
            Self::Emulating { game, .. } => {
                format!("Emulating {}", settings.display_name(game, "a game"))
            }
        }
    }

    /// Optional state line (console generation).
    pub fn state(&self, settings: &DiscordSettings) -> Option<String> {
        match (self, settings.show_generation) {
            (Self::Emulating { console, .. }, true) => Some(console.label().to_string()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    /// Build a controller whose queue can be inspected by tests (no worker).
    fn test_presence(settings: DiscordSettings) -> (Presence, Receiver<PresenceCommand>) {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        let presence = Presence {
            tx,
            last: Arc::new(Mutex::new(None)),
            settings,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        (presence, rx)
    }

    fn active_settings() -> DiscordSettings {
        DiscordSettings {
            application_id: "123456789012345678".into(),
            ..Default::default()
        }
    }

    #[test]
    fn main_menu_is_the_initial_state() {
        let presence = Presence::new(active_settings());
        presence.update(PresenceState::MainMenu);
        assert_eq!(
            *presence.last.lock().unwrap(),
            Some(PresenceState::MainMenu)
        );
    }

    #[test]
    fn game_start_switch_end_and_shutdown() {
        let presence = Presence::new(active_settings());
        presence.update(PresenceState::Loading {
            game: "Game A".into(),
        });
        presence.update(PresenceState::Emulating {
            game: "Game A".into(),
            console: ConsoleGeneration::XboxSeriesXS,
        });
        presence.update(PresenceState::Emulating {
            game: "Game B".into(),
            console: ConsoleGeneration::XboxOne,
        });
        assert!(matches!(
            *presence.last.lock().unwrap(),
            Some(PresenceState::Emulating { ref game, .. }) if game == "Game B"
        ));
        presence.reset();
        assert_eq!(*presence.last.lock().unwrap(), None);
        // Shutdown (drop) must not panic even without Discord running.
    }

    #[test]
    fn duplicate_updates_are_not_queued() {
        let (presence, rx) = test_presence(active_settings());
        let state = PresenceState::Emulating {
            game: "Game".into(),
            console: ConsoleGeneration::XboxSeriesXS,
        };
        presence.update(state.clone());
        let first = rx.try_recv().unwrap();
        presence.update(state);
        assert!(rx.try_recv().is_err(), "duplicate must not enqueue");
        assert!(matches!(first, PresenceCommand::Update(_)));
    }

    #[test]
    fn disabled_presence_never_connects() {
        let settings = DiscordSettings {
            enabled: false,
            application_id: "123".into(),
            ..Default::default()
        };
        let presence = Presence::new(settings);
        assert!(!presence.is_active());
        assert!(presence.worker.is_none());
        presence.update(PresenceState::MainMenu);
    }

    #[test]
    fn missing_application_id_is_a_safe_placeholder() {
        let presence = Presence::new(DiscordSettings::default());
        assert!(!presence.is_active());
        assert!(presence.worker.is_none());
    }

    #[test]
    fn only_display_names_are_forwarded() {
        let settings = DiscordSettings::default();
        assert_eq!(
            settings.display_name(r"C:\Users\alice\Games\Halo Infinite.exe", "Unknown"),
            "Halo Infinite.exe"
        );
        let hidden = DiscordSettings {
            show_game_name: false,
            ..Default::default()
        };
        assert_eq!(hidden.display_name("Halo Infinite", "a game"), "a game");
    }

    #[test]
    fn state_text_matches_the_required_copy() {
        assert_eq!(
            PresenceState::MainMenu.details(&DiscordSettings::default()),
            MAIN_MENU_DETAILS
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_defaults_are_available() {
        assert!(DiscordSettings::default().enabled);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_defaults_are_available() {
        assert!(DiscordSettings::default().enabled);
    }
}
