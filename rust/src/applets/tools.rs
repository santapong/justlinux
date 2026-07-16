//! hypr-tools — desktop control menus & dispatcher. Port of hypr-tools.sh.
//!
//! The embedded `python3 - <<PY` JSON blocks (stash / hide / unhide /
//! reorder) are native here, talking to Hyprland's IPC socket directly;
//! rofi, kitty, pkexec, systemd-run etc. are still spawned — they ARE the
//! UI. All menu strings are byte-identical to the bash version.

use crate::applets::{fw_status, wallpaper};
use crate::{hypr, proc, util};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn conf_path() -> PathBuf {
    util::home().join(".config/hypr/hyprland.conf")
}

fn walldir() -> PathBuf {
    util::home().join("Pictures/wallpaper")
}

fn tools_sh() -> String {
    util::local_bin("hypr-tools.sh").to_string_lossy().into_owned()
}

/// rofi -dmenu wrapper: returns the picked line (trailing newline stripped),
/// None when the user pressed Esc / picked nothing.
fn rofi(input: &str, args: &[&str]) -> Option<String> {
    if util::dry() {
        // tests drive menus via HYPR_ROFI_ANSWERS (answers separated by \x1e)
        let mut rec = vec!["rofi"];
        rec.extend_from_slice(args);
        util::record_action(&rec);
        let answers = std::env::var("HYPR_ROFI_ANSWERS").unwrap_or_default();
        let n: usize = std::env::var("HYPR_ROFI_CURSOR")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let ans = answers.split('\x1e').nth(n).map(str::to_string);
        std::env::set_var("HYPR_ROFI_CURSOR", (n + 1).to_string());
        return ans.filter(|s| !s.is_empty());
    }
    let mut cmd = vec!["rofi", "-dmenu"];
    cmd.extend_from_slice(args);
    let out = util::run_with_input(&cmd, input)?;
    let out = out.trim_end_matches('\n').to_string();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|d| !d.is_empty() && Path::new(d).join(bin).is_file())
}

// ---------- wallpaper picker (legacy rofi flow) ----------

fn pick_wallpaper() {
    // 1. which monitor?
    let mut list = String::from("󰍺  All monitors\n");
    for m in hypr::monitors() {
        if let Some(name) = m.get("name").and_then(Value::as_str) {
            list.push_str(&format!("󰍹  {name}\n"));
        }
    }
    let Some(target) = rofi(&list, &["-i", "-p", "󰸉 Apply to"]) else {
        return;
    };
    let target = if target.contains("All monitors") {
        "all".to_string()
    } else {
        // ${target#* } then xargs: drop up to the first space, trim the rest
        target
            .split_once(' ')
            .map(|(_, r)| r.trim().to_string())
            .unwrap_or(target)
    };

    // 2. browse folders / pick image (thumbnails)
    let home = util::home();
    let mut dir = walldir();
    loop {
        let mut input = String::new();
        if dir != home {
            input.push_str("󰁍  ..\n");
        }
        input.push_str("󰒝  Use this folder (slideshow: random image every 5 min)\n");
        let mut subdirs: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        subdirs.sort();
        for d in &subdirs {
            input.push_str(&format!("󰉋  {d}\n"));
        }
        let mut imgs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .and_then(|e| e.to_str())
                                .map(|e| ["jpg", "jpeg", "png"].contains(&e))
                                .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();
        imgs.sort();
        for f in &imgs {
            let base = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            input.push_str(&format!("{base}\0icon\x1f{}\n", f.display()));
        }
        let prompt = format!("󰸉 {}", util::tilde(&dir.to_string_lossy()));
        let Some(choice) = rofi(
            &input,
            &[
                "-i",
                "-p",
                &prompt,
                "-theme-str",
                "element-icon { size: 56px; } listview { lines: 8; } window { width: 760px; }",
            ],
        ) else {
            return;
        };
        if choice == "󰁍  .." {
            dir = dir.parent().map(Path::to_path_buf).unwrap_or(dir);
        } else if choice.starts_with("󰒝  ") {
            wallpaper::run(&[&dir.to_string_lossy(), &target]);
            return;
        } else if let Some(sub) = choice.strip_prefix("󰉋  ") {
            dir = dir.join(sub);
        } else {
            wallpaper::run(&[&dir.join(&choice).to_string_lossy(), &target]);
            return;
        }
    }
}

// ---------- workspace stash (hide/unhide all windows) ----------

fn stash_state_file(wsid: i64) -> PathBuf {
    util::xdg_runtime().join(format!("hypr-stash-{wsid}.json"))
}

fn stash_toggle() {
    let Some(ws) = hypr::active_workspace() else { return };
    let wsid = ws.get("id").and_then(Value::as_i64).unwrap_or(0);
    let ws_name = ws
        .get("name")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    if ws_name.starts_with("special") {
        util::notify(&[
            "Workspace",
            "You're viewing a hidden stack — go to a normal workspace first",
        ]);
        return;
    }
    let stash = format!("special:stash{wsid}");
    let state_file = stash_state_file(wsid);
    let clients = hypr::clients();

    // stashed windows in clients order (python dict preserved insertion order)
    let stashed: Vec<&Value> = clients
        .iter()
        .filter(|c| c.pointer("/workspace/name").and_then(Value::as_str) == Some(stash.as_str()))
        .collect();

    let msg;
    if !stashed.is_empty() {
        // restore in the exact order they were hidden (saved layout order)
        let order: Vec<String> = std::fs::read_to_string(&state_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
            .unwrap_or_default();
        let stashed_addrs: Vec<String> = stashed
            .iter()
            .map(|c| c.get("address").and_then(Value::as_str).unwrap_or_default().to_string())
            .collect();
        let mut ordered: Vec<String> =
            order.iter().filter(|a| stashed_addrs.contains(a)).cloned().collect();
        for a in &stashed_addrs {
            if !order.contains(a) {
                ordered.push(a.clone());
            }
        }
        for addr in &ordered {
            hypr::dispatch(&format!("movetoworkspacesilent {wsid},address:{addr}"));
        }
        let _ = std::fs::remove_file(&state_file);
        msg = format!("󰘸 Restored {} window(s) on workspace {wsid}", ordered.len());
    } else {
        let mut current: Vec<&Value> = clients
            .iter()
            .filter(|c| c.pointer("/workspace/id").and_then(Value::as_i64) == Some(wsid))
            .collect();
        if current.is_empty() {
            msg = format!("Workspace {wsid} has no windows to hide");
        } else {
            // save layout order: top-left window first, then reading order
            current.sort_by_key(|c| {
                (
                    c.pointer("/at/1").and_then(Value::as_i64).unwrap_or(0),
                    c.pointer("/at/0").and_then(Value::as_i64).unwrap_or(0),
                )
            });
            let addrs: Vec<String> = current
                .iter()
                .map(|c| c.get("address").and_then(Value::as_str).unwrap_or_default().to_string())
                .collect();
            let _ = std::fs::write(&state_file, serde_json::to_string(&addrs).unwrap_or_default());
            for addr in &addrs {
                hypr::dispatch(&format!("movetoworkspacesilent {stash},address:{addr}"));
            }
            msg = format!("󰘸 Hid {} window(s) — ALT+A again to bring them back", addrs.len());
        }
    }
    util::notify(&["Workspace", &msg]);
}

// ---------- per-window hide / unhide (minimize) ----------

fn hidden_state_file() -> PathBuf {
    util::xdg_runtime().join("hypr-hidden.json")
}

fn read_hidden_state() -> serde_json::Map<String, Value> {
    std::fs::read_to_string(hidden_state_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn chars60(s: &str) -> String {
    s.chars().take(60).collect()
}

fn hide_window() {
    let w = hypr::active_window().unwrap_or_else(|| json!({}));
    let addr = w.get("address").and_then(Value::as_str).unwrap_or("").to_string();
    if addr.is_empty() {
        util::notify(&["Hide window", "No focused window"]);
        return;
    }
    let ws_name = w.pointer("/workspace/name").and_then(Value::as_str).unwrap_or("");
    if ws_name.starts_with("special") {
        util::notify(&["Hide window", "This window is already on a hidden workspace"]);
        return;
    }
    let mut d = read_hidden_state();
    d.insert(
        addr.clone(),
        json!({
            "ws": w.pointer("/workspace/id").cloned().unwrap_or(Value::Null),
            "title": w.get("title").and_then(Value::as_str).unwrap_or(""),
            "cls": w.get("class").and_then(Value::as_str).unwrap_or(""),
        }),
    );
    let _ = std::fs::write(
        hidden_state_file(),
        serde_json::to_string(&Value::Object(d)).unwrap_or_default(),
    );
    hypr::dispatch(&format!("movetoworkspacesilent special:hidden,address:{addr}"));
    let title = w.get("title").and_then(Value::as_str).unwrap_or("");
    util::notify(&[
        "󰘸 Window hidden",
        &format!("{}\nALT+CTRL+A to bring it back", chars60(title)),
    ]);
}

fn unhide_window() {
    let mut saved = read_hidden_state();
    let clients = hypr::clients();
    let hidden: Vec<&Value> = clients
        .iter()
        .filter(|c| {
            c.pointer("/workspace/name").and_then(Value::as_str) == Some("special:hidden")
        })
        .collect();
    if hidden.is_empty() {
        util::notify(&[
            "Hidden windows",
            "Nothing is hidden (per-window). ALT+SHIFT+A hides the focused window.",
        ]);
        return;
    }
    let mut lines: Vec<String> = hidden
        .iter()
        .map(|c| {
            format!(
                "{}  —  {}",
                c.get("class").and_then(Value::as_str).unwrap_or(""),
                chars60(c.get("title").and_then(Value::as_str).unwrap_or(""))
            )
        })
        .collect();
    lines.push("󰗐  Restore ALL hidden windows".to_string());
    let input = lines.join("\n");
    let Some(out) = rofi(
        &input,
        &[
            "-i",
            "-format",
            "i",
            "-p",
            "󰘸 Hidden",
            "-mesg",
            "Enter = bring the window back to its workspace",
        ],
    ) else {
        return;
    };
    let Ok(idx) = out.trim().parse::<usize>() else { return };

    let active_ws = hypr::active_workspace()
        .and_then(|w| w.get("id").and_then(Value::as_i64))
        .unwrap_or(0);

    let mut restore = |c: &Value, follow: bool| {
        let addr = c.get("address").and_then(Value::as_str).unwrap_or("").to_string();
        let ws = saved
            .get(&addr)
            .and_then(|s| s.get("ws"))
            .and_then(Value::as_i64)
            .unwrap_or(active_ws);
        let verb = if follow { "movetoworkspace" } else { "movetoworkspacesilent" };
        hypr::dispatch(&format!("{verb} {ws},address:{addr}"));
        saved.remove(&addr);
    };

    if idx == hidden.len() {
        // Restore ALL
        for c in &hidden {
            restore(c, false);
        }
        util::notify(&[
            "󰘸 Restored",
            &format!("{} window(s) back on their workspaces", hidden.len()),
        ]);
    } else if idx < hidden.len() {
        restore(hidden[idx], true); // follow: jump to it
    }
    let _ = std::fs::write(
        hidden_state_file(),
        serde_json::to_string(&Value::Object(saved)).unwrap_or_default(),
    );
}

// ---------- power menu ----------

fn confirm(prompt: &str, yes_line: &str) -> bool {
    let input = format!("No — stay\n{yes_line}\n");
    matches!(
        rofi(
            &input,
            &["-i", "-p", prompt, "-theme-str", "listview { lines: 2; } window { width: 320px; }"]
        ),
        Some(c) if c.starts_with("Yes")
    )
}

fn power_menu() {
    let input = "󰌾  Lock screen\n󰤄  Suspend (sleep)\n󰍃  Logout\n󰜉  Reboot\n⏻  Shutdown\n";
    let Some(pick) = rofi(
        input,
        &[
            "-i",
            "-p",
            "⏻ Power",
            "-theme-str",
            "listview { lines: 5; } window { width: 360px; } element { padding: 12px; } element-text { font: \"JetBrainsMono Nerd Font 14\"; }",
        ],
    ) else {
        return;
    };
    if pick.contains("Lock") {
        util::spawn_detached(&["hyprlock"]);
    } else if pick.contains("Suspend") {
        util::spawn_detached(&["systemctl", "suspend"]);
    } else if pick.contains("Logout") {
        if confirm("󰍃 Log out?", "Yes — log out") {
            hypr::dispatch("exit");
        }
    } else if pick.contains("Reboot") {
        if confirm("󰜉 Reboot?", "Yes — reboot") {
            util::spawn_detached(&["systemctl", "reboot"]);
        }
    } else if pick.contains("Shutdown") && confirm("⏻ Shut down?", "Yes — shut down") {
        util::spawn_detached(&["systemctl", "poweroff"]);
    }
}

// ---------- security: firewall / antivirus ----------

fn kitty_float(title: &str, bash_payload: &str) {
    util::spawn_detached(&[
        "kitty", "--class", "floatterm", "--title", title, "bash", "-c", bash_payload,
    ]);
}

fn firewall_menu() {
    let (state, toggle) = if fw_status::ufw_enabled_in_conf() {
        ("ON", "turn OFF")
    } else {
        ("OFF", "turn ON")
    };
    let input =
        format!("󰕥  Firewall is {state} → {toggle}\n󰋗  View rules & status\n󰐕  Allow a port…\n");
    let Some(pick) = rofi(&input, &["-i", "-p", "󰕥 Firewall"]) else { return };
    if pick.ends_with("turn OFF") {
        if util::run_capture(&["pkexec", "ufw", "disable"]).0 {
            util::notify(&["-u", "critical", "󰕥 Firewall", "DISABLED"]);
        }
    } else if pick.ends_with("turn ON") {
        if util::run_capture(&["pkexec", "ufw", "enable"]).0 {
            util::notify(&["󰕥 Firewall", "Enabled ✔"]);
        }
    } else if pick.contains("View") {
        kitty_float(
            "Firewall rules",
            "sudo ufw status verbose; echo; read -rp \"— Enter to close —\"",
        );
    } else if pick.contains("Allow") {
        let port =
            rofi("", &["-p", "󰐕 Port to allow", "-l", "0", "-mesg", "e.g. 22, 8080, or 443/tcp"]);
        if let Some(port) = port {
            if util::run_capture(&["pkexec", "ufw", "allow", &port]).0 {
                util::notify(&["󰕥 Firewall", &format!("Port {port} allowed")]);
            }
        }
    }
}

fn clamav_menu() {
    let gui = if which("clamtk") {
        "󰍜  Open ClamTk (graphical antivirus app)"
    } else {
        "󰐕  Install ClamTk — a GUI for ClamAV"
    };
    let input = format!(
        "󰃤  Scan Downloads (quick)\n󰋊  Scan whole home folder (slow)\n󰉋  Scan a folder I pick…\n{gui}\n󰚰  Update virus definitions now\n󰋗  Service status\n"
    );
    let Some(pick) = rofi(&input, &["-i", "-p", "󰃤 Antivirus"]) else { return };
    if pick.contains("Downloads") {
        kitty_float(
            "Virus scan: Downloads",
            "echo \"Scanning ~/Downloads …\"; clamdscan --multiscan --fdpass \"$HOME/Downloads\"; echo; read -rp \"— Enter to close —\"",
        );
    } else if pick.contains("home folder") {
        kitty_float(
            "Virus scan: home",
            "echo \"Scanning $HOME — this can take a long time. Ctrl+C to stop.\"; clamdscan --multiscan --fdpass \"$HOME\"; echo; read -rp \"— Enter to close —\"",
        );
    } else if pick.contains("folder I pick") {
        // find "$HOME" -mindepth 1 -maxdepth 2 -type d -not -path '*/.*'
        let home = util::home();
        let mut dirs: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&home) {
            let mut top: Vec<_> = rd.flatten().collect();
            top.sort_by_key(|e| e.file_name());
            for e in top {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if !p.is_dir() || name.starts_with('.') {
                    continue;
                }
                dirs.push(p.to_string_lossy().into_owned());
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    let mut sub: Vec<_> = rd2.flatten().collect();
                    sub.sort_by_key(|e| e.file_name());
                    for e2 in sub {
                        let p2 = e2.path();
                        let n2 = e2.file_name().to_string_lossy().into_owned();
                        if p2.is_dir() && !n2.starts_with('.') {
                            dirs.push(p2.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
        let home_s = home.to_string_lossy().into_owned();
        let listing: String =
            dirs.iter().map(|d| format!("{}\n", d.replacen(&home_s, "~", 1))).collect();
        if let Some(dir) = rofi(&listing, &["-i", "-p", "󰉋 Scan which folder?"]) {
            let stripped = dir.strip_prefix('~').unwrap_or(&dir);
            kitty_float(
                "Virus scan",
                &format!(
                    "echo 'Scanning {dir} …'; clamdscan --multiscan --fdpass \"${{HOME}}{stripped}\"; echo; read -rp '— Enter to close —'"
                ),
            );
        }
    } else if pick.contains("Open ClamTk") {
        util::spawn_detached(&["clamtk"]);
    } else if pick.contains("Install ClamTk") {
        kitty_float(
            "Install ClamTk",
            "sudo apt install -y clamtk; echo; read -rp \"— Done. Enter to close —\"",
        );
    } else if pick.contains("Update") {
        kitty_float(
            "Update virus definitions",
            "sudo systemctl stop clamav-freshclam && sudo freshclam; sudo systemctl start clamav-freshclam; echo; read -rp \"— Enter to close —\"",
        );
    } else if pick.contains("status") {
        kitty_float(
            "ClamAV status",
            "systemctl status clamav-daemon clamav-freshclam --no-pager | head -30; echo; read -rp \"— Enter to close —\"",
        );
    }
}

// ---------- clock / reminders ----------

/// "45m" / "2h" / "10s" — ^[0-9]+[smh]$
pub fn is_rel_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && b[..b.len() - 1].iter().all(u8::is_ascii_digit)
        && matches!(b[b.len() - 1], b's' | b'm' | b'h')
}

/// ^([01]?[0-9]|2[0-3]):[0-5][0-9]$
pub fn parse_clock_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    if !(1..=2).contains(&h.len()) || m.len() != 2 {
        return None;
    }
    if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    if h <= 23 && m <= 59 {
        Some((h, m))
    } else {
        None
    }
}

/// "%Y-%m-%d %H:%M:00" for today at HH:MM, or tomorrow if already past —
/// what the bash did with two `date -d` calls. `mktime` normalizes mday+1,
/// so DST transitions are handled like `date -d tomorrow`.
fn calendar_for(h: u32, m: u32) -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = h as i32;
        tm.tm_min = m as i32;
        tm.tm_sec = 0;
        tm.tm_isdst = -1;
        let mut target = libc::mktime(&mut tm);
        if target <= now {
            libc::localtime_r(&now, &mut tm);
            tm.tm_mday += 1;
            tm.tm_hour = h as i32;
            tm.tm_min = m as i32;
            tm.tm_sec = 0;
            tm.tm_isdst = -1;
            target = libc::mktime(&mut tm);
        }
        let mut out: libc::tm = std::mem::zeroed();
        libc::localtime_r(&target, &mut out);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:00",
            out.tm_year + 1900,
            out.tm_mon + 1,
            out.tm_mday,
            out.tm_hour,
            out.tm_min
        )
    }
}

const REMINDER_PAYLOAD: &str = "export DBUS_SESSION_BUS_ADDRESS=\"unix:path=/run/user/$(id -u)/bus\"; notify-send -u critical -t 0 \"⏰ Reminder\" \"$RTEXT\"";

fn systemd_run_reminder(timing_flag: &str, unit: &str, text: &str) {
    let cmd = [
        "systemd-run",
        "--user",
        timing_flag,
        &format!("--unit={unit}"),
        &format!("--description={text}"),
        &format!("--setenv=RTEXT={text}"),
        "sh",
        "-c",
        REMINDER_PAYLOAD,
    ];
    if util::dry() {
        util::record_action(&cmd);
    } else {
        util::run_capture(&cmd);
    }
}

fn new_reminder() {
    let Some(text) = rofi(
        "",
        &["-p", "⏰ Remind me about…", "-l", "0", "-mesg", "Type your reminder text, then press Enter"],
    ) else {
        return;
    };
    let Some(when) = rofi(
        "5m\n10m\n15m\n30m\n1h\n2h\n",
        &["-i", "-p", "󰔛 When?", "-mesg", "Pick one, or type your own: 45m, 3h, or a clock time like 17:30"],
    ) else {
        return;
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unit = format!("reminder-{nanos}");
    if is_rel_time(&when) {
        systemd_run_reminder(&format!("--on-active={when}"), &unit, &text);
    } else if let Some((h, m)) = parse_clock_time(&when) {
        systemd_run_reminder(&format!("--on-calendar={}", calendar_for(h, m)), &unit, &text);
    } else {
        util::notify(&[
            "-u",
            "low",
            "Reminder",
            &format!("Didn't understand \"{when}\" — use 45m, 2h, or 17:30"),
        ]);
        return;
    }
    util::notify(&["⏰ Reminder set", &format!("\"{text}\" — {when}")]);
}

fn list_reminders() {
    let (_, timers) =
        util::run_capture(&["systemctl", "--user", "list-timers", "reminder-*", "--no-legend"]);
    // grep -o 'reminder-[0-9]*\.timer'
    let mut units: Vec<String> = Vec::new();
    for tok in timers.split_whitespace() {
        if let Some(start) = tok.find("reminder-") {
            let cand = &tok[start..];
            if let Some(end) = cand.find(".timer") {
                let digits = &cand["reminder-".len()..end];
                if digits.chars().all(|c| c.is_ascii_digit()) {
                    let u = format!("reminder-{digits}.timer");
                    if !units.contains(&u) {
                        units.push(u);
                    }
                }
            }
        }
    }
    if units.is_empty() {
        util::notify(&["Reminders", "No reminders scheduled — set one from the clock menu"]);
        return;
    }
    let mut lines = String::new();
    for unit in &units {
        let (_, desc) = util::run_capture(&[
            "systemctl", "--user", "show", unit, "-p", "Description", "--value",
        ]);
        let (_, when_raw) =
            util::run_capture(&["systemctl", "--user", "list-timers", unit, "--no-legend"]);
        let when: String = when_raw
            .lines()
            .next()
            .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        lines.push_str(&format!("{} │ {} │ {}\n", desc.trim_end(), when, unit));
    }
    let Some(sel) = rofi(
        &lines,
        &["-i", "-p", "󰃰 Reminders", "-mesg", "Press Enter on a reminder to CANCEL it — Esc to close"],
    ) else {
        return;
    };
    // unit="${sel##*│ }"  (after the LAST │)
    let unit = sel.rsplit('│').next().unwrap_or("").trim().to_string();
    util::run_capture(&["systemctl", "--user", "stop", &unit]);
    // "Cancelled: ${sel%% │*}"  (before the FIRST " │")
    let cancelled = sel.split(" │").next().unwrap_or("");
    util::notify(&["Reminders", &format!("Cancelled: {cancelled}")]);
}

fn clock_menu() {
    let input = "⏰  New reminder\n󰃰  Reminders — list / cancel\n󰅐  Set timezone\n󰅑  Set date & time manually\n󰑓  Enable automatic time sync (NTP)\n";
    let Some(pick) = rofi(input, &["-i", "-p", "󰥔 Clock"]) else { return };
    if pick.ends_with("New reminder") {
        new_reminder();
    } else if pick.contains("Reminders") {
        list_reminders();
    } else if pick.ends_with("timezone") {
        let (_, tzs) = util::run_capture(&["timedatectl", "list-timezones"]);
        if let Some(tz) = rofi(&tzs, &["-i", "-p", "󰅐 Timezone"]) {
            if util::run_capture(&["timedatectl", "set-timezone", &tz]).0 {
                util::notify(&["Clock", &format!("Timezone set to {tz}")]);
            }
        }
    } else if pick.ends_with("manually") {
        let t = rofi(
            "",
            &["-p", "󰅑 New time", "-l", "0", "-mesg", "Format: 2026-07-15 20:30:00  or just  20:30  — (this turns NTP off)"],
        );
        if let Some(t) = t {
            if util::run_capture(&["timedatectl", "set-ntp", "false"]).0
                && util::run_capture(&["timedatectl", "set-time", &t]).0
            {
                util::notify(&["Clock", &format!("Time set to {t} (auto-sync disabled)")]);
            }
        }
    } else if pick.contains("NTP") && util::run_capture(&["timedatectl", "set-ntp", "true"]).0 {
        util::notify(&["Clock", "Automatic time sync enabled"]);
    }
}

// ---------- bar ----------

fn autohide_running() -> bool {
    !proc::pids_with_cmdline("waybar-autohide.sh").is_empty()
}

fn restart_bar() {
    proc::pkill_comm("waybar", libc::SIGTERM);
    std::thread::sleep(std::time::Duration::from_millis(500));
    hypr::dispatch("exec waybar");
    // a fresh waybar starts visible — restart the auto-hide daemon so its state matches
    if !proc::pids_with_cmdline("waybar-autohid").is_empty() {
        proc::pkill_cmdline("waybar-autohid", libc::SIGTERM);
        std::thread::sleep(std::time::Duration::from_millis(500));
        util::spawn_detached(&[&util::local_bin("waybar-autohide.sh").to_string_lossy()]);
    }
}

fn toggle_autohide() {
    if autohide_running() {
        proc::pkill_cmdline("waybar-autohide.sh", libc::SIGTERM);
        util::notify(&["Waybar", "Auto-hide OFF — bar always visible"]);
    } else {
        util::spawn_detached(&[&util::local_bin("waybar-autohide.sh").to_string_lossy()]);
        util::notify(&[
            "Waybar",
            "Auto-hide ON — touch the top edge to reveal (ALT+B to toggle manually)",
        ]);
    }
}

// ---------- keybind cheat sheet ----------

/// ^\s*bind[elm]*\s*=  (the grep -E filter)
fn is_bind_line(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix("bind") else { return false };
    let after = rest.trim_start_matches(['e', 'l', 'm']);
    after.trim_start().starts_with('=')
}

/// sed 's/bind[elm]*\s*=\s*//' — remove the first bind…= occurrence.
fn strip_bind_prefix(numbered: &str) -> String {
    if let Some(pos) = numbered.find("bind") {
        let rest = &numbered[pos + 4..];
        let rest = rest.trim_start_matches(['e', 'l', 'm']);
        let rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            return format!("{}{}", &numbered[..pos], r);
        }
    }
    numbered.to_string()
}

/// The sed pipeline transforms (each: FIRST occurrence only, like sed s///).
pub fn keybind_display(numbered: &str) -> String {
    let mut s = strip_bind_prefix(numbered);
    for (from, to) in
        [("$mod SHIFT", "ALT+SHIFT"), ("$mod CTRL", "ALT+CTRL"), ("$mod", "ALT")]
    {
        if let Some(p) = s.find(from) {
            s.replace_range(p..p + from.len(), to);
        }
    }
    // s/,\s*/ , /  (first occurrence)
    if let Some(p) = s.find(',') {
        let after = &s[p + 1..];
        let ws = after.len() - after.trim_start().len();
        s.replace_range(p..p + 1 + ws, " , ");
    }
    s
}

fn edit_keybinds() {
    let conf = conf_path();
    let Ok(text) = std::fs::read_to_string(&conf) else { return };
    let mut input = String::new();
    for (i, line) in text.lines().enumerate() {
        if is_bind_line(line) {
            input.push_str(&keybind_display(&format!("{}:{}", i + 1, line)));
            input.push('\n');
        }
    }
    let Some(sel) = rofi(
        &input,
        &[
            "-i",
            "-p",
            "󰌌 Keybinds (Enter = edit)",
            "-theme-str",
            "listview { lines: 12; } window { width: 860px; }",
        ],
    ) else {
        return;
    };
    let line = sel.split(':').next().unwrap_or("").to_string();
    // Open the config at that exact line; reload Hyprland when the editor closes
    util::spawn_detached(&[
        "kitty",
        "--title",
        "Edit keybinding (save & quit to apply)",
        "sh",
        "-c",
        &format!("${{EDITOR:-nvim}} +{line} '{}'; hyprctl reload", conf.display()),
    ]);
}

// ---------- waybar module reorder ----------

const MODULE_SECTIONS: [&str; 3] = ["modules-left", "modules-center", "modules-right"];

pub fn module_pretty_name(m: &str) -> String {
    match m {
        "custom/menu" => "\u{f035c}  Menu button",
        "custom/files" => "\u{f024b}  File manager",
        "custom/windows" => "\u{f05af}  Window switcher",
        "hyprland/workspaces" => "\u{f09e0}  Workspace numbers",
        "hyprland/window" => "\u{f05b2}  Window title",
        "clock" => "\u{f0954}  Clock",
        "custom/wallpaper" => "\u{f0e09}  Wallpaper picker",
        "custom/keybinds" => "\u{f030c}  Keybindings",
        "pulseaudio" => "  Volume",
        "network" => "  Network",
        "cpu" => "  CPU usage",
        "temperature" => "\u{f050f}  CPU temperature",
        "memory" => "  RAM usage",
        "disk" => "\u{f02ca}  Disk usage",
        "tray" => "\u{f1294}  System tray (app icons)",
        "custom/notification" => "\u{f009a}  Notification bell",
        "custom/power" => "⏻  Power button",
        other => other,
    }
    .to_string()
}

fn side_label(section: &str) -> &'static str {
    match section {
        "modules-left" => "LEFT",
        "modules-center" => "CENTER",
        _ => "RIGHT",
    }
}

/// (section, module) pairs in display order.
pub fn flat_modules(cfg: &Value) -> Vec<(String, String)> {
    let mut flat = Vec::new();
    for s in MODULE_SECTIONS {
        if let Some(arr) = cfg.get(s).and_then(Value::as_array) {
            for m in arr {
                if let Some(m) = m.as_str() {
                    flat.push((s.to_string(), m.to_string()));
                }
            }
        }
    }
    flat
}

/// The python reorder logic, 1:1. i2 indexes flat + 3 virtual "END of …" rows.
pub fn apply_reorder(cfg: &mut Value, i1: usize, i2: usize) {
    let flat = flat_modules(cfg);
    if i1 >= flat.len() {
        return;
    }
    let (src_sec, src_mod) = flat[i1].clone();
    // cfg[src_sec].remove(src_mod) — first occurrence
    if let Some(arr) = cfg.get_mut(&src_sec).and_then(Value::as_array_mut) {
        if let Some(pos) = arr.iter().position(|v| v.as_str() == Some(src_mod.as_str())) {
            arr.remove(pos);
        }
    }
    if i2 < flat.len() {
        let (tgt_sec, tgt_mod) = flat[i2].clone();
        if (tgt_sec.as_str(), tgt_mod.as_str()) != (src_sec.as_str(), src_mod.as_str()) {
            if let Some(arr) = cfg.get_mut(&tgt_sec).and_then(Value::as_array_mut) {
                let pos = arr
                    .iter()
                    .position(|v| v.as_str() == Some(tgt_mod.as_str()))
                    .unwrap_or(arr.len());
                arr.insert(pos, Value::String(src_mod));
            }
        } else if let Some(arr) = cfg.get_mut(&src_sec).and_then(Value::as_array_mut) {
            arr.push(Value::String(src_mod)); // moved before itself: no-op, put it back
        }
    } else {
        let sec = MODULE_SECTIONS[(i2 - flat.len()).min(2)];
        if let Some(arr) = cfg.get_mut(sec).and_then(Value::as_array_mut) {
            arr.push(Value::String(src_mod));
        } else if let Some(obj) = cfg.as_object_mut() {
            obj.insert(sec.to_string(), json!([src_mod]));
        }
    }
}

/// json.dump(cfg, indent=4) equivalent (key order preserved).
pub fn to_json_indent4(cfg: &Value) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    use serde::Serialize;
    let _ = cfg.serialize(&mut ser);
    String::from_utf8_lossy(&buf).into_owned()
}

fn reorder_modules() {
    let cfg_path = util::home().join(".config/waybar/config");
    let Ok(text) = std::fs::read_to_string(&cfg_path) else { return };
    let Ok(mut cfg) = serde_json::from_str::<Value>(&text) else { return };
    let flat = flat_modules(&cfg);
    let list: Vec<String> = flat
        .iter()
        .map(|(s, m)| format!("{}   — {} side", module_pretty_name(m), side_label(s)))
        .collect();
    let list_str = list.join("\n") + "\n";
    let Some(i1_raw) = rofi(
        &list_str,
        &[
            "-i", "-format", "i", "-p", "󰜬 Move",
            "-mesg", "Step 1/2 — Which button do you want to move?",
            "-theme-str", "listview { lines: 17; } window { width: 560px; }",
        ],
    ) else {
        return;
    };
    let Ok(i1) = i1_raw.trim().parse::<usize>() else { return };
    if i1 >= flat.len() {
        return;
    }
    // sed 's/ *—.*//' — strip from " —" onwards, and trailing spaces before it
    let moved = list[i1].split('—').next().unwrap_or("").trim_end().to_string();
    let step2 = format!(
        "{list_str}󰁔  …to the END of the LEFT side\n󰁔  …to the END of the CENTER\n󰁔  …to the END of the RIGHT side\n"
    );
    let mesg =
        format!("Step 2/2 — Where should「{moved}」go? (it will be placed BEFORE what you pick)");
    let Some(i2_raw) = rofi(
        &step2,
        &[
            "-i", "-format", "i", "-p", "󰜬 Place",
            "-mesg", &mesg,
            "-theme-str", "listview { lines: 20; } window { width: 560px; }",
        ],
    ) else {
        return;
    };
    let Ok(i2) = i2_raw.trim().parse::<usize>() else { return };
    apply_reorder(&mut cfg, i1, i2);
    if std::fs::write(&cfg_path, to_json_indent4(&cfg)).is_err() {
        return;
    }
    restart_bar();
    if autohide_running() {
        util::notify(&["Waybar", "Reordered (bar is in auto-hide — touch top edge)"]);
    }
}

// ---------- settings / launcher windows (focus-or-spawn) ----------

fn find_client_addr(class: &str, title: &str) -> Option<String> {
    // exact class+title match (a browser tab named "Hypr Settings" must not count)
    hypr::clients().into_iter().find_map(|c| {
        if c.get("class").and_then(Value::as_str) == Some(class)
            && c.get("title").and_then(Value::as_str) == Some(title)
        {
            c.get("address").and_then(Value::as_str).map(String::from)
        } else {
            None
        }
    })
}

fn settings_panel(page: &str) {
    if let Some(addr) = find_client_addr("floatterm", "Hypr Settings") {
        if !page.is_empty() {
            let _ = std::fs::write(util::xdg_runtime().join("hypr-settings.page"), page);
        }
        hypr::dispatch(&format!("focuswindow address:{addr}"));
    } else {
        let bin = util::local_bin("hypr-settings").to_string_lossy().into_owned();
        let mut cmd = vec![
            "kitty", "--class", "floatterm", "-o", "background_opacity=0.93",
            "--title", "Hypr Settings", bin.as_str(),
        ];
        if !page.is_empty() {
            cmd.push(page);
        }
        util::spawn_detached(&cmd);
    }
}

fn launcher(mode: &str) {
    let title = match mode {
        "windows" => "Hypr Windows",
        "wallpaper" => "Hypr Wallpaper",
        "menu" => "Hypr Tools",
        _ => "Hypr Apps",
    };
    if let Some(addr) = find_client_addr("hyprlauncher", title) {
        hypr::dispatch(&format!("focuswindow address:{addr}"));
    } else {
        let bin = util::local_bin("hypr-launcher").to_string_lossy().into_owned();
        util::spawn_detached(&[
            "kitty", "--class", "hyprlauncher", "-o", "background_opacity=0.93",
            "--title", title, &bin, mode,
        ]);
    }
}

// ---------- main menu (legacy rofi hub) ----------

fn main_menu() {
    let autohide_state = if autohide_running() { "ON → turn OFF" } else { "OFF → turn ON" };
    let input = format!(
        "  Settings — control panel (click & pick) ALT+X\n\
󰀻  Apps — launcher                         ALT+R\n\
󰉋  Apps — file manager                     ALT+E\n\
󰖯  Apps — window switcher                  ALT+W\n\
󰸉  Wallpaper — pick image / folder / monitor\n\
󰒝  Wallpaper — random                      ALT+SHIFT+W\n\
⏰  Reminder — set new                      ALT+T\n\
󰃰  Reminder — list / cancel\n\
󰅐  Clock — time, timezone & sync\n\
󰕥  Security — firewall (ufw)\n\
󰃤  Security — antivirus scan (ClamAV)\n\
󰌌  Keybinds — view / edit / learn          ALT+K\n\
󰘸  Workspace — hide/unhide all windows     ALT+A\n\
󰖰  Window — hide focused (minimize)        ALT+SHIFT+A\n\
󰖯  Window — unhide… (pick from list)       ALT+CTRL+A\n\
󰜬  Bar — reorder buttons\n\
󰊠  Bar — hide/show now                     ALT+B\n\
󰗕  Bar — auto-hide: {autohide_state}\n\
  Config — edit Hyprland\n\
  Config — edit bar (waybar)\n\
󰑓  Config — reload Hyprland\n"
    );
    let Some(pick) = rofi(
        &input,
        &[
            "-i", "-p", "󰍜 Tools",
            "-mesg",
            "Type to search: <b>wall</b>, <b>bar</b>, <b>remind</b>, <b>key</b>… — every tool is here",
            "-theme-str", "listview { lines: 16; } window { width: 640px; }",
        ],
    ) else {
        return;
    };
    // bash `case` order preserved
    if pick.contains("Settings — control") {
        settings_panel("");
    } else if pick.contains("Apps — launcher") {
        launcher("apps");
    } else if pick.contains("file manager") {
        util::spawn_detached(&["thunar"]);
    } else if pick.contains("window switcher") {
        launcher("windows");
    } else if pick.contains("Wallpaper — pick") {
        launcher("wallpaper");
    } else if pick.contains("Wallpaper — random") {
        wallpaper::run(&[]);
    } else if pick.contains("Reminder — set") {
        new_reminder();
    } else if pick.contains("Reminder — list") {
        list_reminders();
    } else if pick.contains("Clock —") {
        clock_menu();
    } else if pick.contains("firewall") {
        firewall_menu();
    } else if pick.contains("antivirus") {
        clamav_menu();
    } else if pick.contains("Keybinds") {
        edit_keybinds();
    } else if pick.contains("Workspace —") {
        stash_toggle();
    } else if pick.contains("Window — hide") {
        hide_window();
    } else if pick.contains("Window — unhide") {
        unhide_window();
    } else if pick.contains("Bar — reorder") {
        reorder_modules();
    } else if pick.contains("Bar — hide/show") {
        crate::applets::bar_toggle::run();
    } else if pick.contains("Bar — auto-hide") {
        toggle_autohide();
    } else if pick.ends_with("edit Hyprland") {
        edit_hypr();
    } else if pick.contains("edit bar") {
        edit_bar();
    } else if pick.ends_with("reload Hyprland") {
        hypr::reload();
        util::notify(&["Hyprland", "Config reloaded ✔"]);
    }
}

fn edit_hypr() {
    util::spawn_detached(&[
        "kitty",
        "--title",
        "hyprland.conf",
        "sh",
        "-c",
        &format!("${{EDITOR:-nvim}} '{}'; hyprctl reload", conf_path().display()),
    ]);
}

fn edit_bar() {
    let home = util::home();
    util::spawn_detached(&[
        "kitty",
        "--title",
        "waybar config",
        "sh",
        "-c",
        &format!(
            "${{EDITOR:-nvim}} '{0}/.config/waybar/config' '{0}/.config/waybar/style.css'; '{1}' restart-bar",
            home.display(),
            tools_sh()
        ),
    ]);
}

pub fn run(args: &[&str]) -> ExitCode {
    match args.first().copied().unwrap_or("menu") {
        "settings" => settings_panel(args.get(1).copied().unwrap_or("")),
        "power" => settings_panel("power"), // power button & ALT+ESC open the Power page
        "power-menu" => power_menu(),       // old rofi power menu, still available
        "apps" => launcher("apps"),
        "windows" => launcher("windows"),
        "reorder" => reorder_modules(),
        "autohide" => toggle_autohide(),
        "wallpaper" => launcher("wallpaper"),
        "wallpaper-rofi" => pick_wallpaper(), // old rofi thumbnail picker, still available
        "reminders" => list_reminders(),
        "edit-hypr" => edit_hypr(),
        "edit-bar" => edit_bar(),
        "reload" => {
            hypr::reload();
            util::notify(&["Hyprland", "Config reloaded ✔"]);
        }
        "menu-rofi" => main_menu(), // old rofi tools menu, still available
        "random" => {
            return wallpaper::run(&[]);
        }
        "keys" => edit_keybinds(),
        "restart-bar" => restart_bar(),
        "clock" => clock_menu(),
        "remind" => new_reminder(),
        "stash" => stash_toggle(),
        "hide-window" => hide_window(),
        "unhide-window" => unhide_window(),
        "firewall" => firewall_menu(),
        "clamav" => clamav_menu(),
        _ => launcher("menu"), // ALT+D / bar Tools button -> tile UI
    }
    util::ok_exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_time_matches_bash_regex() {
        for ok in ["5m", "45m", "3h", "10s", "120m"] {
            assert!(is_rel_time(ok), "{ok}");
        }
        for bad in ["m", "5", "5d", "5 m", "h5", "", "1.5h"] {
            assert!(!is_rel_time(bad), "{bad}");
        }
    }

    #[test]
    fn clock_time_matches_bash_regex() {
        assert_eq!(parse_clock_time("17:30"), Some((17, 30)));
        assert_eq!(parse_clock_time("9:05"), Some((9, 5)));
        assert_eq!(parse_clock_time("23:59"), Some((23, 59)));
        assert_eq!(parse_clock_time("24:00"), None);
        assert_eq!(parse_clock_time("17:60"), None);
        assert_eq!(parse_clock_time("17:5"), None); // minutes must be 2 digits
        assert_eq!(parse_clock_time("175:30"), None);
        assert_eq!(parse_clock_time("17-30"), None);
    }

    #[test]
    fn keybind_display_transforms_like_sed() {
        assert_eq!(keybind_display("114:bind = $mod, R, exec, tool"), "114:ALT , R, exec, tool");
        assert_eq!(
            keybind_display("20:bind = $mod SHIFT, S, exec, shot"),
            "20:ALT+SHIFT , S, exec, shot"
        );
        assert_eq!(keybind_display("7:bindel = $mod CTRL, A, exec, x"), "7:ALT+CTRL , A, exec, x");
    }

    #[test]
    fn bind_line_matcher_mirrors_grep() {
        assert!(is_bind_line("bind = $mod, Q, exec, kitty"));
        assert!(is_bind_line("  bindel = ,XF86AudioRaiseVolume, exec, x"));
        assert!(is_bind_line("bindm = $mod, mouse:272, movewindow"));
        assert!(!is_bind_line("# bind = commented"));
        assert!(!is_bind_line("binds { }"));
        assert!(!is_bind_line("monitor = DP-1"));
    }

    fn cfg_fixture() -> Value {
        json!({
            "modules-left": ["custom/menu", "hyprland/workspaces"],
            "modules-center": ["clock"],
            "modules-right": ["cpu", "custom/power"],
            "other-key": {"keep": true}
        })
    }

    #[test]
    fn reorder_moves_before_target() {
        let mut cfg = cfg_fixture();
        // move "custom/menu" (0) before "cpu" (3)
        apply_reorder(&mut cfg, 0, 3);
        assert_eq!(cfg["modules-left"], json!(["hyprland/workspaces"]));
        assert_eq!(cfg["modules-right"], json!(["custom/menu", "cpu", "custom/power"]));
    }

    #[test]
    fn reorder_before_itself_is_noop_back_to_end() {
        let mut cfg = cfg_fixture();
        apply_reorder(&mut cfg, 2, 2); // "clock" before itself
        assert_eq!(cfg["modules-center"], json!(["clock"]));
    }

    #[test]
    fn reorder_to_end_of_side() {
        let mut cfg = cfg_fixture();
        let n = flat_modules(&cfg).len(); // 5
        apply_reorder(&mut cfg, 2, n); // clock -> END of LEFT
        assert_eq!(
            cfg["modules-left"],
            json!(["custom/menu", "hyprland/workspaces", "clock"])
        );
        assert_eq!(cfg["modules-center"], json!([]));
        let mut cfg2 = cfg_fixture();
        apply_reorder(&mut cfg2, 0, n + 2); // menu -> END of RIGHT
        assert_eq!(cfg2["modules-right"], json!(["cpu", "custom/power", "custom/menu"]));
    }

    #[test]
    fn reorder_same_section_earlier_target_matches_python() {
        // python removes src first, then inserts at tgt's NEW index
        let mut cfg = json!({
            "modules-left": ["a", "b", "c"],
            "modules-center": [],
            "modules-right": []
        });
        apply_reorder(&mut cfg, 2, 0); // move "c" before "a"
        assert_eq!(cfg["modules-left"], json!(["c", "a", "b"]));
        let mut cfg2 = json!({
            "modules-left": ["a", "b", "c"],
            "modules-center": [],
            "modules-right": []
        });
        apply_reorder(&mut cfg2, 0, 2); // move "a" before "c" (c shifts to idx 1 after removal)
        assert_eq!(cfg2["modules-left"], json!(["b", "a", "c"]));
    }

    #[test]
    fn json_indent4_and_key_order_preserved() {
        let out = to_json_indent4(&cfg_fixture());
        assert!(
            out.starts_with("{\n    \"modules-left\": [\n        \"custom/menu\","),
            "got: {}",
            &out[..80.min(out.len())]
        );
        let l = out.find("modules-left").unwrap();
        let c = out.find("modules-center").unwrap();
        let r = out.find("modules-right").unwrap();
        assert!(l < c && c < r);
    }

    #[test]
    fn module_names_cover_the_python_table() {
        assert_eq!(module_pretty_name("clock"), "\u{f0954}  Clock");
        assert_eq!(module_pretty_name("unknown/x"), "unknown/x");
    }

    #[test]
    fn reminder_list_line_parsing() {
        let sel = "Buy milk │ Wed 2026-07-16 18:00:00 │ reminder-123.timer";
        let unit = sel.rsplit('│').next().unwrap().trim();
        assert_eq!(unit, "reminder-123.timer");
        let cancelled = sel.split(" │").next().unwrap();
        assert_eq!(cancelled, "Buy milk");
    }
}
