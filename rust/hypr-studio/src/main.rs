//! hypr-claude-studio — a VS-Code-style workspace for Claude Code
//! sessions. bin/hypr-claude-studio (python/Textual) is the spec;
//! docs/claude-studio.md is the contract. Rung 4 — the LAST rung — of
//! the Rust migration: the tmux orchestration ports as subprocess work,
//! the Textual sidebar is hand-rolled in ratatui (sidebar.rs).
//!
//! Modes: (default) focus-or-spawn · --sidebar (inside tmux) ·
//! --split h|v · --palette (inside a display-popup). Every mode needs a
//! branch in main(): an unknown flag falls through to launch(), which
//! spawns a SECOND studio rather than failing quietly.

mod sidebar;

use std::path::Path;
use std::process::Command;

pub const TMUX_SESSION: &str = "claude-studio";
const KITTY_CLASS: &str = "hyprclaudestudio";

pub fn me() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "hypr-claude-studio".into())
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
    let _ = Command::new("setsid")
        .args([
            "kitty",
            "--class",
            KITTY_CLASS,
            "--title",
            "Claude Studio",
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

// See bin/hypr-claude-studio for the full commentary on every choice
// here — the ranges, the missing scrollport, the run-shell ✕ path. The
// format strings are copied verbatim; only the interpolation moved.
const CLOSE_X: &str = "#{?#{!=:#{window_index},0},#[range=user|x#{window_index}] ✕ #[norange],}";

fn pane_n(ink: &str, restore: &str) -> String {
    format!(
        "#{{?#{{@beside}},{ink}+#{{=/10/…:@beside}} {restore},\
         #{{?#{{>:#{{window_panes}},1}},{ink}·#{{window_panes}} {restore},}}}}"
    )
}

fn attn(good: &str, sub: &str, restore: &str) -> String {
    format!(
        "#{{?#{{==:#{{window_index}},0}},,\
         #{{?window_bell_flag,{good}● {restore},\
         #{{?window_activity_flag,{sub}○ {restore},}}}}}}"
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
    let kill_cmd = format!(
        "run-shell 'tmux -L {TMUX_SESSION} kill-window -t #{{s/^x//:mouse_status_range}}'"
    );
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
    ];
    for s in sets {
        tmux(&s);
    }
    tmux(&["set", "-g", "status-style", &format!("bg={bg},fg={sub}")]);
    tmux(&[
        "set",
        "-g",
        "status-left",
        &format!("#[fg={acc2},bold] 󰚩 Claude Studio #[default] "),
    ]);
    tmux(&["set", "-g", "status-left-length", "40"]);
    tmux(&["set", "-g", "status-right", &status_right]);
    tmux(&["set", "-g", "status-right-length", "60"]);
    tmux(&["set", "-g", "@status-centre", &centre]);
    tmux(&[
        "set",
        "-g",
        "status-format[0]",
        "#[align=left]#{E:status-left}#[align=centre]#{E:@status-centre}#[align=right]#{E:status-right}",
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
            "#{{?pane_active, #[fg={acc},bold]#{{pane_index}} #{{pane_current_command}} ,\
             #[fg={sub}] #{{pane_index}} #{{pane_current_command}} }}"
        ),
    ]);
    tmux(&[
        "set-hook",
        "-g",
        "window-layout-changed",
        "if -F '#{==:#{window_panes},1}' 'setw pane-border-status off ; set -uw @beside' 'setw pane-border-status top'",
    ]);
    tmux(&["bind-key", "|", "split-window", "-h", "-c", "#{pane_current_path}"]);
    tmux(&["bind-key", "-", "split-window", "-v", "-c", "#{pane_current_path}"]);
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
    tmux(&["bind-key", "-n", "MouseUp2Status", "kill-window", "-t="]);
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
pub fn tab_name(sid: &str, cwd: &str, width: usize) -> String {
    let mut title = String::new();
    if !sid.is_empty() {
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
    if out.is_empty() { "~".into() } else { out }
}

/// Re-derive what every tab claims to be, from what it actually holds.
pub fn rename_open_tabs() {
    let mut windows: std::collections::HashMap<String, Vec<(i32, String, String)>> =
        std::collections::HashMap::new();
    for line in tmux_out(&[
        "list-panes",
        "-s",
        "-F",
        "#{window_index}|#{pane_index}|#{pane_current_path}|#{pane_start_command}",
    ])
    .lines()
    {
        let mut it = line.splitn(4, '|');
        let (idx, pane, cwd, cmd) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        if idx == "0" {
            continue; // the sidebar owns its window; never renamed
        }
        windows.entry(idx.to_string()).or_default().push((
            pane.parse().unwrap_or(0),
            cwd.to_string(),
            cmd.to_string(),
        ));
    }
    for (idx, mut panes) in windows {
        // LOWEST surviving pane index is the tab's identity (python)
        panes.sort_by_key(|p| p.0);
        let ident = &panes[0];
        if let Some(sid) = find_uuid(&ident.2) {
            let name = tab_name(&sid, &ident.1, 18);
            tmux(&["rename-window", "-t", &idx, &name]);
        }
        let mut beside = String::new();
        for (_p, cwd, cmd) in &panes[1..] {
            if let Some(sid2) = find_uuid(cmd) {
                beside = tab_name(&sid2, cwd, 10);
                break;
            }
        }
        // SESSION-QUALIFIED: set-option -t is a target-PANE (python)
        let target = format!("{TMUX_SESSION}:{idx}");
        if !beside.is_empty() {
            tmux(&["set-option", "-w", "-t", &target, "@beside", &beside]);
        } else {
            tmux(&["set-option", "-uw", "-t", &target, "@beside"]);
        }
    }
}

/// Open (or focus) a tab running this transcript's conversation.
pub fn open_session_tab(sid: &str, cwd: &str) {
    for line in tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_start_command}"])
        .lines()
    {
        let (idx, cmd) = line.split_once('|').unwrap_or(("", ""));
        if cmd.contains(sid) {
            tmux(&["select-window", "-t", idx]);
            return;
        }
    }
    let name = tab_name(sid, cwd, 18);
    tmux(&[
        "new-window",
        "-t",
        &format!("{TMUX_SESSION}:"),
        "-n",
        &name,
        "-c",
        cwd,
        &format!("claude --resume {sid}"),
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
pub fn new_session_tab(cwd: &str) {
    let name = Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".into());
    let cmd = if claude_takes_session_id() {
        format!("claude --session-id {}", uuid4())
    } else {
        "claude".to_string() // older CLI: degraded, not broken
    };
    tmux(&["new-window", "-t", &format!("{TMUX_SESSION}:"), "-n", &name, "-c", cwd, &cmd]);
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
pub fn open_session_beside(sid: &str, cwd: &str) -> &'static str {
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
        open_session_tab(sid, cwd); // nothing to sit beside yet
        return "tab";
    };
    let t = format!("{TMUX_SESSION}:{target}");
    tmux(&["split-window", "-h", "-t", &t, "-c", cwd, &format!("claude --resume {sid}")]);
    let beside = tab_name(sid, cwd, 10);
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
        if idx == "0" {
            continue;
        }
        if pane == "0" {
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
    // (action, display): ("tab", idx, "") | ("sid", sid, cwd)
    let mut items: Vec<((&str, String, String), String)> = Vec::new();
    for line in tmux_out(&["list-windows", "-F", "#{window_index}|#{window_name}"]).lines() {
        let (idx, name) = line.split_once('|').unwrap_or(("", ""));
        if idx == "0" {
            continue; // the tree is where you already are
        }
        items.push((("tab", idx.to_string(), String::new()), format!("tab   {idx:>2}  {name}")));
    }
    for row in hyprdesk::session_rows() {
        // PAST only: resuming a live session opens a second client (spec)
        if row.kind != "past" || row.sid.is_empty() || open_sids.contains(&row.sid) {
            continue;
        }
        let label = if row.detail.is_empty() { row.label.clone() } else { row.detail.clone() };
        items.push((
            ("sid", row.sid.clone(), row.cwd.clone()),
            format!("past      {label}  {}", row.dir).trim_end().to_string(),
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
    let Some(((kind, first, second), _)) = items.get(i) else { return };
    if *kind == "tab" {
        // session-qualified: inside a popup "current" is not the user's
        tmux(&["select-window", "-t", &format!("{TMUX_SESSION}:{first}")]);
    } else {
        open_session_tab(first, second);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
