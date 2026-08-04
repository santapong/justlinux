//! serial-watch — serial hotplug toast, ported from bin/serial-watch:
//! notify when a board is plugged/unplugged with its human name AND the
//! /dev/ttyXXX node, and bust the robotics widget cache so the row
//! appears within a couple of seconds. The last resident python on the
//! fleet — everything else python is on-demand.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn snapshot() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/dev/serial/by-id") else {
        return out;
    };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let dev = std::fs::canonicalize(e.path())
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_default();
        out.insert(name, dev);
    }
    out
}

fn pretty(name: &str) -> String {
    let s = name.replace("usb-", "");
    let s = s.split("-if").next().unwrap_or(&s).replace('_', " ");
    s.chars().take(48).collect()
}

fn toast(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-t", "5000", title, body])
        .output();
}

fn main() {
    let cache = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
        .join("widget-robotics.cache");
    let mut prev = snapshot();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let cur = snapshot();
        if cur == prev {
            continue;
        }
        for (name, dev) in &cur {
            if !prev.contains_key(name) {
                toast("󱐋 Board connected", &format!("{}\n→ /dev/{dev}", pretty(name)));
            }
        }
        for (name, dev) in &prev {
            if !cur.contains_key(name) {
                toast("󰌘 Board disconnected", &format!("{} (/dev/{dev})", pretty(name)));
            }
        }
        let _ = std::fs::remove_file(&cache); // widget refreshes next tick
        prev = cur;
    }
}
