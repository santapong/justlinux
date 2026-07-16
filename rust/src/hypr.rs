//! Hyprland IPC client — talks to .socket.sock directly instead of
//! spawning `hyprctl` (the bash/python versions forked a process per query).
//!
//! Falls back to spawning `hyprctl` when the socket can't be found, so the
//! tools still work in odd environments; `hyprctl hyprpaper …` is always
//! spawned because hyprpaper has its own socket protocol and the CLI is the
//! stable interface for it.

use crate::util;
use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock
pub fn socket_path() -> PathBuf {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    PathBuf::from(runtime).join("hypr").join(sig).join(".socket.sock")
}

fn socket_request(cmd: &str) -> std::io::Result<Vec<u8>> {
    let mut s = UnixStream::connect(socket_path())?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    s.write_all(cmd.as_bytes())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Send one request ("dispatch …", "keyword …", "reload", "j/clients" …).
/// Under DRYRUN, records the request and returns "".
pub fn request(cmd: &str) -> String {
    if util::dry() {
        util::record_action(&["hyprctl-ipc", cmd]);
        return String::new();
    }
    match socket_request(cmd) {
        Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
        Err(_) => {
            // fallback: hyprctl CLI (also lets tests stub it on PATH)
            let args: Vec<&str> = if let Some(rest) = cmd.strip_prefix("j/") {
                let mut v = vec!["-j"];
                v.extend(rest.split_whitespace());
                v
            } else {
                cmd.split_whitespace().collect()
            };
            let mut full = vec!["hyprctl"];
            full.extend(args);
            util::run_capture(&full).1
        }
    }
}

/// JSON request: `what` without the j/ prefix, e.g. "clients".
pub fn request_json(what: &str) -> Option<Value> {
    let out = request(&format!("j/{what}"));
    serde_json::from_str(&out).ok()
}

/// hyprctl dispatch …
pub fn dispatch(args: &str) -> String {
    request(&format!("dispatch {args}"))
}

/// hyprctl keyword <opt> <value> (live appearance changes).
pub fn keyword(opt: &str, value: &str) -> String {
    request(&format!("keyword {opt} {value}"))
}

/// hyprctl -j getoption <opt> → "int" value (bools are 0/1), or default.
pub fn getoption_int(opt: &str, default: i64) -> i64 {
    if util::dry() {
        return default;
    }
    match request_json(&format!("getoption {opt}")) {
        Some(v) => {
            if let Some(i) = v.get("int").and_then(Value::as_i64) {
                i
            } else if let Some(c) = v.get("custom").and_then(Value::as_str) {
                // gaps report as "N N N N"
                c.split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default)
            } else {
                default
            }
        }
        None => default,
    }
}

/// Raw "custom" string for options like gaps ("5 10 5 10"), or None.
pub fn getoption_custom(opt: &str) -> Option<String> {
    if util::dry() {
        return None;
    }
    request_json(&format!("getoption {opt}"))
        .and_then(|v| v.get("custom").and_then(Value::as_str).map(|s| s.trim().to_string()))
}

/// `hyprctl hyprpaper <args…>` — deliberately spawned, see module docs.
pub fn hyprpaper(args: &[&str]) {
    if util::dry() {
        let mut full = vec!["hyprctl", "hyprpaper"];
        full.extend_from_slice(args);
        util::record_action(&full);
        return;
    }
    let mut full = vec!["hyprctl", "hyprpaper"];
    full.extend_from_slice(args);
    util::run_capture(&full);
}

/// Clients list as JSON array (empty vec on failure).
pub fn clients() -> Vec<Value> {
    request_json("clients")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

pub fn active_workspace() -> Option<Value> {
    request_json("activeworkspace")
}

pub fn active_window() -> Option<Value> {
    request_json("activewindow")
}

/// Monitors: [{name, x, y, focused}, …]
pub fn monitors() -> Vec<Value> {
    request_json("monitors")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// Cursor position (x, y) — used by the autohide daemon.
pub fn cursor_pos() -> Option<(i64, i64)> {
    let out = socket_request("cursorpos").ok()?;
    let s = String::from_utf8_lossy(&out);
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

pub fn reload() {
    request("reload");
}
