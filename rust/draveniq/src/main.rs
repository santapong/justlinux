//! draveniq — a VS-Code-style workspace for Claude Code
//! sessions. bin/draveniq (python/Textual) is the spec;
//! docs/draveniq.md is the contract. Rung 4 — the LAST rung — of
//! the Rust migration: the tmux orchestration ports as subprocess work,
//! the Textual sidebar is hand-rolled in ratatui (sidebar.rs).
//!
//! Modes: (default) focus-or-spawn · --sidebar (inside tmux) ·
//! --split h|v · --palette (inside a display-popup). Every mode needs a
//! branch in main(): an unknown flag falls through to launch(), which
//! spawns a SECOND studio rather than failing quietly.

mod plan;
mod sidebar;

use std::path::Path;
use std::process::Command;

pub const TMUX_SESSION: &str = "draveniq";
const KITTY_CLASS: &str = "draveniq";

pub fn me() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "draveniq".into())
}

pub fn hex(c: hyprdesk::Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Own socket + no user config: the studio must never inherit or restyle
/// the user's real tmux server (plugins, resurrect, theme).
pub fn tmux(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(["-L", TMUX_SESSION])
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn tmux_out(args: &[&str]) -> String {
    Command::new("tmux")
        .args(["-L", TMUX_SESSION])
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn hypr_json(args: &[&str]) -> serde_json::Value {
    Command::new("hyprctl")
        .args(args)
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or(serde_json::Value::Null)
}

// ---------------- default mode: focus or spawn ----------------

fn launch() {
    if let Some(clients) = hypr_json(&["clients", "-j"]).as_array() {
        for c in clients {
            if c.get("class").and_then(|v| v.as_str()) == Some(KITTY_CLASS) {
                // SUMMON the studio to the current workspace (drop-term
                // UX) instead of yanking the user to wherever it was left
                let addr = format!(
                    "address:{}",
                    c.get("address").and_then(|v| v.as_str()).unwrap_or("")
                );
                let ws = hypr_json(&["-j", "activeworkspace"])
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "movetoworkspace", &format!("{ws},{addr}")])
                    .status();
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "focuswindow", &addr])
                    .status();
                let _ = Command::new("hyprctl").args(["dispatch", "centerwindow"]).status();
                return;
            }
        }
    }
    // Studio-only kitty overlay: adds grabbed-aware mouse maps so links and
    // selection still work while tmux owns the mouse. Falls back to the
    // normal config if the overlay was never installed, so a partial install
    // degrades to "as before" rather than a kitty that refuses to start.
    let studio_conf = format!(
        "{}/.config/kitty/draveniq.conf",
        std::env::var("HOME").unwrap_or_default()
    );
    let mut kitty_args: Vec<String> = vec!["kitty".into()];
    if Path::new(&studio_conf).is_file() {
        kitty_args.push("--config".into());
        kitty_args.push(studio_conf);
    }
    let _ = Command::new("setsid")
        .args(&kitty_args)
        .args([
            "--class",
            KITTY_CLASS,
            "--title",
            "DravenIQ Meta Harness",
            "-e",
            "tmux",
            "-f",
            "/dev/null",
            "-L",
            TMUX_SESSION,
            "new-session",
            "-A",
            "-s",
            TMUX_SESSION,
            "-n",
            "sessions",
            &format!("{} --sidebar", me()),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ---------------- tab-bar theme (python style_tmux, verbatim) ----------

// See bin/draveniq for the full commentary on every choice
// here — the ranges, the missing scrollport, the run-shell ✕ path. The
// format strings are copied verbatim; only the interpolation moved.
// every window is a real tab in the per-window-tree model — the old
// "tab 0 is the sidebar" exemptions are gone with it
const CLOSE_X: &str = "#[range=user|x#{window_index}] ✕ #[norange]";

fn pane_n(ink: &str, restore: &str) -> String {
    // the tree pane rides along in the selected window — the bare count
    // subtracts it, and only shows once a REAL second pane exists
    format!(
        "#{{?#{{@beside}},{ink}+#{{=/10/…:@beside}} {restore},\
         #{{?#{{>:#{{window_panes}},2}},{ink}·#{{e|-:#{{window_panes}},1}} {restore},}}}}"
    )
}

fn attn(good: &str, sub: &str, restore: &str) -> String {
    format!(
        "#{{?window_bell_flag,{good}● {restore},\
         #{{?window_activity_flag,{sub}○ {restore},}}}}"
    )
}

pub fn style_tmux() {
    let pal = hyprdesk::colors();
    let (bg, fg) = (hex(pal.bg), hex(pal.fg));
    let (acc, sub) = (hex(pal.accent), hex(pal.sub));
    let (muted, good, warn) = (hex(pal.muted), hex(pal.good), hex(pal.warn));
    let acc2 = hex(pal.accent2);
    let attn_plain = attn(
        &format!("#[fg={good}]"),
        &format!("#[fg={sub}]"),
        &format!("#[fg={sub}]"),
    );

    let pane_n_plain = pane_n(&format!("#[fg={sub}]"), &format!("#[fg={sub}]"));
    let pane_n_cur = pane_n("", "");
    let _ = &warn; // centre uses it via format capture
    let me_ = me();
    let popup_cmd = format!("display-popup -E -w 70% -h 60% \"{me_} --palette\"");
    // every mouse close routes through --close-tab, which rescues the
    // tree pane before the window dies (the handoff's join-pane design)
    let kill_cmd = format!("run-shell '{me_} --close-tab #{{s/^x//:mouse_status_range}}'");
    let split_cmd = format!("run-shell '{me_} --split #{{mouse_status_range}}'");
    let mouse_else = format!(
        "if -F '#{{m/r:^s[hv]$,#{{mouse_status_range}}}}' \
         {{ {split_cmd} }} \
         {{ if -F '#{{==:#{{mouse_status_range}},pal}}' \
         {{ {popup_cmd} }} \
         {{ select-window -t= }} }}"
    );
    let palette_button = format!("#[range=user|pal]#[bg={muted},fg={fg}] / #[default]#[norange]");
    let split_buttons = format!(
        "#[range=user|sh]#[bg={muted},fg={fg}] │ #[default]#[norange] \
         #[range=user|sv]#[bg={muted},fg={fg}] ─ #[default]#[norange]"
    );
    let status_right = format!("{palette_button}  {split_buttons}  #[fg={sub}]#S ");
    let centre = format!(
        "#[fg={sub}]󰉋 #{{=/20/…:#{{b:pane_current_path}}}}\
         #[fg={muted}] · #[fg={fg}]#{{session_windows}}#[fg={sub}] tabs\
         #{{?window_zoomed_flag,#[fg={warn}] [zoom],}}\
         #{{?pane_in_mode,#[fg={acc2}] #{{pane_mode}},}}"
    );
    let ws_format = format!(
        "  #[fg={sub}]#I #[fg={fg}]#W {pane_n_plain}{attn_plain}{CLOSE_X} "
    );
    // the selected tab carries NO attention marks — selecting is what
    // clears them, so a mark here is always stale (design)
    let ws_current = format!(
        "#[bg={acc},fg={bg},bold]  #I #W {pane_n_cur}{CLOSE_X}#[bg={acc},fg={bg}] #[default]"
    );
    let palette_popup = format!("{me_} --palette");
    let sets: Vec<Vec<&str>> = vec![
        vec!["set", "-g", "status-position", "top"],
        vec!["set", "-g", "mouse", "on"],
        vec!["set", "-g", "status-interval", "5"],
        vec!["set", "-g", "monitor-activity", "on"],
        vec!["set", "-g", "status", "2"],
        // ---- terminal parity with a plain kitty window ----
        // the server starts with `-f /dev/null`, so tmux's own defaults
        // apply and NOTHING here comes from the user's ~/.tmux.conf.
        // without these, panes get TERM=screen-256color: no truecolor,
        // no OSC 8 hyperlinks, no styled underlines.
        vec!["set", "-g", "default-terminal", "tmux-256color"],
        // Shift+Enter = newline in claude/codex needs the kitty keyboard
        // protocol to cross tmux; off (the -f /dev/null default) it
        // collapses to a plain CR and SUBMITS instead. `on` was tried
        // first and was not enough: it forwards extended keys only to a
        // pane whose app asked for them, tracked per pane — a tab that
        // started before the setting landed kept getting plain CR.
        // `always` forwards them to every pane; terminal (zsh) tabs see
        // a raw ^[[13;2u on Shift+Enter, which is harmless.
        vec!["set", "-s", "extended-keys", "always"],
        vec!["set", "-s", "extended-keys-format", "csi-u"],
        vec!["set", "-ga", "terminal-features", ",xterm-kitty:extkeys"],
        vec!["set", "-ga", "terminal-features", ",xterm-kitty:RGB:hyperlinks:usstyle:strikethrough"],
        vec!["set", "-ga", "terminal-overrides", ",xterm-kitty:Tc"],
        // panes otherwise inherit the environment of whichever launch()
        // first started the server — which may be hours stale
        vec![
            "set", "-g", "update-environment",
            "WAYLAND_DISPLAY DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_RUNTIME_DIR SSH_AUTH_SOCK SSH_AGENT_PID",
        ],
        // ---- clipboard ----
        // `mouse on` above means tmux, not kitty, owns the drag — so
        // kitty's copy_on_select never fires and a selection would go
        // nowhere without these. set-clipboard drives OSC 52 to kitty;
        // the copy-pipe bindings below are the belt to that braces.
        vec!["set", "-g", "set-clipboard", "on"],
        vec!["set", "-g", "allow-passthrough", "on"],
        // scrollback deep enough to actually search a long conversation
        vec!["set", "-g", "history-limit", "50000"],
    ];
    for s in sets {
        tmux(&s);
    }
    tmux(&["set", "-g", "status-style", &format!("bg={bg},fg={sub}")]);
    tmux(&[
        "set",
        "-g",
        "status-left",
        &format!("#[fg={acc2},bold] 󰚩 DravenIQ #[default] "),
    ]);
    tmux(&["set", "-g", "status-left-length", "40"]);
    tmux(&["set", "-g", "status-right", &status_right]);
    tmux(&["set", "-g", "status-right-length", "60"]);
    tmux(&["set", "-g", "@status-centre", &centre]);
    tmux(&[
        "set",
        "-g",
        "status-format[0]",
        "#[align=left]#{E:status-left}#[align=centre]#{E:@status-centre}#{?@voice, 󰍬 #{@voice},}#[align=right]#{E:status-right}",
    ]);
    tmux(&[
        "set",
        "-g",
        "status-format[1]",
        "#[align=left]#{W:#[range=window|#{window_index}]#{E:window-status-format}#[norange],#[range=window|#{window_index}]#{E:window-status-current-format}#[norange]}",
    ]);
    tmux(&["set", "-g", "pane-border-style", &format!("fg={muted}")]);
    tmux(&["set", "-g", "pane-active-border-style", &format!("fg={acc}")]);
    tmux(&[
        "set",
        "-g",
        "pane-border-format",
        &format!(
            "#{{?pane_active, #[fg={acc},bold]#{{pane_index}} \
             #{{?#{{m/r:sidebar,#{{pane_start_command}}}},sessions,#{{pane_current_command}}}} ,\
             #[fg={sub}] #{{pane_index}} \
             #{{?#{{m/r:sidebar,#{{pane_start_command}}}},sessions,#{{pane_current_command}}}} }}"
        ),
    ]);
    tmux(&[
        "set-hook",
        "-g",
        "window-layout-changed",
        &format!(
            "if -F '#{{==:#{{window_panes}},1}}' 'setw pane-border-status off ; set -uw @beside' 'setw pane-border-status top' ; run-shell -b '{me_} --window-solo'"
        ),
    ]);
    tmux(&["bind-key", "|", "split-window", "-h", "-c", "#{pane_current_path}"]);
    tmux(&["bind-key", "-", "split-window", "-v", "-c", "#{pane_current_path}"]);
    // ---- copy / paste ----
    // drag-release yanks straight to the Wayland clipboard. `-and-cancel`
    // leaves copy-mode on release, which is what makes a drag feel like a
    // drag in any other terminal instead of stranding you in a mode.
    // both tables: mode-keys is emacs by default here, but a future
    // vi-mode flip would silently lose the binding otherwise.
    for table in ["copy-mode", "copy-mode-vi"] {
        tmux(&[
            "bind-key", "-T", table, "MouseDragEnd1Pane",
            "send-keys", "-X", "copy-pipe-and-cancel", "wl-copy",
        ]);
        // double/triple-click select word/line AND copy, matching kitty
        tmux(&[
            "bind-key", "-T", table, "DoubleClick1Pane",
            "send-keys", "-X", "copy-pipe-no-clear", "wl-copy",
        ]);
    }
    // middle-click and prefix-p paste from the same clipboard
    tmux(&["bind-key", "-n", "MouseDown2Pane", "run-shell", "wl-paste --no-newline | tmux load-buffer - && tmux paste-buffer"]);
    tmux(&["bind-key", "p", "run-shell", "wl-paste --no-newline | tmux load-buffer - && tmux paste-buffer"]);
    tmux(&["bind-key", "Left", "select-pane", "-L"]);
    tmux(&["bind-key", "Right", "select-pane", "-R"]);
    tmux(&["bind-key", "Up", "select-pane", "-U"]);
    tmux(&["bind-key", "Down", "select-pane", "-D"]);
    tmux(&["set", "-g", "window-status-format", &ws_format]);
    tmux(&["set", "-g", "window-status-current-format", &ws_current]);
    // sub, not muted: muted is ~1.5:1 against the bar and the rule
    // disappeared on real wallpapers — a separator you cannot see fails
    // its one job (found live)
    tmux(&["set", "-g", "window-status-separator", &format!("#[fg={sub}]│#[default]")]);
    tmux(&[
        "bind-key",
        "-n",
        "MouseUp1Status",
        "if",
        "-F",
        "#{m/r:^x[0-9]+$,#{mouse_status_range}}",
        &kill_cmd,
        &mouse_else,
    ]);
    tmux(&[
        "bind-key",
        "-n",
        "MouseUp2Status",
        "if",
        "-F",
        "#{m/r:^window\\|,#{mouse_status_range}}",
        &format!("run-shell '{me_} --close-tab #{{s/^window\\|//:mouse_status_range}}'"),
        "",
    ]);
    // the tree pane follows the selected window (join-pane moves it)
    tmux(&[
        "set-hook", "-g", "after-select-window",
        &format!("run-shell -b '{me_} --tree-follow'"),
    ]);
    tmux(&[
        "set-hook", "-g", "after-new-window",
        &format!("run-shell -b '{me_} --tree-follow'"),
    ]);
    tmux(&["bind-key", "C-Space", "display-popup", "-E", "-w", "70%", "-h", "60%", &palette_popup]);
    tmux(&["bind-key", "g", "display-popup", "-E", "-w", "70%", "-h", "60%", &palette_popup]);
}

// ---------------- tab naming / opening (python parity) ----------------

fn find_uuid(text: &str) -> Option<String> {
    // [0-9a-f]{8}-{4}-{4}-{4}-{12}, no regex crate needed
    let b = text.as_bytes();
    let is_h = |c: u8| c.is_ascii_hexdigit() && !c.is_ascii_uppercase();
    'outer: for i in 0..b.len().saturating_sub(35) {
        let seg = [8usize, 4, 4, 4, 12];
        let mut p = i;
        for (si, &n) in seg.iter().enumerate() {
            for _ in 0..n {
                if p >= b.len() || !is_h(b[p]) {
                    continue 'outer;
                }
                p += 1;
            }
            if si < 4 {
                if p >= b.len() || b[p] != b'-' {
                    continue 'outer;
                }
                p += 1;
            }
        }
        return Some(text[i..i + 36].to_string());
    }
    None
}

/// A tab you can tell apart: prefer Claude's own conversation title.
pub fn tab_name(sid: &str, cwd: &str, width: usize, agent: &str) -> String {
    let mut title = String::new();
    if agent == "codex" {
        // codex has no aiTitle: the first user line is the best name
        if let Some(p) = hyprdesk::codex_tx_for_sid(sid) {
            title = hyprdesk::codex_meta(&p).1;
        }
    } else if !sid.is_empty() {
        if let Ok(dir) = std::fs::read_dir(hyprdesk::home().join(".claude/projects")) {
            for proj in dir.flatten() {
                let p = proj.path().join(format!("{sid}.jsonl"));
                if p.is_file() {
                    title = hyprdesk::session_title(&p);
                    break;
                }
            }
        }
    }
    let name = if !title.is_empty() {
        title
    } else {
        Path::new(cwd.trim_end_matches('/'))
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let name = if name.is_empty() { "~".to_string() } else { name };
    // tmux renames on '#' and whitespace oddities; keep it plain
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").replace('#', "");
    let out: String = name.chars().take(width).collect();
    let out = if out.is_empty() { "~".to_string() } else { out };
    match agent {
        "codex" => format!("{CODEX_GLYPH} {out}"),
        "hermes" => format!("{HERMES_GLYPH} {out}"),
        _ => out,
    }
}

/// Re-derive what every tab claims to be, from what it actually holds.
/// ONE tmux round-trip in, ONE out: the list carries the current name and
/// @beside so unchanged tabs cost nothing, and every change rides a single
/// `tmux cmd ; cmd ; …` invocation (was 1 + 2·windows forks, 37 ms).
pub fn rename_open_tabs() {
    // window -> (current name, current beside, panes)
    let mut windows: std::collections::HashMap<String, (String, String, Vec<(i32, String, String)>)> =
        std::collections::HashMap::new();
    for line in tmux_out(&[
        "list-panes",
        "-s",
        "-F",
        "#{window_index}|#{window_name}|#{@beside}|#{pane_index}|#{pane_current_path}|#{pane_start_command}",
    ])
    .lines()
    {
        let mut it = line.splitn(6, '|');
        let (idx, wname, wbeside, pane, cwd, cmd) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        if cmd.contains("--sidebar") {
            continue; // the tree pane is furniture, not identity
        }
        windows
            .entry(idx.to_string())
            .or_insert_with(|| (wname.to_string(), wbeside.to_string(), Vec::new()))
            .2
            .push((pane.parse().unwrap_or(0), cwd.to_string(), cmd.to_string()));
    }
    let mut batch: Vec<String> = Vec::new();
    for (idx, (cur_name, cur_beside, mut panes)) in windows {
        if panes.is_empty() {
            continue; // pure-sessions window keeps its name
        }
        // LOWEST surviving pane index is the tab's identity (python)
        panes.sort_by_key(|p| p.0);
        let ident = &panes[0];
        if let Some(sid) = find_uuid(&ident.2) {
            let name = tab_name(&sid, &ident.1, 18, agent_of_cmd(&ident.2));
            if name != cur_name {
                batch.extend(["rename-window".into(), "-t".into(), idx.clone(), name, ";".into()]);
            }
        }
        let mut beside = String::new();
        for (_p, cwd, cmd) in &panes[1..] {
            if let Some(sid2) = find_uuid(cmd) {
                beside = tab_name(&sid2, cwd, 10, agent_of_cmd(cmd));
                break;
            }
        }
        // SESSION-QUALIFIED: set-option -t is a target-PANE (python)
        let target = format!("{TMUX_SESSION}:{idx}");
        if beside != cur_beside {
            if !beside.is_empty() {
                batch.extend(["set-option".into(), "-w".into(), "-t".into(), target, "@beside".into(), beside, ";".into()]);
            } else {
                batch.extend(["set-option".into(), "-uw".into(), "-t".into(), target, "@beside".into(), ";".into()]);
            }
        }
    }
    if batch.is_empty() {
        return;
    }
    batch.pop(); // trailing ';'
    let args: Vec<&str> = batch.iter().map(|s| s.as_str()).collect();
    tmux(&args);
}


/// (window index, session id) for every tab whose identity pane carries a
/// session uuid — the LOWEST pane index, as rename_open_tabs decides it.
pub fn window_sids() -> Vec<(String, String)> {
    let mut best: std::collections::HashMap<String, (i32, String)> = std::collections::HashMap::new();
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_index}|#{pane_start_command}"]).lines() {
        let mut it = line.splitn(3, '|');
        let (idx, pane, cmd) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        if cmd.contains("--sidebar") || cmd.contains("--plan") {
            continue;
        }
        let Some(sid) = find_uuid(cmd) else { continue };
        let p: i32 = pane.parse().unwrap_or(0);
        match best.get(idx) {
            Some((bp, _)) if *bp <= p => {}
            _ => {
                best.insert(idx.to_string(), (p, sid));
            }
        }
    }
    best.into_iter().map(|(idx, (_, sid))| (idx, sid)).collect()
}

/// Open (or re-focus) the plan viewer beside tab `idx`. `-d`: the
/// conversation keeps the keyboard — Claude may be waiting on an answer.
pub fn open_plan_pane(idx: &str, file: &Path) {
    let target = format!("{TMUX_SESSION}:{idx}");
    let want = format!("--plan {}", file.display());
    for line in tmux_out(&["list-panes", "-t", &target, "-F", "#{pane_id}|#{pane_start_command}"]).lines() {
        let (id, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains(&want) {
            tmux(&["select-window", "-t", &target]);
            tmux(&["select-pane", "-t", id]);
            return;
        }
    }
    tmux(&[
        "split-window", "-d", "-h", "-l", "45%", "-t", &target,
        &format!("{} --plan '{}'", me(), file.display()),
    ]);
}

fn with_user_path(cmd: &str) -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    // cmd is built from uuids and fixed flags — no single quotes to escape,
    // but guard anyway so a future caller can't break out of the quoting
    let inner = cmd.replace('\'', r"'\''");
    format!("{shell} -c 'source ~/.zshrc >/dev/null 2>&1; exec {inner}'")
}

/// Glyph that marks a Codex conversation in tabs and the tree.
/// (passes the fc-list gate against JetBrainsMono NF: nf-md-robot)
pub const CODEX_GLYPH: &str = "󰚩";
/// U+F0627 (nf-md-alpha_h_box…): passes `fc-list :charset=f0627` on JetBrainsMono NF.
pub const HERMES_GLYPH: &str = "󰘧";

/// The CLI line that resumes `sid` under `agent` ("claude" | "codex").
pub fn resume_cmd(agent: &str, sid: &str) -> String {
    if agent == "hermes" {
        // hermes sessions live in ~/.hermes/sessions; v1 opens a fresh REPL
        "hermes".to_string()
    } else if agent == "codex" {
        format!("codex resume {sid}")
    } else {
        format!("claude --resume {sid}")
    }
}

/// Which agent a pane's start command runs.
pub fn agent_of_cmd(cmd: &str) -> &'static str {
    if cmd.contains("hermes") {
        "hermes"
    } else if cmd.contains("codex") {
        "codex"
    } else {
        "claude"
    }
}

/// Open (or focus) a tab running this transcript's conversation.
pub fn open_session_tab(sid: &str, cwd: &str, agent: &str) {
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_start_command}"])
        .lines()
    {
        let (idx, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains(sid) {
            tmux(&["select-window", "-t", idx]);
            return;
        }
    }
    let name = tab_name(sid, cwd, 18, agent);
    tmux(&[
        "new-window",
        "-t",
        &format!("{TMUX_SESSION}:"),
        "-n",
        &name,
        "-c",
        cwd,
        &with_user_path(&resume_cmd(agent, sid)),
    ]);
}

/// Whether this CLI accepts --session-id (one `claude --help`, cached
/// per studio process — the sidebar is long-lived like the python app).
fn claude_takes_session_id() -> bool {
    use std::sync::OnceLock;
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        Command::new("claude")
            .arg("--help")
            .output()
            .map(|o| {
                let text =
                    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
                text.contains("--session-id")
            })
            .unwrap_or(false)
    })
}

fn uuid4() -> String {
    // v4 from /dev/urandom — no rand crate for one id
    let mut b = [0u8; 16];
    use std::io::Read;
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        // degrade to a pid/time mix; still unique enough for a tab id
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        b[..16].copy_from_slice(&t.to_le_bytes()[..16.min(16)]);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// A new conversation with its identity fixed UP FRONT (python).
pub fn new_session_tab(cwd: &str, agent: &str) {
    let base = Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".into());
    if agent == "hermes" {
        let name = format!("{HERMES_GLYPH} {base}");
        tmux(&["new-window", "-t", &format!("{TMUX_SESSION}:"), "-n", &name, "-c", cwd, &with_user_path("hermes")]);
        return;
    }
    if agent == "codex" {
        // codex has no --session-id: identity comes later, from the
        // rollout the process holds open (hyprdesk::codex_procs)
        let name = format!("{CODEX_GLYPH} {base}");
        tmux(&["new-window", "-t", &format!("{TMUX_SESSION}:"), "-n", &name, "-c", cwd, &with_user_path("codex")]);
        return;
    }
    let name = base;
    let cmd = if claude_takes_session_id() {
        format!("claude --session-id {}", uuid4())
    } else {
        "claude".to_string() // older CLI: degraded, not broken
    };
    tmux(&["new-window", "-t", &format!("{TMUX_SESSION}:"), "-n", &name, "-c", cwd, &with_user_path(&cmd)]);
}

/// The tab a split should land in — never window 0 (python _work_window).
fn work_window() -> Option<String> {
    let idx = tmux_out(&["display-message", "-p", "#{window_index}"]).trim().to_string();
    if !idx.is_empty() && idx != "0" {
        return Some(idx);
    }
    let prev = tmux_out(&["display-message", "-p", "-t", "!", "#{window_index}"])
        .trim()
        .to_string();
    if !prev.is_empty() && prev != "0" {
        return Some(prev);
    }
    tmux_out(&["list-windows", "-F", "#{window_index}"])
        .split_whitespace()
        .filter(|l| *l != "0")
        .last()
        .map(String::from)
}

/// Two conversations on screen at once. Returns what it did.
pub fn open_session_beside(sid: &str, cwd: &str, agent: &str) -> &'static str {
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_start_command}"])
        .lines()
    {
        let (idx, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains(sid) {
            tmux(&["select-window", "-t", idx]);
            return "focused";
        }
    }
    let Some(target) = work_window() else {
        open_session_tab(sid, cwd, agent); // nothing to sit beside yet
        return "tab";
    };
    let t = format!("{TMUX_SESSION}:{target}");
    tmux(&["split-window", "-h", "-t", &t, "-c", cwd, &with_user_path(&resume_cmd(agent, sid))]);
    let beside = tab_name(sid, cwd, 10, agent);
    tmux(&["set-option", "-w", "-t", &t, "@beside", &beside]);
    tmux(&["select-window", "-t", &target]);
    "split"
}

pub fn open_settings_tab(page: &str) {
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_start_command}"])
        .lines()
    {
        let (idx, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains("hypr-settings") {
            tmux(&["select-window", "-t", idx]);
            return;
        }
    }
    let home = hyprdesk::home().display().to_string();
    tmux(&[
        "new-window",
        "-t",
        &format!("{TMUX_SESSION}:"),
        "-n",
        "󰌘 settings",
        "-c",
        &home,
        &format!("python3 {home}/.local/bin/hypr-settings {page}"),
    ]);
}

pub fn new_term_tab(cwd: &str) {
    let base = Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".into());
    tmux(&[
        "new-window",
        "-t",
        &format!("{TMUX_SESSION}:"),
        "-n",
        &format!(" {base}"),
        "-c",
        cwd,
    ]);
}

/// What pressing q would actually destroy (python studio_stake).
pub fn studio_stake() -> (usize, usize, Vec<String>) {
    let (mut names, mut convos) = (Vec::new(), 0usize);
    for line in tmux_out(&[
        "list-panes",
        "-s",
        "-F",
        "#{window_index}|#{pane_index}|#{window_name}|#{pane_start_command}",
    ])
    .lines()
    {
        let mut it = line.splitn(4, '|');
        let (idx, pane, name, cmd) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        if cmd.contains("--sidebar") || name == "sessions" {
            continue; // the tree is the studio's own furniture
        }
        if !names.contains(&name.to_string()) {
            names.push(name.to_string());
        }
        if find_uuid(cmd).is_some() {
            convos += 1;
        }
    }
    (names.len(), convos, names)
}

/// Split the selected tab; the sidebar gets a terminal TAB instead.
fn split_here(where_: &str) {
    let idx = tmux_out(&["display-message", "-p", "#{window_index}"]).trim().to_string();
    let cwd = {
        let c = tmux_out(&["display-message", "-p", "#{pane_current_path}"]).trim().to_string();
        if c.is_empty() { hyprdesk::home().display().to_string() } else { c }
    };
    if idx == "0" {
        new_term_tab(&hyprdesk::home().display().to_string());
        return;
    }
    let dir = if where_.ends_with('h') { "-h" } else { "-v" };
    tmux(&["split-window", dir, "-c", &cwd]);
}

// ---------------- jump palette (fzf inside a display-popup) ------------

fn palette() {
    let have_fzf = Command::new("sh")
        .args(["-c", "command -v fzf"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_fzf {
        tmux(&["choose-tree", "-Z"]); // degrade, never a blank popup
        return;
    }
    let mut open_sids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in tmux_out(&["list-panes", "-s", "-F", "#{pane_start_command}"]).lines() {
        if let Some(sid) = find_uuid(line) {
            open_sids.insert(sid);
        }
    }
    // (action, display): ("tab", idx, "", "") | ("sid", sid, cwd, agent)
    let mut items: Vec<((&str, String, String, String), String)> = Vec::new();
    for line in tmux_out(&["list-windows", "-F", "#{window_index}|#{window_name}"]).lines() {
        let (idx, name) = line.split_once('|').unwrap_or(("", ""));
        if idx == "0" {
            continue; // the tree is where you already are
        }
        items.push((("tab", idx.to_string(), String::new(), String::new()), format!("tab   {idx:>2}  {name}")));
    }
    for row in hyprdesk::session_rows() {
        // PAST only: resuming a live session opens a second client (spec)
        if row.kind != "past" || row.sid.is_empty() || open_sids.contains(&row.sid) {
            continue;
        }
        let label = if row.detail.is_empty() { row.label.clone() } else { row.detail.clone() };
        let mark = if row.agent == "codex" { format!("{CODEX_GLYPH} ") } else { String::new() };
        items.push((
            ("sid", row.sid.clone(), row.cwd.clone(), row.agent.clone()),
            format!("past      {mark}{label}  {}", row.dir).trim_end().to_string(),
        ));
    }
    if items.is_empty() {
        return;
    }
    let menu: String = items
        .iter()
        .enumerate()
        .map(|(i, (_, d))| format!("{i}\t{d}\n"))
        .collect();
    use std::io::Write;
    let Ok(mut child) = Command::new("fzf")
        .args([
            "--delimiter", "\t", "--with-nth", "2..", "--no-multi", "--reverse", "--height",
            "100%", "--prompt", "jump ",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(menu.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else { return };
    let picked = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || picked.is_empty() {
        return; // Esc, or nothing matched
    }
    let Some(i) = picked.split('\t').next().and_then(|s| s.parse::<usize>().ok()) else {
        return;
    };
    let Some(((kind, first, second, agent), _)) = items.get(i) else { return };
    if *kind == "tab" {
        // session-qualified: inside a popup "current" is not the user's
        tmux(&["select-window", "-t", &format!("{TMUX_SESSION}:{first}")]);
    } else {
        open_session_tab(first, second, agent);
    }
}

/// Every conversation window carries its OWN tree pane (spawned on
/// demand). The single-moving-pane design flickered on every tab
/// switch — the pane visibly left one window and re-joined the next —
/// and a mistimed close could strand it. Instances are cheap (~4 MB)
/// and only the visible one polls (the sidebar throttles itself when
/// its window is not active).
/// Where the select/new-window hooks record the active window index so
/// N sidebar instances can learn "am I on screen?" from a stat() instead
/// of each forking tmux every 2 s (4.7 ms × instances, measured).
pub fn active_file() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
        .join("draveniq.active")
}

pub fn note_active(idx: &str) {
    let f = active_file();
    let tmp = f.with_extension("tmp");
    if std::fs::write(&tmp, idx).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

fn tree_attach() {
    let cur = tmux_out(&["display-message", "-p", "#{window_index}"]).trim().to_string();
    if cur.is_empty() {
        return;
    }
    note_active(&cur);
    let panes = tmux_out(&[
        "list-panes", "-t", &format!("{TMUX_SESSION}:{cur}"), "-F", "#{pane_start_command}",
    ]);
    if !panes.lines().any(|c| c.contains("--sidebar")) {
        tmux(&[
            "split-window", "-d", "-b", "-h", "-l", "34",
            "-t", &format!("{TMUX_SESSION}:{cur}"),
            &format!("{} --sidebar --attached", me()),
        ]);
    }
    window_solo();
    // the launch-time pure-sessions window is a spare once real tabs
    // exist — retire it (its own tree lives full-window there)
    let wins = tmux_out(&["list-windows", "-F", "#{window_index}|#{window_name}|#{window_panes}"]);
    let total = wins.lines().count();
    if total > 1 {
        for l in wins.lines() {
            let p: Vec<&str> = l.splitn(3, '|').collect();
            if p.len() == 3 && p[1] == "sessions" && p[2] == "1" && p[0] != cur {
                tmux(&["kill-window", "-t", &format!("{TMUX_SESSION}:{}", p[0])]);
            }
        }
    }
}

/// Close a tab. Each window owns its tree pane now, so the tree dies
/// with its window by design — EXCEPT the last window, which keeps its
/// tree and becomes the sessions window (an empty studio still shows
/// the tree, never a dead session).
fn close_tab(idx: &str) {
    let total = tmux_out(&["list-windows", "-F", "x"]).lines().count();
    if total > 1 {
        tmux(&["kill-window", "-t", &format!("{TMUX_SESSION}:{idx}")]);
        return;
    }
    let mut others: Vec<String> = Vec::new();
    let mut tree_here = false;
    for line in tmux_out(&[
        "list-panes", "-t", &format!("{TMUX_SESSION}:{idx}"), "-F",
        "#{pane_id}|#{pane_start_command}",
    ])
    .lines()
    {
        let (id, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains("--sidebar") {
            tree_here = true;
        } else {
            others.push(id.to_string());
        }
    }
    if tree_here {
        for id in others {
            tmux(&["kill-pane", "-t", &id]);
        }
        tmux(&["rename-window", "-t", &format!("{TMUX_SESSION}:{idx}"), "sessions"]);
    } else {
        tmux(&["kill-window", "-t", &format!("{TMUX_SESSION}:{idx}")]);
    }
}

/// A window whose panes have dwindled to just the tree: retire it if
/// it is a background window and others exist, otherwise it IS the
/// sessions view now. Called from the window-layout-changed hook, so a
/// conversation exiting can never leave a zombie tab — nor kill the
/// studio (the tree pane keeps the last window alive).
/// SWEEP, not point-check: #{window_index} inside a hook's run-shell
/// expands against the ACTIVE window, not the window whose layout
/// changed (found live — the zombie survived), so trust nothing and
/// examine every window.
fn window_solo() {
    // window -> (pane count, has sidebar pane)
    let mut wins: Vec<(String, usize, bool)> = Vec::new();
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_start_command}"]).lines() {
        let (idx, cmd) = line.split_once('|').unwrap_or(("", ""));
        match wins.iter_mut().find(|(i, ..)| i == idx) {
            Some(w) => {
                w.1 += 1;
                w.2 |= cmd.contains("--sidebar");
            }
            None => wins.push((idx.to_string(), 1, cmd.contains("--sidebar"))),
        }
    }
    let cur = tmux_out(&["display-message", "-p", "#{window_index}"]).trim().to_string();
    if !cur.is_empty() {
        note_active(&cur); // layout changes include a window dying under the cursor
    }
    let total = wins.len();
    for (idx, panes, has_tree) in wins {
        if panes != 1 || !has_tree {
            continue; // real content present
        }
        if total > 1 && idx != cur {
            tmux(&["kill-window", "-t", &format!("{TMUX_SESSION}:{idx}")]);
        } else {
            tmux(&["rename-window", "-t", &format!("{TMUX_SESSION}:{idx}"), "sessions"]);
        }
    }
}


/// Measure the studio's hot paths against the REAL machine state.
/// `draveniq --bench [n]` — mean/p95 ms per call + RSS.
/// Numbers, not guesses, pick the optimisation targets (docs/perf/).
fn bench(n: usize) {
    fn rss_kb() -> u64 {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|pages| pages * 4)
            .unwrap_or(0)
    }
    fn time<F: FnMut()>(label: &str, n: usize, mut f: F) {
        let mut ms: Vec<f64> = Vec::with_capacity(n);
        for _ in 0..n {
            let t = std::time::Instant::now();
            f();
            ms.push(t.elapsed().as_secs_f64() * 1e3);
        }
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = ms.iter().sum::<f64>() / n as f64;
        let p95 = ms[((n as f64 * 0.95) as usize).min(n - 1)];
        println!("{label:<28} mean {mean:8.2} ms   p95 {p95:8.2} ms   (n={n})");
    }
    println!("rss at start          {} kB", rss_kb());
    let procs = hyprdesk::claude_procs();
    println!("claude procs: {}  windows: {}", procs.len(), sidebar::open_windows().len());
    time("claude_procs (/proc scan)", n, || { hyprdesk::claude_procs(); });
    time("recent_transcripts(25)", n, || { hyprdesk::recent_transcripts(25); });
    time("session_rows (reload)", n, || { hyprdesk::session_rows(); });
    time("daemon_hosted (/proc)", n, || { hyprdesk::daemon_hosted(); });
    time("codex_procs (/proc)", n, || { hyprdesk::codex_procs(); });
    time("recent_codex_transcripts", n, || { hyprdesk::recent_codex_transcripts(25); });
    {
        let txs = hyprdesk::recent_transcripts(25);
        time("session_meta x25", n, || { for (_, p) in &txs { hyprdesk::session_meta(p); } });
        time("session_title x25", n, || { for (_, p) in &txs { hyprdesk::session_title(p); } });
    }
    time("open_windows (tmux)", n, || { sidebar::open_windows(); });
    time("window_active (tmux)", n, || {
        tmux_out(&["display-message", "-p", "-t", "0", "#{window_active}"]);
    });
    if let Some(p) = procs.first() {
        let pid = p.pid;
        time("window_of_pid (hyprctl)", n, || { hyprdesk::window_of_pid(pid); });
    }
    time("rename_open_tabs", (n / 4).max(1), rename_open_tabs);
    let txs = hyprdesk::recent_transcripts(25);
    if let Some((_, p)) = txs.first() {
        time("session_title (1 tail)", n, || { hyprdesk::session_title(p); });
    }
    println!("rss at end            {} kB", rss_kb());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--new-tab") {
        let agent = args.iter().position(|a| a == "--agent").and_then(|i| args.get(i + 1)).map(|s| s.as_str()).unwrap_or("claude");
        let cwd = args.iter().position(|a| a == "--cwd").and_then(|i| args.get(i + 1)).cloned()
            .unwrap_or_else(|| std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
        new_session_tab(&cwd, agent);
        return;
    }
    if args.iter().any(|a| a == "--open-plan") {
        // the sidebar records each tab's plan in the @plan window option
        let line = tmux_out(&["display-message", "-p", "-t", &format!("{TMUX_SESSION}:"), "#{window_index}|#{@plan}"]);
        let (idx, plan) = line.trim().split_once('|').unwrap_or(("", ""));
        if idx.is_empty() || plan.is_empty() {
            println!("no plan seen for the active tab yet");
        } else {
            open_plan_pane(idx, Path::new(plan));
            println!("plan: {}", Path::new(plan).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--plan-dump") {
        let file = args.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let width = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(80);
        plan::dump(Path::new(file), width);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--plan") {
        if let Some(file) = args.get(i + 1) {
            plan::run(Path::new(file));
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--bench") {
        bench(args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(20));
        return;
    }
    if args.iter().any(|a| a == "--window-solo") {
        window_solo();
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--close-tab") {
        if let Some(idx) = args.get(i + 1) {
            close_tab(idx);
        }
        return;
    }
    if args.iter().any(|a| a == "--tree-follow") || args.iter().any(|a| a == "--tree-attach") {
        tree_attach(); // --tree-follow kept for stale bindings
        return;
    }
    if args.iter().any(|a| a == "--sidebar") {
        sidebar::run();
    } else if let Some(i) = args.iter().position(|a| a == "--split") {
        split_here(args.get(i + 1).map(|s| s.as_str()).unwrap_or("sh"));
    } else if args.iter().any(|a| a == "--palette") {
        palette();
    } else if args.iter().any(|a| a == "--style") {
        // re-dress a LIVE studio's bar and bindings (wallpaper recolor,
        // and the swap moment: the old bindings name the python cmdline)
        style_tmux();
        rename_open_tabs();
    } else {
        launch();
    }
}
