//! screenshot region|screen|all — save to ~/Pictures/Screenshots, copy to
//! clipboard, notify. Port of screenshot.sh; the `hyprctl … | python3`
//! JSON hop is now a native IPC call.

use crate::hypr;
use crate::util;
use serde_json::Value;
use std::process::{Command, ExitCode, Stdio};

pub fn run(args: &[&str]) -> ExitCode {
    let mode = args.first().copied().unwrap_or("region");
    let dir = util::home().join("Pictures/Screenshots");
    if std::fs::create_dir_all(&dir).is_err() {
        return util::fail_exit();
    }
    let (y, mo, d, h, mi, s) = util::localtime_now();
    let file = dir.join(format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}.png"));
    let file_s = file.to_string_lossy().into_owned();

    let what = match mode {
        "region" => {
            // Esc in slurp cancels quietly (exit 0), like the script
            let out = Command::new("slurp").output();
            let Ok(out) = out else { return util::fail_exit() };
            if !out.status.success() {
                return util::ok_exit();
            }
            let geom = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if geom.is_empty() {
                return util::ok_exit();
            }
            if !util::run_capture(&["grim", "-g", &geom, &file_s]).0 {
                return util::fail_exit();
            }
            "region"
        }
        "screen" => {
            let mon = hypr::active_workspace()
                .and_then(|w| w.get("monitor").and_then(Value::as_str).map(String::from));
            let Some(mon) = mon else { return util::fail_exit() };
            if !util::run_capture(&["grim", "-o", &mon, &file_s]).0 {
                return util::fail_exit();
            }
            "this screen"
        }
        "all" => {
            if !util::run_capture(&["grim", &file_s]).0 {
                return util::fail_exit();
            }
            "all screens"
        }
        // bash `case` fell through silently for unknown modes: no capture,
        // then wl-copy of a nonexistent file failed under set -e. We just exit.
        _ => return util::fail_exit(),
    };

    // wl-copy < "$f"
    let copied = std::fs::File::open(&file)
        .ok()
        .and_then(|f| {
            Command::new("wl-copy")
                .stdin(Stdio::from(f))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
        })
        .map(|st| st.success())
        .unwrap_or(false);
    if !copied {
        return util::fail_exit();
    }

    util::notify(&[
        "-i",
        &file_s,
        &format!("Screenshot ({what})"),
        &format!("Copied to clipboard + saved:\n{}", util::tilde(&file_s)),
    ]);
    util::ok_exit()
}
