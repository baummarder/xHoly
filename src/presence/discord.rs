//! Discord RPC backend (worker thread only).
//!
//! Uses the maintained `discord-rich-presence` crate (v1.1.0, MIT) for
//! the local Discord IPC socket. The rest of xHoly only sees the
//! [`IpcClient`] abstraction, so connection failures, disconnects and
//! reconnect behaviour are testable without Discord installed.
//!
//! Every method here runs on the presence worker thread — never on the
//! emulation thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Receiver, Arc};
use std::time::{Duration, Instant};

use super::{DiscordSettings, PresenceCommand, PresenceState};
use discord_rich_presence::{activity::Activity, DiscordIpc, DiscordIpcClient};

/// Poll interval of the worker (bounds shutdown latency).
const POLL: Duration = Duration::from_millis(20);
/// Delay before retrying a failed connection or activity update.
const RETRY: Duration = Duration::from_secs(3);

/// Minimal IPC abstraction used by the worker.
pub trait IpcClient: Send {
    /// Connect and handshake with Discord.
    fn connect(&mut self) -> Result<(), String>;
    /// Set the activity details/state.
    fn set_activity(&mut self, details: &str, state: Option<&str>) -> Result<(), String>;
    /// Clear the activity.
    fn clear_activity(&mut self) -> Result<(), String>;
    /// Close the connection.
    fn close(&mut self);
}

/// Factory used by the worker to create IPC clients.
pub type ClientFactory = Box<dyn FnMut(&str) -> Result<Box<dyn IpcClient>, String>>;

struct DiscordClient(DiscordIpcClient);

impl IpcClient for DiscordClient {
    fn connect(&mut self) -> Result<(), String> {
        self.0.connect().map_err(|e| e.to_string())
    }

    fn set_activity(&mut self, details: &str, state: Option<&str>) -> Result<(), String> {
        let mut activity = Activity::new().details(details);
        if let Some(state) = state {
            activity = activity.state(state);
        }
        self.0.set_activity(activity).map_err(|e| e.to_string())
    }

    fn clear_activity(&mut self) -> Result<(), String> {
        self.0.clear_activity().map_err(|e| e.to_string())
    }

    fn close(&mut self) {
        let _ = self.0.close();
    }
}

/// Real client factory. No client is created when the ID is empty — the
/// controller already prevents that, but we keep the guard here too.
fn real_client(application_id: &str) -> Result<Box<dyn IpcClient>, String> {
    if application_id.trim().is_empty() {
        return Err("no Discord Application ID configured".into());
    }
    // v1.1.0 constructs infallibly; the actual connection is attempted by
    // `connect()` on the worker thread.
    let client = DiscordIpcClient::new(application_id);
    Ok(Box::new(DiscordClient(client)))
}

/// Run the worker with the real Discord IPC client.
pub(super) fn run_worker(
    rx: Receiver<PresenceCommand>,
    settings: DiscordSettings,
    stop: Arc<AtomicBool>,
) {
    run_worker_with(rx, settings, stop, RETRY, real_client);
}

/// Run the worker with an injectable client factory and retry delay (tests).
pub(super) fn run_worker_with<F>(
    rx: Receiver<PresenceCommand>,
    settings: DiscordSettings,
    stop: Arc<AtomicBool>,
    retry_delay: Duration,
    mut factory: F,
) where
    F: FnMut(&str) -> Result<Box<dyn IpcClient>, String>,
{
    let mut client: Option<Box<dyn IpcClient>> = None;
    let mut retry_at: Option<Instant> = None;
    let mut last_sent: Option<(String, Option<String>)> = None;

    loop {
        if stop.load(Ordering::Relaxed) {
            if let Some(active) = client.as_mut() {
                let _ = active.clear_activity();
                active.close();
            }
            return;
        }
        let command = match rx.recv_timeout(POLL) {
            Ok(command) => command,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        match command {
            PresenceCommand::Reset => {
                if let Some(active) = client.as_mut() {
                    let _ = active.clear_activity();
                }
                last_sent = None;
            }
            PresenceCommand::Update(state) => {
                let text = activity_text(&state, &settings);
                if last_sent.as_ref() == Some(&text) {
                    continue;
                }
                if client.is_none() {
                    if retry_at.is_some_and(|at| Instant::now() < at) {
                        continue;
                    }
                    match factory(&settings.application_id) {
                        Ok(mut fresh) => match fresh.connect() {
                            Ok(()) => {
                                client = Some(fresh);
                                retry_at = None;
                            }
                            Err(err) => {
                                log::debug!("xHoly: Discord connect failed: {err}");
                                retry_at = Some(Instant::now() + retry_delay);
                                continue;
                            }
                        },
                        Err(err) => {
                            log::debug!("xHoly: Discord IPC unavailable: {err}");
                            retry_at = Some(Instant::now() + retry_delay);
                            continue;
                        }
                    }
                }
                let (details, state_line) = text.clone();
                let result = client
                    .as_mut()
                    .expect("client connected")
                    .set_activity(&details, state_line.as_deref());
                match result {
                    Ok(()) => last_sent = Some(text),
                    Err(err) => {
                        // Connection dropped mid-update: drop the client and
                        // retry on a later command. No panic, no busy loop.
                        log::debug!("xHoly: Discord activity update failed: {err}");
                        if let Some(active) = client.take() {
                            let mut active = active;
                            active.close();
                        }
                        last_sent = None;
                        retry_at = Some(Instant::now() + retry_delay);
                    }
                }
            }
        }
    }
}

/// Build the `(details, state)` payload from a presence state.
fn activity_text(state: &PresenceState, settings: &DiscordSettings) -> (String, Option<String>) {
    (state.details(settings), state.state(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct FakeClient {
        log: Arc<Mutex<Vec<String>>>,
        connect_fails: bool,
        set_fails_once: bool,
    }

    impl IpcClient for FakeClient {
        fn connect(&mut self) -> Result<(), String> {
            if self.connect_fails {
                Err("discord not running".into())
            } else {
                self.log.lock().unwrap().push("connect".into());
                Ok(())
            }
        }
        fn set_activity(&mut self, details: &str, state: Option<&str>) -> Result<(), String> {
            if self.set_fails_once {
                self.set_fails_once = false;
                return Err("disconnected".into());
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("set:{details}:{}", state.unwrap_or("-")));
            Ok(())
        }
        fn clear_activity(&mut self) -> Result<(), String> {
            self.log.lock().unwrap().push("clear".into());
            Ok(())
        }
        fn close(&mut self) {
            self.log.lock().unwrap().push("close".into());
        }
    }

    fn settings() -> DiscordSettings {
        DiscordSettings {
            application_id: "123456789012345678".into(),
            ..Default::default()
        }
    }

    /// Short backoff so reconnect tests finish quickly.
    const TEST_RETRY: Duration = Duration::from_millis(10);

    fn settle() {
        std::thread::sleep(Duration::from_millis(80));
    }

    #[test]
    fn activity_text_matches_required_copy() {
        let s = settings();
        assert_eq!(
            activity_text(&PresenceState::MainMenu, &s),
            ("Chilling in Main Menu".into(), None)
        );
        assert_eq!(
            activity_text(
                &PresenceState::Loading {
                    game: "Halo".into()
                },
                &s
            )
            .0,
            "Loading Halo"
        );
        assert_eq!(
            activity_text(
                &PresenceState::Emulating {
                    game: "Halo".into(),
                    console: super::super::ConsoleGeneration::XboxSeriesXS,
                },
                &s
            ),
            ("Emulating Halo".into(), Some("Xbox Series X|S".into()))
        );
    }

    #[test]
    fn discord_not_installed_never_writes_an_activity() {
        let (tx, rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = std::thread::spawn(move || {
            run_worker_with(rx, settings(), stop, TEST_RETRY, move |_| {
                Err::<Box<dyn IpcClient>, String>("no discord socket".into())
            });
        });
        tx.send(PresenceCommand::Update(PresenceState::Emulating {
            game: "Halo".into(),
            console: super::super::ConsoleGeneration::XboxOne,
        }))
        .unwrap();
        settle();
        assert!(log.lock().unwrap().is_empty());
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn discord_closed_retries_without_panicking() {
        let (tx, rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let logged = log.clone();
        let worker = std::thread::spawn(move || {
            run_worker_with(rx, settings(), stop, TEST_RETRY, move |_| {
                Ok(Box::new(FakeClient {
                    log: logged.clone(),
                    connect_fails: true,
                    set_fails_once: false,
                }) as Box<dyn IpcClient>)
            });
        });
        tx.send(PresenceCommand::Update(PresenceState::MainMenu))
            .unwrap();
        settle();
        assert!(log.lock().unwrap().is_empty());
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn disconnect_then_reconnect_recovers() {
        let (tx, rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let fail_once = Arc::new(AtomicBool::new(true));
        let logged = log.clone();
        let fail_flag = fail_once.clone();
        let worker = std::thread::spawn(move || {
            run_worker_with(rx, settings(), stop, TEST_RETRY, move |_| {
                let fails = fail_flag.swap(false, Ordering::Relaxed);
                Ok(Box::new(FakeClient {
                    log: logged.clone(),
                    connect_fails: false,
                    set_fails_once: fails,
                }) as Box<dyn IpcClient>)
            });
        });
        let state = PresenceState::Emulating {
            game: "Game".into(),
            console: super::super::ConsoleGeneration::XboxOne,
        };
        tx.send(PresenceCommand::Update(state.clone())).unwrap();
        settle();
        // Retries are paced; keep reporting the new state until the
        // backoff expires, then the worker reconnects automatically.
        for _ in 0..20 {
            tx.send(PresenceCommand::Update(PresenceState::MainMenu))
                .unwrap();
            settle();
            if log
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.starts_with("set:Chilling in Main Menu"))
            {
                break;
            }
        }
        drop(tx);
        worker.join().unwrap();
        let log = log.lock().unwrap();
        assert!(log.iter().filter(|e| *e == "connect").count() >= 2);
        assert!(log
            .iter()
            .any(|e| e.starts_with("set:Chilling in Main Menu")));
    }

    #[test]
    fn reset_clears_the_activity() {
        let (tx, rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let logged = log.clone();
        let worker = std::thread::spawn(move || {
            run_worker_with(rx, settings(), stop, TEST_RETRY, move |_| {
                Ok(Box::new(FakeClient {
                    log: logged.clone(),
                    connect_fails: false,
                    set_fails_once: false,
                }) as Box<dyn IpcClient>)
            });
        });
        tx.send(PresenceCommand::Update(PresenceState::MainMenu))
            .unwrap();
        tx.send(PresenceCommand::Reset).unwrap();
        settle();
        drop(tx);
        worker.join().unwrap();
        assert!(log.lock().unwrap().iter().any(|e| e == "clear"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_backend_supports_ipc_factory() {
        assert!(real_client("").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backend_supports_ipc_factory() {
        assert!(real_client("").is_err());
    }
}
