//! End-to-end tests: run the real `justlinux` binary against a fake
//! Hyprland IPC socket and stubbed external commands (notify-send, grim,
//! systemctl, …). This is the same idea as the python suite's
//! HYPRSETTINGS_DRYRUN pilot harness, but exercising the actual process
//! boundary: argv[0] dispatch, socket protocol, file writes, exit codes.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct World {
    home: PathBuf,
    runtime: PathBuf,
    stubs: PathBuf,
    stub_log: PathBuf,
    /// every request the fake Hyprland socket received
    requests: Arc<Mutex<Vec<String>>>,
}

const SIG: &str = "TESTSIG";

/// Canned JSON the fake compositor serves, per test scenario.
#[derive(Clone, Default)]
struct Compositor {
    active_workspace: String,
    active_window: String,
    clients: String,
    monitors: String,
    cursorpos: String,
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_justlinux")
}

impl World {
    fn new(name: &str, comp: Compositor) -> World {
        let base = std::env::temp_dir().join(format!("jl-it-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let runtime = base.join("runtime");
        let stubs = base.join("stubs");
        std::fs::create_dir_all(home.join(".config/hypr")).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::fs::create_dir_all(&stubs).unwrap();
        let sockdir = runtime.join("hypr").join(SIG);
        std::fs::create_dir_all(&sockdir).unwrap();
        let stub_log = base.join("stub.log");

        // fake Hyprland IPC socket
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let listener = UnixListener::bind(sockdir.join(".socket.sock")).unwrap();
        let req_clone = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                req_clone.lock().unwrap().push(req.clone());
                let resp: String = if req == "j/activeworkspace" {
                    comp.active_workspace.clone()
                } else if req == "j/activewindow" {
                    comp.active_window.clone()
                } else if req == "j/clients" {
                    comp.clients.clone()
                } else if req == "j/monitors" {
                    comp.monitors.clone()
                } else if req == "cursorpos" {
                    comp.cursorpos.clone()
                } else {
                    "ok".to_string()
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });

        World { home, runtime, stubs, stub_log, requests }
    }

    /// Install a stub command that logs its argv (\x1f-separated fields,
    /// \x1e-terminated records — args may contain newlines) and runs
    /// `extra` shell code.
    fn stub(&self, name: &str, extra: &str) {
        let path = self.stubs.join(name);
        let script = format!(
            "#!/bin/bash\nprintf '%s\\x1f' \"$(basename \"$0\")\" \"$@\" >> \"$STUB_LOG\"\nprintf '\\x1e' >> \"$STUB_LOG\"\n{extra}\n"
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn cmd(&self, applet: &str, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        c.arg(applet).args(args);
        c.env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("HYPRLAND_INSTANCE_SIGNATURE", SIG)
            .env("STUB_LOG", &self.stub_log)
            .env("PATH", format!("{}:/usr/bin:/bin", self.stubs.display()))
            .env_remove("HYPRSETTINGS_DRYRUN");
        c
    }

    fn stub_calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(&self.stub_log)
            .unwrap_or_default()
            .split('\x1e')
            .filter(|r| !r.is_empty())
            .map(|r| r.trim_end_matches('\x1f').split('\x1f').map(String::from).collect())
            .collect()
    }

    /// Wait (≤3s) for a stub call whose program name is `name`.
    fn wait_stub_call(&self, name: &str) -> Option<Vec<String>> {
        for _ in 0..30 {
            if let Some(c) = self.stub_calls().into_iter().find(|c| c[0] == name) {
                return Some(c);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    fn dispatches(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("dispatch "))
            .cloned()
            .collect()
    }
}

fn ws(id: i64, name: &str) -> String {
    format!(r#"{{"id":{id},"name":"{name}","monitor":"DP-1"}}"#)
}

/// Wait until a process with the given comm is visible in /proc (the
/// daemons find waybar exactly this way; under parallel test load a bash
/// script can take a while to appear).
fn wait_for_comm(name: &str) {
    for _ in 0..50 {
        let found = std::fs::read_dir("/proc").ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                std::fs::read_to_string(e.path().join("comm"))
                    .map(|c| c.trim_end() == name)
                    .unwrap_or(false)
            })
        });
        if found {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("process with comm {name} never appeared");
}

fn client(addr: &str, ws_id: i64, ws_name: &str, at: (i64, i64), title: &str, class: &str) -> String {
    format!(
        r#"{{"address":"{addr}","at":[{},{}],"workspace":{{"id":{ws_id},"name":"{ws_name}"}},"title":"{title}","class":"{class}"}}"#,
        at.0, at.1
    )
}

// ---------------- status applets ----------------

#[test]
fn av_status_reports_running_daemon() {
    let w = World::new("av-on", Compositor::default());
    w.stub("systemctl", "exit 0");
    let out = w.cmd("av-status", &[]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"{"text":"󰃤","class":"on","tooltip":"Antivirus (ClamAV): daemon running — click to scan"}"#
    );
}

#[test]
fn av_status_reports_stopped_daemon() {
    let w = World::new("av-off", Compositor::default());
    w.stub("systemctl", "exit 3");
    let out = w.cmd("av-status", &[]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"{"text":"󰃤","class":"off","tooltip":"Antivirus (ClamAV): daemon NOT running — click for options"}"#
    );
}

#[test]
fn fw_status_off_without_ufw_conf() {
    // container has no /etc/ufw/ufw.conf ENABLED=yes → OFF branch
    let w = World::new("fw", Compositor::default());
    w.stub("systemctl", "exit 0");
    let out = w.cmd("fw-status", &[]).output().unwrap();
    let line = String::from_utf8_lossy(&out.stdout);
    assert!(line.contains(r#""class":"off""#), "{line}");
    assert!(line.contains("Firewall (ufw)"));
}

// ---------------- screenshot ----------------

#[test]
fn screenshot_screen_uses_ipc_monitor_and_notifies() {
    let comp = Compositor {
        active_workspace: ws(3, "3"),
        ..Default::default()
    };
    let w = World::new("shot-screen", comp);
    w.stub("grim", "touch \"${@: -1}\""); // create the output file
    w.stub("wl-copy", "cat > /dev/null");
    w.stub("notify-send", "");
    let out = w.cmd("screenshot", &["screen"]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let calls = w.stub_calls();
    let grim = calls.iter().find(|c| c[0] == "grim").expect("grim called");
    assert_eq!(grim[1], "-o");
    assert_eq!(grim[2], "DP-1"); // monitor came from the fake IPC socket
    assert!(grim[3].ends_with(".png"));
    let file = Path::new(&grim[3]);
    assert!(file.exists());
    assert!(file.starts_with(w.home.join("Pictures/Screenshots")));
    let notify = calls.iter().find(|c| c[0] == "notify-send").expect("notified");
    assert!(notify.iter().any(|a| a == "Screenshot (this screen)"));
    assert!(
        notify.iter().any(|a| a.contains("Copied to clipboard + saved:\n~/Pictures/Screenshots/")),
        "{notify:?}"
    );
}

#[test]
fn screenshot_region_cancelled_by_slurp_esc_exits_zero_silently() {
    let w = World::new("shot-cancel", Compositor::default());
    w.stub("slurp", "exit 1"); // user pressed Esc
    w.stub("grim", "touch \"${@: -1}\"");
    w.stub("notify-send", "");
    let out = w.cmd("screenshot", &["region"]).output().unwrap();
    assert!(out.status.success());
    let calls = w.stub_calls();
    assert!(!calls.iter().any(|c| c[0] == "grim"), "grim must not run");
    assert!(!calls.iter().any(|c| c[0] == "notify-send"));
}

#[test]
fn screenshot_all_captures_everything() {
    let w = World::new("shot-all", Compositor::default());
    w.stub("grim", "touch \"${@: -1}\"");
    w.stub("wl-copy", "cat > /dev/null");
    w.stub("notify-send", "");
    let out = w.cmd("screenshot", &["all"]).output().unwrap();
    assert!(out.status.success());
    let calls = w.stub_calls();
    let grim = calls.iter().find(|c| c[0] == "grim").unwrap();
    assert_eq!(grim.len(), 2); // grim <file> — no -o/-g
    let notify = calls.iter().find(|c| c[0] == "notify-send").unwrap();
    assert!(notify.iter().any(|a| a == "Screenshot (all screens)"));
}

// ---------------- stash / hide / unhide ----------------

#[test]
fn stash_hides_windows_in_reading_order_and_saves_state() {
    let clients = format!(
        "[{},{},{}]",
        client("0xaa", 4, "4", (800, 0), "right-top", "kitty"),
        client("0xbb", 4, "4", (0, 0), "left-top", "brave"),
        client("0xcc", 2, "2", (0, 0), "other-ws", "thunar"),
    );
    let comp = Compositor {
        active_workspace: ws(4, "4"),
        clients,
        ..Default::default()
    };
    let w = World::new("stash-hide", comp);
    w.stub("notify-send", "");
    let out = w.cmd("hypr-tools", &["stash"]).output().unwrap();
    assert!(out.status.success());
    // reading order: (y, x) → 0xbb (0,0) then 0xaa (800,0); 0xcc untouched
    assert_eq!(
        w.dispatches(),
        vec![
            "dispatch movetoworkspacesilent special:stash4,address:0xbb",
            "dispatch movetoworkspacesilent special:stash4,address:0xaa",
        ]
    );
    let state: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(w.runtime.join("hypr-stash-4.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state, vec!["0xbb", "0xaa"]);
    let notify = w.stub_calls().into_iter().find(|c| c[0] == "notify-send").unwrap();
    assert!(notify.iter().any(|a| a.contains("Hid 2 window(s)")), "{notify:?}");
}

#[test]
fn stash_restores_in_saved_order() {
    let clients = format!(
        "[{},{}]",
        client("0xaa", 99, "special:stash4", (800, 0), "a", "kitty"),
        client("0xbb", 99, "special:stash4", (0, 0), "b", "brave"),
    );
    let comp = Compositor {
        active_workspace: ws(4, "4"),
        clients,
        ..Default::default()
    };
    let w = World::new("stash-restore", comp);
    w.stub("notify-send", "");
    // saved order: bb first
    std::fs::write(w.runtime.join("hypr-stash-4.json"), r#"["0xbb","0xaa"]"#).unwrap();
    let out = w.cmd("hypr-tools", &["stash"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        w.dispatches(),
        vec![
            "dispatch movetoworkspacesilent 4,address:0xbb",
            "dispatch movetoworkspacesilent 4,address:0xaa",
        ]
    );
    assert!(!w.runtime.join("hypr-stash-4.json").exists(), "state file removed");
    let notify = w.stub_calls().into_iter().find(|c| c[0] == "notify-send").unwrap();
    assert!(notify.iter().any(|a| a.contains("Restored 2 window(s) on workspace 4")));
}

#[test]
fn stash_refuses_on_special_workspace() {
    let comp = Compositor {
        active_workspace: r#"{"id":-99,"name":"special:stash4","monitor":"DP-1"}"#.into(),
        clients: "[]".into(),
        ..Default::default()
    };
    let w = World::new("stash-special", comp);
    w.stub("notify-send", "");
    let out = w.cmd("hypr-tools", &["stash"]).output().unwrap();
    assert!(out.status.success());
    assert!(w.dispatches().is_empty());
    let notify = w.stub_calls().into_iter().find(|c| c[0] == "notify-send").unwrap();
    assert!(notify.iter().any(|a| a.contains("go to a normal workspace first")));
}

#[test]
fn hide_window_saves_state_and_moves_to_hidden() {
    let comp = Compositor {
        active_window: client("0xdd", 4, "4", (0, 0), "My Editor", "nvim"),
        ..Default::default()
    };
    let w = World::new("hide-win", comp);
    w.stub("notify-send", "");
    let out = w.cmd("hypr-tools", &["hide-window"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        w.dispatches(),
        vec!["dispatch movetoworkspacesilent special:hidden,address:0xdd"]
    );
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(w.runtime.join("hypr-hidden.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["0xdd"]["ws"], 4);
    assert_eq!(state["0xdd"]["title"], "My Editor");
    assert_eq!(state["0xdd"]["cls"], "nvim");
}

#[test]
fn unhide_window_restores_to_saved_workspace_via_rofi_pick() {
    let clients = format!(
        "[{},{}]",
        client("0xee", -98, "special:hidden", (0, 0), "Hidden One", "kitty"),
        client("0xff", -98, "special:hidden", (0, 0), "Hidden Two", "brave"),
    );
    let comp = Compositor {
        active_workspace: ws(1, "1"),
        clients,
        ..Default::default()
    };
    let w = World::new("unhide-win", comp);
    w.stub("notify-send", "");
    w.stub("rofi", "cat > /dev/null; echo 1"); // pick index 1 (Hidden Two)
    std::fs::write(
        w.runtime.join("hypr-hidden.json"),
        r#"{"0xee":{"ws":4,"title":"Hidden One","cls":"kitty"},"0xff":{"ws":7,"title":"Hidden Two","cls":"brave"}}"#,
    )
    .unwrap();
    let out = w.cmd("hypr-tools", &["unhide-window"]).output().unwrap();
    assert!(out.status.success());
    // follow=true → movetoworkspace (not silent), to saved ws 7
    assert_eq!(w.dispatches(), vec!["dispatch movetoworkspace 7,address:0xff"]);
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(w.runtime.join("hypr-hidden.json")).unwrap(),
    )
    .unwrap();
    assert!(state.get("0xff").is_none(), "restored entry removed");
    assert!(state.get("0xee").is_some(), "other entry kept");
}

#[test]
fn unhide_restore_all_uses_silent_moves() {
    let clients = format!(
        "[{},{}]",
        client("0xee", -98, "special:hidden", (0, 0), "One", "kitty"),
        client("0xff", -98, "special:hidden", (0, 0), "Two", "brave"),
    );
    let comp = Compositor {
        active_workspace: ws(1, "1"),
        clients,
        ..Default::default()
    };
    let w = World::new("unhide-all", comp);
    w.stub("notify-send", "");
    w.stub("rofi", "cat > /dev/null; echo 2"); // index 2 = "Restore ALL"
    let out = w.cmd("hypr-tools", &["unhide-window"]).output().unwrap();
    assert!(out.status.success());
    // no saved state → restored to active ws 1, silent
    assert_eq!(
        w.dispatches(),
        vec![
            "dispatch movetoworkspacesilent 1,address:0xee",
            "dispatch movetoworkspacesilent 1,address:0xff",
        ]
    );
    let notify = w.stub_calls().into_iter().find(|c| c[0] == "notify-send").unwrap();
    assert!(notify.iter().any(|a| a.contains("2 window(s) back on their workspaces")));
}

// ---------------- wallpaper ----------------

#[test]
fn wallpaper_writes_conf_and_recolors() {
    let _guard = SIGNAL_TESTS.lock().unwrap_or_else(|p| p.into_inner()); // sends SIGUSR2 to any comm "waybar"
    let comp = Compositor {
        monitors: r#"[{"name":"DP-1","x":0,"y":0,"focused":true},{"name":"HDMI-A-1","x":1600,"y":0,"focused":false}]"#.into(),
        ..Default::default()
    };
    let w = World::new("wp-all", comp);
    w.stub("hyprctl", ""); // hyprpaper subcommand goes through the CLI
    w.stub("wallust", "");
    w.stub("swaync-client", "");
    let img = w.home.join("Pictures/wallpaper/blue.png");
    std::fs::create_dir_all(img.parent().unwrap()).unwrap();
    std::fs::write(&img, "png").unwrap();
    let out = w.cmd("wallpaper", &[img.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Wallpaper: "), "{stdout}");
    assert!(stdout.contains("Desktop recolored ✔"));

    let conf = std::fs::read_to_string(w.home.join(".config/hypr/hyprpaper.conf")).unwrap();
    assert!(conf.contains("# Managed by wallpaper.sh"));
    assert!(conf.contains(&format!("path     = {}", img.display())));
    assert!(conf.contains("splash = false"));

    let calls = w.stub_calls();
    // hyprpaper: fallback "," + one per monitor
    let hp: Vec<&Vec<String>> = calls.iter().filter(|c| c[0] == "hyprctl").collect();
    assert_eq!(hp.len(), 3, "{hp:?}");
    assert!(hp[0][2].starts_with("wallpaper"));
    assert!(calls.iter().any(|c| c[0] == "wallust" && c[1] == "run"));
    // native IPC reload happened
    assert!(w.requests.lock().unwrap().iter().any(|r| r == "reload"));
}

#[test]
fn wallpaper_single_monitor_keeps_other_assignments() {
    let _guard = SIGNAL_TESTS.lock().unwrap_or_else(|p| p.into_inner()); // sends SIGUSR2 to any comm "waybar"
    let comp = Compositor::default();
    let w = World::new("wp-one", comp);
    w.stub("hyprctl", "");
    w.stub("wallust", "");
    w.stub("swaync-client", "");
    std::fs::write(
        w.home.join(".config/hypr/hyprpaper.conf"),
        "wallpaper {\n    monitor  = \n    path     = /old-fallback.png\n    fit_mode = cover\n}\nwallpaper {\n    monitor  = HDMI-A-1\n    path     = /old-hdmi.png\n    fit_mode = cover\n}\n",
    )
    .unwrap();
    let img = w.home.join("pic.jpg");
    std::fs::write(&img, "jpg").unwrap();
    let out = w.cmd("wallpaper", &[img.to_str().unwrap(), "DP-1"]).output().unwrap();
    assert!(out.status.success());
    let conf = std::fs::read_to_string(w.home.join(".config/hypr/hyprpaper.conf")).unwrap();
    assert!(conf.contains("path     = /old-fallback.png"), "fallback kept:\n{conf}");
    assert!(conf.contains("path     = /old-hdmi.png"), "other monitor kept:\n{conf}");
    let canonical = std::fs::canonicalize(&img).unwrap();
    assert!(
        conf.contains(&format!("path     = {}", canonical.display())),
        "new DP-1 assignment:\n{conf}"
    );
}

#[test]
fn wallpaper_rejects_missing_file() {
    let w = World::new("wp-missing", Compositor::default());
    let out = w.cmd("wallpaper", &["/no/such/image.png"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No such image or folder"));
}

// ---------------- reorder ----------------

#[test]
fn reorder_moves_module_and_restarts_bar() {
    let _guard = SIGNAL_TESTS.lock().unwrap_or_else(|p| p.into_inner()); // restart_bar pkills any comm "waybar"
    let w = World::new("reorder", Compositor::default());
    w.stub("notify-send", "");
    // rofi: first call → move item 0; second call → before item 3
    w.stub(
        "rofi",
        r#"cat > /dev/null
COUNT_FILE="$STUB_LOG.rofi-count"
N=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
echo $((N+1)) > "$COUNT_FILE"
if [ "$N" = 0 ]; then echo 0; else echo 3; fi"#,
    );
    std::fs::create_dir_all(w.home.join(".config/waybar")).unwrap();
    std::fs::write(
        w.home.join(".config/waybar/config"),
        r#"{
    "layer": "top",
    "modules-left": ["custom/menu", "hyprland/workspaces"],
    "modules-center": ["clock"],
    "modules-right": ["cpu", "custom/power"],
    "clock": {"format": "{:%H:%M}"}
}"#,
    )
    .unwrap();
    let out = w.cmd("hypr-tools", &["reorder"]).output().unwrap();
    assert!(out.status.success());
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(w.home.join(".config/waybar/config")).unwrap())
            .unwrap();
    assert_eq!(cfg["modules-left"], serde_json::json!(["hyprland/workspaces"]));
    assert_eq!(cfg["modules-right"], serde_json::json!(["custom/menu", "cpu", "custom/power"]));
    // non-module keys survive the rewrite, in order
    assert_eq!(cfg["layer"], "top");
    assert_eq!(cfg["clock"]["format"], "{:%H:%M}");
    // bar restarted via IPC
    assert!(w.requests.lock().unwrap().iter().any(|r| r == "dispatch exec waybar"));
}

// ---------------- bar toggle & autohide daemon ----------------

/// Tests that create processes with comm "waybar" OR send signals by comm
/// scan (restart-bar's pkill, wallpaper's SIGUSR2) interfere across worlds
/// exactly like real pkill would — run them one at a time.
static SIGNAL_TESTS: Mutex<()> = Mutex::new(());

#[test]
fn bar_toggle_signals_waybar_when_no_daemon() {
    let _guard = SIGNAL_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let w = World::new("bar-toggle", Compositor::default());
    // fake waybar: a bash script named "waybar" that logs SIGUSR1
    let log = w.home.join("usr1.log");
    let waybar = w.stubs.join("waybar");
    std::fs::write(
        &waybar,
        format!(
            "#!/bin/bash\ntrap 'echo usr1 >> {}' USR1\nfor i in $(seq 1 100); do sleep 0.1; done\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&waybar, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut wb = Command::new(&waybar).spawn().unwrap();
    wait_for_comm("waybar");
    std::thread::sleep(Duration::from_millis(200)); // let the trap install

    let out = w.cmd("bar-toggle", &[]).output().unwrap();
    assert!(out.status.success());
    // give the trap a moment (signal lands after the current sleep tick)
    let mut seen = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if std::fs::read_to_string(&log).map(|s| s.contains("usr1")).unwrap_or(false) {
            seen = true;
            break;
        }
    }
    let _ = wb.kill();
    let _ = wb.wait();
    assert!(seen, "waybar received SIGUSR1");
}

#[test]
fn autohide_daemon_hides_shows_and_pins() {
    let _guard = SIGNAL_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let comp = Compositor {
        cursorpos: "512, 300".into(), // cursor well below the bar
        ..Default::default()
    };
    let w = World::new("autohide", comp);
    let log = w.home.join("usr1.log");
    let waybar = w.stubs.join("waybar");
    std::fs::write(
        &waybar,
        format!(
            "#!/bin/bash\ntrap 'echo toggle >> {}' USR1\nfor i in $(seq 1 200); do sleep 0.1; done\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&waybar, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut wb = Command::new(&waybar).spawn().unwrap();
    wait_for_comm("waybar");
    std::thread::sleep(Duration::from_millis(200)); // let the trap install

    // run the daemon through its symlink: exercises argv[0] dispatch AND
    // keeps the `pgrep -f waybar-autohide.sh` cmdline contract observable
    let link = w.home.join(".local/bin/waybar-autohide.sh");
    std::os::unix::fs::symlink(bin(), &link).unwrap();
    let mut daemon_cmd = Command::new(&link);
    daemon_cmd
        .env("HOME", &w.home)
        .env("XDG_RUNTIME_DIR", &w.runtime)
        .env("HYPRLAND_INSTANCE_SIGNATURE", SIG)
        .env("PATH", format!("{}:/usr/bin:/bin", w.stubs.display()));
    let mut daemon = daemon_cmd.spawn().unwrap();
    let toggles = |log: &Path| {
        std::fs::read_to_string(log).unwrap_or_default().lines().count()
    };
    // startup hides the bar → 1 toggle
    let mut ok = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if toggles(&log) == 1 {
            ok = true;
            break;
        }
    }
    assert!(ok, "bar hidden on startup (1 toggle), got {}", toggles(&log));

    // SIGUSR1 → hidden → show + pin → 2 toggles
    unsafe { libc::kill(daemon.id() as i32, libc::SIGUSR1) };
    ok = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if toggles(&log) == 2 {
            ok = true;
            break;
        }
    }
    assert!(ok, "SIGUSR1 shows+pins (2 toggles), got {}", toggles(&log));

    // pinned: cursor at 300 must NOT hide it again
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(toggles(&log), 2, "pinned bar stays visible");

    // SIGTERM → daemon exits, bar left visible (already visible → no extra toggle)
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    let mut exited = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(_)) = daemon.try_wait() {
            exited = true;
            break;
        }
    }
    let _ = wb.kill();
    let _ = wb.wait();
    if !exited {
        let _ = daemon.kill();
        let _ = daemon.wait();
    }
    assert!(exited, "daemon exits on SIGTERM");
    assert_eq!(toggles(&log), 2, "bar left visible on daemon stop");
}

// ---------------- keybind sheet through the dispatcher ----------------

#[test]
fn keys_menu_builds_cheatsheet_and_opens_editor_at_line() {
    let w = World::new("keys", Compositor::default());
    std::fs::write(
        w.home.join(".config/hypr/hyprland.conf"),
        "monitor = DP-1\nbind = $mod, Q, exec, kitty\nbind = $mod SHIFT, S, exec, shot\n",
    )
    .unwrap();
    // rofi prints its stdin to the log via stdin capture, picks the 2nd line
    w.stub("rofi", r#"INPUT=$(cat); echo "$INPUT" > "$STUB_LOG.rofi-input"; echo "$INPUT" | sed -n 2p"#);
    w.stub("kitty", "");
    let out = w.cmd("hypr-tools", &["keys"]).output().unwrap();
    assert!(out.status.success());
    let rofi_input = std::fs::read_to_string(format!("{}.rofi-input", w.stub_log.display())).unwrap();
    assert_eq!(
        rofi_input.trim(),
        "2:ALT , Q, exec, kitty\n3:ALT+SHIFT , S, exec, shot".trim()
    );
    // kitty is spawned detached — wait for its stub to log
    let kitty = w.wait_stub_call("kitty").expect("kitty spawned");
    let payload = kitty.last().unwrap();
    assert!(payload.contains("+3 "), "editor opens at line 3: {payload}");
    assert!(payload.contains("hyprctl reload"));
}
