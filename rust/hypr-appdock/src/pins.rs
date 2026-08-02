//! pins.json v2 state (per-monitor pin sets), ported from bin/hypr-appdock.
//!
//! The whole load → mutate → save cycle runs under an exclusive flock on
//! pins.lock so the (python) picker process and this dock process can
//! never silently revert each other's edits. A corrupt pins.json is
//! retried briefly, then moved aside — an edit is NEVER built on the
//! empty fallback (that would wipe all pins on save).

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use serde_json::{json, Value};

fn pins_path() -> PathBuf {
    hyprdesk::home().join(".local/state/hypr-appdock/pins.json")
}

fn launcher_state() -> PathBuf {
    hyprdesk::home().join(".local/state/hypr-launcher/state.json")
}

pub fn load_pins() -> Value {
    match std::fs::read_to_string(pins_path()) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => v,
            // mid-write/corrupt: defaults, but DON'T overwrite the file —
            // the next good save wins, per-workspace pins survive
            Err(_) => json!({"default": [], "workspaces": {}}),
        },
        Err(_) => {
            // first run: seed the default set from Hypr Launcher favorites
            let favs = std::fs::read_to_string(launcher_state())
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.get("favorites").cloned())
                .unwrap_or_else(|| json!([]));
            let state = json!({"default": favs, "workspaces": {}});
            save_pins(&state);
            state
        }
    }
}

fn save_pins(state: &Value) {
    let path = pins_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // unique tmp per writer: a shared fixed name lets two processes
    // truncate each other mid-replace
    let tmp = path.with_file_name(format!(".pins-{}", std::process::id()));
    if std::fs::write(&tmp, serde_json::to_string_pretty(state).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Run `mutate(state)` under the lock and save. Returns the new state,
/// or None if the edit was aborted (persistent corruption).
pub fn pins_update(mutate: impl FnOnce(&mut Value)) -> Option<Value> {
    let path = pins_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let lockf = File::create(path.with_file_name("pins.lock")).ok()?;
    unsafe { libc::flock(lockf.as_raw_fd(), libc::LOCK_EX) };
    let mut state = None;
    for _ in 0..3 {
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => {
                    state = Some(v);
                    break;
                }
                // another writer mid-flight? retry
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
            },
            Err(_) => {
                state = Some(load_pins()); // missing file: seeded default
                break;
            }
        }
    }
    let Some(mut state) = state else {
        // persistent corruption: preserve it, abort the edit
        let _ = std::fs::rename(&path, path.with_file_name("pins.json.bad"));
        eprintln!("hypr-appdock: pins.json corrupt — moved to pins.json.bad, edit aborted");
        return None;
    };
    mutate(&mut state);
    save_pins(&state);
    Some(state)
}

pub fn pins_mtime() -> i64 {
    std::fs::metadata(pins_path())
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn as_str_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolution: monitor's workspace set → monitor default →
/// legacy workspace set → global default.
pub fn pins_for(state: &Value, mon: &str, ws: i64) -> Vec<String> {
    let wsk = ws.to_string();
    if let Some(m) = state.get("monitors").and_then(|ms| ms.get(mon)) {
        if let Some(v) = m.get("workspaces").and_then(|w| w.get(&wsk)) {
            return as_str_list(v);
        }
        if let Some(v) = m.get("default") {
            return as_str_list(v);
        }
    }
    if let Some(v) = state.get("workspaces").and_then(|w| w.get(&wsk)) {
        return as_str_list(v);
    }
    as_str_list(state.get("default").unwrap_or(&Value::Null))
}

/// The mutable pin list for (mon, ws), materializing lazily. First
/// monitor-scoped edit copies BOTH the legacy default and the legacy
/// per-workspace sets into the monitor block, so existing pins never
/// vanish on the monitor's other workspaces (python critic fix).
pub fn edit_list<'a>(state: &'a mut Value, mon: &str, ws: i64) -> &'a mut Vec<Value> {
    let default = state.get("default").cloned().unwrap_or_else(|| json!([]));
    let legacy_ws = state.get("workspaces").cloned().unwrap_or_else(|| json!({}));
    if !state.is_object() {
        *state = json!({});
    }
    let obj = state.as_object_mut().unwrap();
    let mons = obj
        .entry("monitors")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap();
    if !mons.contains_key(mon) {
        mons.insert(
            mon.to_string(),
            json!({"default": default, "workspaces": legacy_ws}),
        );
        obj.insert("version".into(), json!(2));
    }
    let m = obj
        .get_mut("monitors")
        .unwrap()
        .get_mut(mon)
        .unwrap()
        .as_object_mut()
        .unwrap();
    let mdefault = m.get("default").cloned().unwrap_or_else(|| json!([]));
    let wss = m
        .entry("workspaces")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap();
    wss.entry(ws.to_string()).or_insert(mdefault);
    wss.get_mut(&ws.to_string()).unwrap().as_array_mut().unwrap()
}
