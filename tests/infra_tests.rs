//! Phase 11.5 infrastructure integration tests.
//!
//! Cross-module flows plus stress and timeout/deadlock coverage
//! per the phase spec. Everything runs headless.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use xholy::benchmark::{BenchmarkRunner, Report};
use xholy::cheat_engine::cheat_list::{
    CheatEntry, CheatValueSerde, MemoryRegionSerde, TriggerKindSerde,
};
use xholy::cheat_engine::{CheatManager, CheatValue, MemoryRegion};
use xholy::cloud::cloud_client::{CloudClient, HttpResponse, Request};
use xholy::cloud::{CloudSync, ConflictResolution, SyncDirection, Transport};
use xholy::config::{load_effective, DEFAULTS_TOML};
use xholy::debug_menu::debug_panel::fps_panel;
use xholy::debug_menu::{DebugMenu, PanelKind};
use xholy::input::{KeyEvent, KeyboardState, PressEvent, PressState};
use xholy::logging::{Level, Logger, Record, Sink};
use xholy::mod_system::{LifecycleState, ModExecutor, ModHooks, ModManifest, Version};
use xholy::multiplayer::{ConsistencyMode, Lobby, ReplicatedState, SyncEngine};
use xholy::network::{
    HttpMethod, HttpRequest, HttpResponse as NetResponse, LoopbackTransport, NetCache,
};
use xholy::save::{SaveManager, SaveMeta, SaveOutcome, SavePayload};
use xholy::shader_cache::{CacheKey, ShaderCache};
use xholy::storage::{DriveKind, SsdCache, StorageDevice, StorageManager};
use xholy::telemetry::{
    EventKind, MemorySender, TelemetryCollector, TelemetryEvent, TelemetryFilter,
};
use xholy::utils::utils_thread::{Permit, Semaphore};
use xholy::utils::{fnv1a, human_bytes, slugify};

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("xholy_infra_{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ---------- config ----------

#[test]
fn config_loader_precedence_end_to_end() {
    let dir = tmpdir("cfg");
    let f = dir.join("x.toml");
    std::fs::write(&f, "[debug]\nshow_fps = false\n").unwrap();
    let v = load_effective(
        DEFAULTS_TOML,
        Some(&f),
        &vec![("debug.show_memory".to_string(), serde_json::json!(false))],
    )
    .unwrap();
    assert_eq!(v["debug"]["show_fps"], serde_json::json!(false)); // file layer
    assert_eq!(v["debug"]["show_memory"], serde_json::json!(false)); // CLI layer
    assert_eq!(v["debug"]["hotkey"], serde_json::json!("F1")); // default
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- logging ----------

/// In-memory sink for assertions.
#[derive(Clone, Default)]
struct MemSink {
    records: Arc<Mutex<Vec<Record>>>,
}
impl Sink for MemSink {
    fn write(&mut self, rec: &Record) {
        self.records.lock().unwrap().push(rec.clone());
    }
}

#[test]
fn logging_levels_and_filters() {
    let sink = MemSink::default();
    let seen = sink.records.clone();
    let log = Logger::new()
        .with_min_level(Level::Info)
        .add_sink(Arc::new(Mutex::new(sink)));
    log.info("m", "hello");
    log.trace("m", "dropped-by-floor");
    log.error("m", "boom");
    let recs = seen.lock().unwrap();
    assert_eq!(recs.len(), 2);
    assert!(recs
        .iter()
        .any(|r| r.message == "boom" && r.level == Level::Error));
}

// ---------- input ----------

#[test]
fn input_keyboard_and_press_edges() {
    let mut kb = KeyboardState::default();
    kb.apply(&KeyEvent::new("Space", true));
    assert!(kb.is_down("Space"));
    kb.apply(&KeyEvent::new("Space", false));
    assert!(!kb.is_down("Space"));

    let mut ps = PressState::Up;
    assert_eq!(ps.update(true, false), Some(PressEvent::Pressed));
    assert_eq!(ps.update(true, false), Some(PressEvent::Held));
    assert_eq!(ps.update(false, false), Some(PressEvent::Released));
}

// ---------- save (stress: all slots) ----------

fn payload(slot: u8) -> SavePayload {
    SavePayload {
        meta: SaveMeta {
            name: format!("s{slot}"),
            title_id: "GAME".into(),
            playtime_secs: slot as u64 * 60,
            ..Default::default()
        },
        state: vec![slot; 64],
    }
}

#[test]
fn save_all_slots_roundtrip() {
    let dir = tmpdir("saves");
    let mut mgr = SaveManager::new(dir.clone());
    for slot in 1..=8u8 {
        assert!(matches!(
            mgr.save("GAME", slot, &payload(slot)),
            SaveOutcome::Ok(_)
        ));
    }
    mgr.scan("GAME");
    for slot in 1..=8u8 {
        let out = mgr.load_payload("GAME", slot).unwrap();
        assert_eq!(out.state[0], slot);
        assert_eq!(out.state.len(), 64);
        assert_eq!(out.meta.name, format!("s{slot}"));
    }
    assert!(matches!(
        mgr.load("GAME", 99),
        SaveOutcome::Empty(99) | SaveOutcome::Err(_)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- mods ----------

#[test]
fn mod_executor_full_lifecycle() {
    let ran = Arc::new(AtomicBool::new(false));
    let ran2 = ran.clone();
    let mut ex = ModExecutor::new();
    ex.register(ModManifest {
        id: "infra.test".into(),
        name: "Test".into(),
        version: Version::new(1, 0, 0),
        authors: vec![],
        description: String::new(),
        dependencies: vec![],
        entry: None,
    })
    .unwrap();
    ex.attach_hooks(
        "infra.test",
        ModHooks {
            on_init: Some(Box::new(move |_| ran2.store(true, Ordering::SeqCst))),
            ..Default::default()
        },
    )
    .unwrap();
    ex.init("infra.test").unwrap();
    ex.enable("infra.test").unwrap();
    assert_eq!(ex.state("infra.test"), Some(LifecycleState::Enabled));
    ex.update_all(0.016);
    assert_eq!(ex.update_count("infra.test"), 1);
    ex.disable("infra.test").unwrap();
    ex.unload("infra.test").unwrap();
    assert!(ran.load(Ordering::SeqCst));
}

// ---------- cheats ----------

#[test]
fn cheat_engine_writes_guest_ram() {
    let mut cm = CheatManager::new(0x1000, 512);
    cm.install(
        "G",
        vec![CheatEntry {
            name: "ammo".into(),
            description: String::new(),
            address: 0x1100,
            region: MemoryRegionSerde::Ram,
            value: CheatValueSerde::U32(9999),
            trigger: TriggerKindSerde::Always,
            enabled: true,
            hotkey: None,
        }],
    );
    cm.enable("G", 0);
    assert_eq!(cm.tick("G", |_| false, None), 1);
    assert_eq!(
        cm.read_value(MemoryRegion::Ram, 0x1100),
        Some(CheatValue::U32(9999))
    );
}

// ---------- benchmark + report ----------

#[test]
fn benchmark_runner_and_report() {
    let r = BenchmarkRunner::new();
    let mut rep = Report::new("infra bench");
    rep.add(r.cpu(&[0x5Au8; 4096], 50));
    rep.add(r.memory(32 * 1024, 10));
    let md = xholy::benchmark::render_markdown(&rep);
    assert!(md.contains("Cpu"));
    assert!(md.contains("Memory"));
}

// ---------- debug ----------

#[test]
fn debug_menu_and_panels() {
    let mut menu = DebugMenu::new();
    menu.toggle();
    assert!(menu.visible);
    let p = fps_panel(60.0, 16.7, 20.0);
    assert_eq!(p.kind, PanelKind::Fps);
}

// ---------- cloud (in-memory transport) ----------

#[derive(Default)]
struct MemCloud {
    store: Mutex<std::collections::HashMap<String, Vec<u8>>>,
}
impl Transport for MemCloud {
    fn send(&self, req: &Request) -> Result<HttpResponse, xholy::cloud::CloudError> {
        let key = req.url.clone();
        match req.method.as_str() {
            "GET" => Ok(self
                .store
                .lock()
                .unwrap()
                .get(&key)
                .map(|b| HttpResponse {
                    status: 200,
                    body: b.clone(),
                })
                .unwrap_or(HttpResponse {
                    status: 404,
                    body: vec![],
                })),
            "PUT" => {
                self.store.lock().unwrap().insert(key, req.body.clone());
                Ok(HttpResponse {
                    status: 200,
                    body: vec![],
                })
            }
            _ => Ok(HttpResponse {
                status: 405,
                body: vec![],
            }),
        }
    }
}

#[test]
fn cloud_sync_upload_then_noop() {
    let client =
        CloudClient::new("https://cloud.test", MemCloud::default()).with_auth(|| Ok("tok".into()));
    let mut sync = CloudSync::new(client);
    let d = sync
        .sync_title("G1", 1, b"save", ConflictResolution::Local)
        .unwrap();
    assert_eq!(d, SyncDirection::Upload);
    let d2 = sync
        .sync_title("G1", 1, b"save", ConflictResolution::Local)
        .unwrap();
    assert_eq!(d2, SyncDirection::Noop);
}

// ---------- shader cache ----------

#[test]
fn shader_cache_persists_across_instances() {
    let dir = tmpdir("shaders");
    {
        let mut c = ShaderCache::with_dir(&dir);
        let k = CacheKey::from_source(b"dxbc-bytes", "dxbc", b"layout-v1");
        c.put(&k, b"compiled");
    }
    let mut c2 = ShaderCache::with_dir(&dir);
    let k2 = CacheKey::from_source(b"dxbc-bytes", "dxbc", b"layout-v1");
    assert_eq!(c2.get(&k2).unwrap(), b"compiled");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- network ----------

#[test]
fn network_offline_cache_serves() {
    let t = LoopbackTransport::new();
    t.push(
        HttpMethod::Get,
        "http://api.test/data",
        NetResponse {
            status: 200,
            headers: Default::default(),
            body: b"payload".to_vec(),
        },
    );
    let mut nc = NetCache::new(t);
    let r = nc.send(&HttpRequest::get("http://api.test/data")).unwrap();
    assert_eq!(r.body, b"payload");
    nc.offline = true;
    let r2 = nc.send(&HttpRequest::get("http://api.test/data")).unwrap();
    assert_eq!(r2.body, b"payload");
}

// ---------- telemetry ----------

#[test]
fn telemetry_filter_and_batch() {
    let sender = MemorySender::new();
    let mut filter = TelemetryFilter::new();
    filter.deny_keys(&["username"]);
    let mut c = TelemetryCollector::new(filter.predicate(), sender.clone());
    c.batch_size = 2;
    c.record(
        TelemetryEvent::new(EventKind::FeatureUse, "0.1.0", "windows").attr("username", "admin"), // dropped by filter
    );
    c.record(TelemetryEvent::new(EventKind::Crash, "0.1.0", "windows"));
    c.record(TelemetryEvent::new(
        EventKind::SessionEnd,
        "0.1.0",
        "windows",
    ));
    c.flush();
    assert_eq!(c.dropped, 1);
    assert_eq!(c.sent, 2);
    assert_eq!(sender.total_events(), 2);
}

// ---------- multiplayer ----------

#[test]
fn multiplayer_lobby_and_sync_rules() {
    let mut lobby = Lobby::new("room", 4);
    lobby.join(1, "host").unwrap();
    lobby.join(2, "client").unwrap();
    lobby.set_ready(1, true).unwrap();
    lobby.set_ready(2, true).unwrap();
    lobby.start(1).unwrap();

    let mut state = ReplicatedState::new();
    state.set(1, "score", vec![10], 5);
    let engine = SyncEngine::new(ConsistencyMode::ClientServer, 1);
    assert!(engine.accept_write(1, "score"));
    assert!(!engine.accept_write(2, "score"));
    assert!(engine.accept_write(2, "in:fire"));
    let _ = state.hash();
}

// ---------- storage ----------

#[test]
fn storage_devices_and_ssd_cache() {
    let mut mgr = StorageManager::new();
    mgr.add(StorageDevice::new("nvme", DriveKind::Nvme, 1000));
    mgr.add(StorageDevice::new("hdd", DriveKind::Hdd, 4000));
    assert_eq!(mgr.names().len(), 2);
    assert_eq!(mgr.fastest().unwrap().name, "nvme");

    let mut cache = SsdCache::new(4096, 4);
    let d = cache.read(0, 8192, |o, l| vec![(o % 256) as u8; l]);
    assert_eq!(d.len(), 8192);
}

// ---------- utils ----------

#[test]
fn utils_hash_and_strings() {
    assert_eq!(fnv1a(b"xholy"), fnv1a(b"xholy"));
    assert_eq!(slugify("Hello World"), "hello-world");
    assert!(human_bytes(1024 * 1024).contains("MiB"));
}

// ---------- stress: parallel saves ----------

#[test]
fn stress_parallel_save_slots() {
    let dir = tmpdir("stress");
    let dir2 = dir.clone();
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let d = dir2.clone();
            std::thread::spawn(move || {
                let mut mgr = SaveManager::new(d);
                let title = format!("G{t}");
                for slot in 1..=8u8 {
                    let p = SavePayload {
                        meta: SaveMeta {
                            title_id: title.clone(),
                            ..Default::default()
                        },
                        state: vec![t as u8; 128],
                    };
                    let _ = mgr.save(&title, slot, &p);
                    let _ = mgr.load_payload(&title, slot);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // Each title dir should hold 8 slot files.
    let g0 = dir.join("G0");
    let count = std::fs::read_dir(&g0)
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert!(count >= 8, "expected >=8 saves in G0, got {count}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- timeout / deadlock detection ----------

#[test]
fn timeout_detection_channel_based() {
    // If the worker deadlocks, recv_timeout errors → test fails.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        let _ = tx.send(42);
    });
    let v = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(v, 42);
}

#[test]
fn semaphore_no_deadlock_under_contention() {
    let sem = Arc::new(Semaphore::new(1));
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let s = Arc::clone(&sem);
        let c = Arc::clone(&counter);
        handles.push(std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if Permit::try_new(&s).is_some() {
                    c.fetch_add(1, Ordering::SeqCst);
                    break;
                }
                if Instant::now() > deadline {
                    panic!("semaphore deadlock");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(counter.load(Ordering::SeqCst), 4);
}
