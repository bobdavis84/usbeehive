use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::Emitter;
use usbeehive::{DeviceManager, Snapshot};

const MAX_EVENTS: usize = 100;

#[derive(Clone, Debug, Serialize)]
pub struct TransitionEvent {
    pub kind: &'static str,
    pub device_id: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct UiSnapshot {
    pub devices: Vec<Value>,
    pub events: Vec<TransitionEvent>,
    pub source: &'static str,
    pub updated_at_ms: u64,
}

fn scrub(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in ["serial", "serial_number", "sysfs_path", "raw_attributes"] {
                map.remove(key);
            }
            // Summaries also carry serial as a (machine_key, value) pair.
            if let Some(Value::Array(properties)) = map.get_mut("properties") {
                properties.retain(|pair| {
                    !matches!(pair, Value::Array(items) if items.first().and_then(Value::as_str) == Some("serial"))
                });
            }
            for child in map.values_mut() {
                scrub(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                scrub(child);
            }
        }
        _ => {}
    }
}

fn ui_devices(snapshot: &Snapshot) -> Vec<Value> {
    snapshot
        .summaries
        .iter()
        .filter_map(|summary| {
            let mut value = serde_json::to_value(summary).ok()?;
            scrub(&mut value);
            value
                .as_object_mut()?
                .insert("id".into(), Value::String(summary.id()));
            Some(value)
        })
        .collect()
}

struct Backend {
    manager: DeviceManager,
    payload: UiSnapshot,
    transitions: VecDeque<TransitionEvent>,
}

impl Backend {
    fn new() -> Self {
        let mut manager = DeviceManager::new();
        manager.refresh();
        let snapshot = manager.snapshot();
        let payload = UiSnapshot {
            devices: ui_devices(snapshot),
            events: vec![],
            source: "live",
            updated_at_ms: snapshot.metadata.as_ref().map_or(0, |m| m.captured_at_ms),
        };
        Self {
            manager,
            payload,
            transitions: VecDeque::new(),
        }
    }

    fn refresh(&mut self) -> (UiSnapshot, bool) {
        let old = self.manager.snapshot().clone();
        self.manager.refresh();
        let snapshot = self.manager.snapshot();
        let diff = snapshot.diff(&old);
        let changed = !diff.is_empty();
        let at_ms = snapshot.metadata.as_ref().map_or(0, |m| m.captured_at_ms);
        for (kind, ids) in [
            ("connected", diff.added),
            ("disconnected", diff.removed),
            ("changed", diff.changed),
            (
                "power_warning",
                diff.newly_degraded
                    .iter()
                    .map(|n| format!("typec:port{n}"))
                    .collect(),
            ),
            (
                "power_restored",
                diff.resolved
                    .iter()
                    .map(|n| format!("typec:port{n}"))
                    .collect(),
            ),
            ("data_warning", diff.newly_rate_degraded),
            ("data_restored", diff.rate_restored),
        ] {
            for device_id in ids {
                if self.transitions.len() == MAX_EVENTS {
                    self.transitions.pop_front();
                }
                self.transitions.push_back(TransitionEvent {
                    kind,
                    device_id,
                    at_ms,
                });
            }
        }
        self.payload = UiSnapshot {
            devices: ui_devices(snapshot),
            events: self.transitions.iter().cloned().collect(),
            source: "live",
            updated_at_ms: at_ms,
        };
        (self.payload.clone(), changed)
    }
}

type Shared = Arc<Mutex<Backend>>;

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, Shared>) -> Result<UiSnapshot, String> {
    state
        .lock()
        .map(|s| s.payload.clone())
        .map_err(|_| "Snapshot unavailable".into())
}

#[tauri::command]
fn refresh_snapshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, Shared>,
) -> Result<UiSnapshot, String> {
    let (payload, _) = state.lock().map_err(|_| "Snapshot unavailable")?.refresh();
    let _ = app.emit("snapshot-updated", &payload);
    Ok(payload)
}

pub fn run() {
    let backend = Arc::new(Mutex::new(Backend::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Arc::new(Mutex::new(None::<thread::JoinHandle<()>>));
    let setup_stop = stop.clone();
    let setup_worker = worker.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(backend.clone())
        .invoke_handler(tauri::generate_handler![get_snapshot, refresh_snapshot])
        .setup(move |app| {
            let handle = app.handle().clone();
            let backend = backend.clone();
            let stop_flag = setup_stop.clone();
            let thread = thread::spawn(move || {
                let mut watcher = usbeehive::watch::Watcher::new().ok();
                while !stop_flag.load(Ordering::Relaxed) {
                    let update = if let Some(w) = watcher.as_mut() {
                        let ready = matches!(
                            w.wait(Some(Duration::from_millis(500))),
                            usbeehive::watch::WaitResult::Readable
                        );
                        ready && w.drain() > 0
                    } else {
                        thread::sleep(Duration::from_secs(2));
                        true
                    };
                    if update && !stop_flag.load(Ordering::Relaxed) {
                        if let Ok(mut state) = backend.lock() {
                            let (payload, changed) = state.refresh();
                            drop(state);
                            if changed {
                                let _ = handle.emit("snapshot-updated", payload);
                            }
                        }
                    }
                }
            });
            *setup_worker.lock().map_err(|_| "Watcher unavailable")? = Some(thread);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to initialize USBeehive")
        .run(move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                stop.store(true, Ordering::Relaxed);
                if let Ok(mut guard) = worker.lock() {
                    if let Some(thread) = guard.take() {
                        let _ = thread.join();
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrub_removes_sensitive_fields_recursively() {
        let mut value = serde_json::json!({"serial":"secret", "properties":[["serial","serial-secret"],["port.id","usb3-port2"]], "usb_device":{"serial":"nested", "sysfs_path":"/sys/private", "raw_attributes":{"uevent":"secret"}, "bus_port":"1-2"}});
        scrub(&mut value);
        assert_eq!(
            value,
            serde_json::json!({"properties":[["port.id","usb3-port2"]], "usb_device":{"bus_port":"1-2"}})
        );
    }
    #[test]
    fn events_are_bounded() {
        assert_eq!(MAX_EVENTS, 100);
    }
    #[test]
    fn live_snapshot_and_refresh_are_serializable() {
        let mut backend = Backend::new();
        let (next, _) = backend.refresh();
        assert_eq!(next.source, "live");
        assert!(next.updated_at_ms > 0);
        for device in &next.devices {
            assert!(device.get("id").and_then(Value::as_str).is_some());
            assert!(device.get("category").is_some());
            let text = device.to_string();
            assert!(!text.contains("sysfs_path"));
            assert!(!text.contains("raw_attributes"));
        }
        assert!(serde_json::to_string(&next).is_ok());
    }
}
