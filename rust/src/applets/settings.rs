//! Hypr Settings — a click-and-pick control panel for Hyprland (ratatui).
//!
//! Port of the python/textual app: sidebar navigation + titled cards,
//! colored with the live wallust palette, terminal-default background so
//! kitty's transparency shows through (glass). Appearance changes apply
//! LIVE via hyprctl keywords (native IPC here); Save writes them into
//! ~/.config/hypr/hyprland.conf so they survive reboot.

use crate::applets::{av_status, fw_status};
use crate::{colors, hypr, proc, util};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const GAUGE_SLOTS: usize = 12;

// key -> (hyprctl option, config path, min, max, label)
pub const INTS: [(&str, &str, [&str; 2], i64, i64, &str); 4] = [
    ("gaps_in", "general:gaps_in", ["general", "gaps_in"], 0, 40, "Inner gaps (between windows)"),
    ("gaps_out", "general:gaps_out", ["general", "gaps_out"], 0, 60, "Outer gaps (screen edges)"),
    ("border_size", "general:border_size", ["general", "border_size"], 0, 10, "Border thickness"),
    ("rounding", "decoration:rounding", ["decoration", "rounding"], 0, 20, "Corner rounding"),
];

// key -> (hyprctl option, config path, label)
pub const BOOLS: [(&str, &str, &[&str], &str); 3] = [
    (
        "blur",
        "decoration:blur:enabled",
        &["decoration", "blur", "enabled"],
        "Background blur (glass effect)",
    ),
    (
        "shadow",
        "decoration:shadow:enabled",
        &["decoration", "shadow", "enabled"],
        "Window shadows",
    ),
    ("animations", "animations:enabled", &["animations", "enabled"], "Animations"),
];

pub const NAV: [(&str, &str, &str); 5] = [
    ("appearance", "󰉼", "Appearance"),
    ("bar", "󰜬", "Bar"),
    ("wallpaper", "󰸉", "Wallpaper"),
    ("security", "󰕥", "Security"),
    ("power", "⏻", "Power"),
];

fn page_file() -> PathBuf {
    util::xdg_runtime().join("hypr-settings.page")
}

// ---------- pure logic (unit-tested) ----------

/// Rewrite `key = value` lines in hyprland.conf, matched by block path.
/// Returns (new_text, set of paths actually written) — the python
/// save_to_config, line for line.
pub fn save_to_config(
    text: &str,
    values: &HashMap<Vec<String>, String>,
) -> (String, BTreeSet<Vec<String>>) {
    let mut stack: Vec<String> = Vec::new();
    let mut out = String::new();
    let mut written = BTreeSet::new();
    for raw in text.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let nl = if raw.ends_with('\n') { "\n" } else { "" };
        let stripped = line.trim();
        if is_block_open(stripped) {
            stack.push(stripped.split('{').next().unwrap_or("").trim().to_string());
            out.push_str(raw);
        } else if stripped.starts_with('}') {
            stack.pop();
            out.push_str(raw);
        } else if let Some((indent, key, _val, comment)) = parse_kv(line) {
            let mut path: Vec<String> = stack.clone();
            path.push(key.to_string());
            if let Some(newval) = values.get(&path) {
                out.push_str(&format!("{indent}{key} = {newval}{comment}{nl}"));
                written.insert(path);
            } else {
                out.push_str(raw);
            }
        } else {
            out.push_str(raw);
        }
    }
    (out, written)
}

/// ^[\w.-]+\s*\{
fn is_block_open(stripped: &str) -> bool {
    let Some(brace) = stripped.find('{') else { return false };
    let name = stripped[..brace].trim_end();
    !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// ^(\s*)([\w.]+)\s*=\s*(.*?)(\s*#.*)?$  → (indent, key, value, comment)
fn parse_kv(line: &str) -> Option<(&str, &str, &str, &str)> {
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];
    let eq = trimmed_start.find('=')?;
    let key = trimmed_start[..eq].trim_end();
    if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return None;
    }
    let rest = &trimmed_start[eq + 1..];
    let (value_part, comment) = match rest.find('#') {
        Some(h) => rest.split_at(h),
        None => (rest, ""),
    };
    let _ = value_part;
    // keep the comment with its leading whitespace, like the python regex
    let comment_full = if comment.is_empty() {
        ""
    } else {
        let vp_trimmed = value_part.trim_end();
        let ws_start = eq + 1 + vp_trimmed.len();
        &trimmed_start[ws_start..]
    };
    Some((indent, key, value_part.trim(), comment_full))
}

pub fn gauge_filled(val: i64, lo: i64, hi: i64) -> usize {
    if hi <= lo {
        return 0;
    }
    let frac = (val - lo) as f64 / (hi - lo) as f64;
    ((frac * GAUGE_SLOTS as f64).round() as i64).clamp(0, GAUGE_SLOTS as i64) as usize
}

// ---------- app state ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wid {
    Nav(usize),
    Dec(usize),
    Inc(usize),
    Switch(usize),   // BOOLS index
    AutohideSwitch,
    Save,
    Revert,
    BarToggle,
    BarRestart,
    BarReorder,
    WpRandom,
    WpPick,
    FwToggle,
    FwRules,
    AvScan,
    AvGui,
    AvUpdate,
    PLock,
    PSuspend,
    PLogout,
    PReboot,
    PShutdown,
    ModalNo,
    ModalYes,
}

pub struct Confirm {
    question: String,
    cmd: Vec<String>,
    yes_focused: bool,
}

enum TimerAction {
    RefreshSecurity,
    RefreshWallpapers,
}

pub struct App {
    pal: colors::Palette,
    pub page: usize,
    pub ints: HashMap<&'static str, i64>,
    pub limits: HashMap<&'static str, i64>,
    pub multi: HashMap<&'static str, String>,
    pub bools: HashMap<&'static str, bool>,
    pub dirty: BTreeSet<&'static str>,
    pub autohide: bool,
    fw_on: bool,
    av_on: bool,
    wallpapers: Vec<(String, String)>,
    pub focus: usize,
    modal: Option<Confirm>,
    toast: Option<(String, Style, Instant, Duration)>,
    timers: Vec<(Instant, TimerAction)>,
    fw_rx: Option<mpsc::Receiver<String>>,
    hits: Vec<(Rect, Wid)>,
    poll_tick: u32,
    pub should_quit: bool,
    pub last_save_missing: Vec<String>,
}

fn autohide_on() -> bool {
    if util::dry() {
        return false;
    }
    !proc::pids_with_cmdline("waybar-autohide.sh").is_empty()
}

fn current_wallpapers() -> Vec<(String, String)> {
    let path = util::home().join(".config/hypr/hyprpaper.conf");
    let mut pairs = Vec::new();
    let Ok(text) = std::fs::read_to_string(path) else { return pairs };
    let mut mon: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("monitor") {
            let val = line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
            mon = Some(if val.is_empty() { "(all)".to_string() } else { val.to_string() });
        } else if line.starts_with("path") {
            if let Some(m) = mon.take() {
                let p = line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
                let name = p.rsplit('/').next().unwrap_or(p).to_string();
                pairs.push((m, name));
            }
        }
    }
    pairs
}

impl App {
    pub fn new(initial_page: Option<&str>) -> Self {
        let defaults: HashMap<&str, i64> =
            [("gaps_in", 5), ("gaps_out", 10), ("border_size", 3), ("rounding", 8)]
                .into_iter()
                .collect();
        let mut ints = HashMap::new();
        let mut limits = HashMap::new();
        let mut multi = HashMap::new();
        for (key, opt, _, _, hi, _) in INTS {
            let v = hypr::getoption_int(opt, defaults[key]);
            ints.insert(key, v);
            // if the live value exceeds the panel's range, stretch the range
            limits.insert(key, hi.max(v));
        }
        // per-side gaps ("5 10 5 10") are shown read-only, never saved
        for key in ["gaps_in", "gaps_out"] {
            let opt = INTS.iter().find(|i| i.0 == key).unwrap().1;
            if let Some(raw) = hypr::getoption_custom(opt) {
                let toks: BTreeSet<&str> = raw.split_whitespace().collect();
                if toks.len() > 1 {
                    multi.insert(
                        INTS.iter().find(|i| i.0 == key).unwrap().0,
                        raw.clone(),
                    );
                }
            }
        }
        let mut bools = HashMap::new();
        for (key, opt, _, _) in BOOLS {
            bools.insert(key, hypr::getoption_int(opt, 1) != 0);
        }
        let mut app = App {
            pal: colors::read_palette(),
            page: 0,
            ints,
            limits,
            multi,
            bools,
            dirty: BTreeSet::new(),
            autohide: autohide_on(),
            fw_on: false,
            av_on: false,
            wallpapers: Vec::new(),
            focus: 0,
            modal: None,
            toast: None,
            timers: Vec::new(),
            fw_rx: None,
            hits: Vec::new(),
            poll_tick: 0,
            should_quit: false,
            last_save_missing: Vec::new(),
        };
        // a leftover page-note from a previous run must not hijack us
        match initial_page {
            Some(p) => app.goto_page(p),
            None => {
                let _ = std::fs::remove_file(page_file());
            }
        }
        app.refresh_security();
        app.refresh_wallpapers();
        app
    }

    pub fn goto_page(&mut self, page: &str) {
        if let Some(i) = NAV.iter().position(|(name, _, _)| *name == page) {
            self.page = i;
            self.focus = 0;
            self.on_page_change();
        }
    }

    fn on_page_change(&mut self) {
        self.refresh_security();
        self.refresh_wallpapers();
        self.autohide = autohide_on();
    }

    fn refresh_security(&mut self) {
        self.fw_on = fw_status::ufw_enabled_in_conf();
        self.av_on = if util::dry() { false } else { av_status::service_active("clamav-daemon") };
    }

    fn refresh_wallpapers(&mut self) {
        self.wallpapers = current_wallpapers();
    }

    pub fn notify(&mut self, msg: &str) {
        self.notify_style(msg, Style::default(), Duration::from_secs(5));
    }

    fn notify_style(&mut self, msg: &str, style: Style, timeout: Duration) {
        self.toast = Some((msg.to_string(), style, Instant::now(), timeout));
    }

    fn notify_warning(&mut self, msg: &str, secs: u64) {
        let c = color(&self.pal.yellow);
        self.notify_style(msg, Style::default().fg(c), Duration::from_secs(secs));
    }

    fn notify_error(&mut self, msg: &str) {
        let c = color(&self.pal.danger);
        self.notify_style(msg, Style::default().fg(c), Duration::from_secs(5));
    }

    /// Interactive widgets on the current page, in tab order.
    pub fn focusables(&self) -> Vec<Wid> {
        let mut v = Vec::new();
        match self.page {
            0 => {
                for (i, (key, ..)) in INTS.iter().enumerate() {
                    if !self.multi.contains_key(key) {
                        v.push(Wid::Dec(i));
                        v.push(Wid::Inc(i));
                    }
                }
                for i in 0..BOOLS.len() {
                    v.push(Wid::Switch(i));
                }
                v.push(Wid::Save);
                v.push(Wid::Revert);
            }
            1 => {
                v.push(Wid::AutohideSwitch);
                v.push(Wid::BarToggle);
                v.push(Wid::BarRestart);
                v.push(Wid::BarReorder);
            }
            2 => {
                v.push(Wid::WpRandom);
                v.push(Wid::WpPick);
            }
            3 => {
                v.push(Wid::FwToggle);
                v.push(Wid::FwRules);
                v.push(Wid::AvScan);
                v.push(Wid::AvGui);
                v.push(Wid::AvUpdate);
            }
            _ => {
                v.push(Wid::PLock);
                v.push(Wid::PSuspend);
                v.push(Wid::PLogout);
                v.push(Wid::PReboot);
                v.push(Wid::PShutdown);
            }
        }
        v
    }

    // ---------- actions ----------

    pub fn step(&mut self, idx: usize, delta: i64) {
        let (key, opt, _, lo, _, _) = INTS[idx];
        if self.multi.contains_key(key) {
            return;
        }
        let hi = self.limits[key];
        let v = (self.ints[key] + delta).clamp(lo, hi);
        self.ints.insert(key, v);
        self.dirty.insert(key);
        hypr::keyword(opt, &v.to_string());
    }

    pub fn toggle_bool(&mut self, idx: usize) {
        let (key, opt, _, _) = BOOLS[idx];
        let v = !self.bools[key];
        self.bools.insert(key, v);
        self.dirty.insert(key);
        hypr::keyword(opt, if v { "1" } else { "0" });
    }

    fn toggle_autohide_switch(&mut self) {
        let want = !self.autohide;
        if want && !autohide_on() {
            util::spawn_detached(&[&util::local_bin("waybar-autohide.sh").to_string_lossy()]);
        } else if !want && autohide_on() {
            proc::pkill_cmdline("waybar-autohide.sh", libc::SIGTERM);
        }
        self.autohide = want;
        self.notify(&format!(
            "Auto-hide {}",
            if want { "ON — move mouse to top edge to reveal" } else { "OFF — bar always visible" }
        ));
    }

    pub fn do_save(&mut self) {
        let mut values: HashMap<Vec<String>, String> = HashMap::new();
        let mut names: HashMap<Vec<String>, String> = HashMap::new();
        for key in self.dirty.clone() {
            if let Some((_, _, path, _, _, label)) = INTS.iter().find(|i| i.0 == key) {
                if self.multi.contains_key(key) {
                    continue;
                }
                let p: Vec<String> = path.iter().map(|s| s.to_string()).collect();
                values.insert(p.clone(), self.ints[key].to_string());
                names.insert(p, label.to_string());
            } else if let Some((_, _, path, label)) = BOOLS.iter().find(|b| b.0 == key) {
                let p: Vec<String> = path.iter().map(|s| s.to_string()).collect();
                values.insert(p.clone(), if self.bools[key] { "true" } else { "false" }.into());
                names.insert(p, label.to_string());
            }
        }
        self.last_save_missing.clear();
        if values.is_empty() {
            self.notify("Nothing changed since opening — nothing to save");
            return;
        }
        let conf = util::home().join(".config/hypr/hyprland.conf");
        let text = match std::fs::read_to_string(&conf) {
            Ok(t) => t,
            Err(e) => {
                self.notify_error(&format!("Save failed: {e}"));
                return;
            }
        };
        let (new_text, written) = save_to_config(&text, &values);
        if let Err(e) = std::fs::write(&conf, new_text) {
            self.notify_error(&format!("Save failed: {e}"));
            return;
        }
        let missing: Vec<Vec<String>> =
            values.keys().filter(|p| !written.contains(*p)).cloned().collect();
        if !missing.is_empty() {
            let miss: Vec<String> = missing.iter().map(|p| names[p].clone()).collect();
            self.last_save_missing = miss.clone();
            self.notify_warning(
                &format!("⚠ NOT saved: {} — its line is missing from hyprland.conf", miss.join(", ")),
                10,
            );
        }
        if !written.is_empty() {
            self.dirty.retain(|k| {
                let path: Vec<String> = if let Some(i) = INTS.iter().find(|i| i.0 == *k) {
                    i.2.iter().map(|s| s.to_string()).collect()
                } else if let Some(b) = BOOLS.iter().find(|b| b.0 == *k) {
                    b.2.iter().map(|s| s.to_string()).collect()
                } else {
                    return true;
                };
                !written.contains(&path)
            });
            self.notify(&format!("Saved {} setting(s) — they survive reboot", written.len()));
        }
    }

    pub fn do_revert(&mut self) {
        if !util::dry() {
            hypr::reload();
        }
        self.dirty.clear();
        for (key, opt, _, _, hi, _) in INTS {
            if self.multi.contains_key(key) {
                continue;
            }
            let v = hypr::getoption_int(opt, self.ints[key]);
            self.ints.insert(key, v);
            self.limits.insert(key, hi.max(v));
        }
        for (key, opt, _, _) in BOOLS {
            let cur = self.bools[key];
            self.bools.insert(key, hypr::getoption_int(opt, cur as i64) != 0);
        }
        self.notify("Reverted to the saved config");
    }

    fn do_fw_toggle(&mut self) {
        if !which("ufw") {
            self.notify_error("ufw is not installed — run: sudo apt install ufw");
            return;
        }
        let verb = if self.fw_on { "disable" } else { "enable" };
        if util::dry() {
            util::spawn_detached(&["pkexec", "ufw", verb]);
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.fw_rx = Some(rx);
        let verb = verb.to_string();
        std::thread::spawn(move || {
            let out = std::process::Command::new("pkexec").args(["ufw", &verb]).output();
            let msg = match out {
                Ok(o) if o.status.code() == Some(126) || o.status.code() == Some(127) => {
                    "warn:Cancelled — no password given".to_string()
                }
                Ok(o) if !o.status.success() => {
                    let err = String::from_utf8_lossy(if o.stderr.is_empty() {
                        &o.stdout
                    } else {
                        &o.stderr
                    })
                    .trim()
                    .chars()
                    .take(120)
                    .collect::<String>();
                    let err = if err.is_empty() { "unknown error".into() } else { err };
                    format!("err:ufw {verb} failed: {err}")
                }
                Ok(_) => "ok:".to_string(),
                Err(e) => format!("err:ufw {verb} failed: {e}"),
            };
            let _ = tx.send(msg);
        });
        self.timers.push((Instant::now() + Duration::from_secs(10), TimerAction::RefreshSecurity));
    }

    fn do_open_clamtk(&mut self) {
        if which("clamtk") {
            util::spawn_detached(&["clamtk"]);
        } else {
            self.notify_error(
                "ClamTk is not installed — use the Tools menu to install it, or: sudo apt install clamtk",
            );
        }
    }

    pub fn activate(&mut self, wid: Wid) {
        let tools = util::local_bin("hypr-tools.sh").to_string_lossy().into_owned();
        match wid {
            Wid::Nav(i) => {
                self.page = i;
                self.focus = 0;
                self.on_page_change();
            }
            Wid::Dec(i) => self.step(i, -1),
            Wid::Inc(i) => self.step(i, 1),
            Wid::Switch(i) => self.toggle_bool(i),
            Wid::AutohideSwitch => self.toggle_autohide_switch(),
            Wid::Save => self.do_save(),
            Wid::Revert => self.do_revert(),
            Wid::BarToggle => {
                util::spawn_detached(&[&util::local_bin("bar-toggle.sh").to_string_lossy()])
            }
            Wid::BarRestart => util::spawn_detached(&[&tools, "restart-bar"]),
            Wid::BarReorder => util::spawn_detached(&[&tools, "reorder"]),
            Wid::WpRandom => {
                util::spawn_detached(&[&util::local_bin("wallpaper.sh").to_string_lossy()]);
                self.timers
                    .push((Instant::now() + Duration::from_secs(4), TimerAction::RefreshWallpapers));
            }
            Wid::WpPick => {
                util::spawn_detached(&[&tools, "wallpaper"]);
                self.timers
                    .push((Instant::now() + Duration::from_secs(4), TimerAction::RefreshWallpapers));
            }
            Wid::FwToggle => self.do_fw_toggle(),
            Wid::FwRules => util::spawn_detached(&[
                "kitty", "--class", "floatterm", "--title", "Firewall rules", "bash", "-c",
                "pkexec ufw status verbose; echo; read -rp 'Enter to close…'",
            ]),
            Wid::AvScan => util::spawn_detached(&[&tools, "clamav"]),
            Wid::AvGui => self.do_open_clamtk(),
            Wid::AvUpdate => util::spawn_detached(&[
                "kitty", "--class", "floatterm", "--title", "Update virus definitions", "bash",
                "-c",
                "pkexec bash -c 'systemctl stop clamav-freshclam; freshclam; systemctl start clamav-freshclam'; echo; read -rp 'Enter to close…'",
            ]),
            Wid::PLock => util::spawn_detached(&["hyprlock"]),
            Wid::PSuspend => util::spawn_detached(&["systemctl", "suspend"]),
            Wid::PLogout => self.confirm("Really log out?", &["hyprctl", "dispatch", "exit"]),
            Wid::PReboot => self.confirm("Really reboot?", &["systemctl", "reboot"]),
            Wid::PShutdown => self.confirm("Really shut down?", &["systemctl", "poweroff"]),
            Wid::ModalNo | Wid::ModalYes => {
                let yes = wid == Wid::ModalYes;
                if let Some(m) = self.modal.take() {
                    if yes {
                        let cmd: Vec<&str> = m.cmd.iter().map(String::as_str).collect();
                        util::spawn_detached(&cmd);
                    }
                }
            }
        }
    }

    pub fn confirm(&mut self, question: &str, cmd: &[&str]) {
        self.modal = Some(Confirm {
            question: question.to_string(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            yes_focused: false,
        });
    }

    // ---------- events ----------

    pub fn on_key(&mut self, ev: KeyEvent) {
        if let Some(m) = &mut self.modal {
            match ev.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.modal = None;
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    m.yes_focused = !m.yes_focused;
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let wid = if m.yes_focused { Wid::ModalYes } else { Wid::ModalNo };
                    self.activate(wid);
                }
                _ => {}
            }
            return;
        }
        match ev.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(c @ '1'..='5') => {
                let i = (c as u8 - b'1') as usize;
                self.activate(Wid::Nav(i));
            }
            KeyCode::Down => self.move_focus(1),
            KeyCode::Up => self.move_focus(-1),
            KeyCode::Tab => {
                if ev.modifiers.contains(KeyModifiers::SHIFT) {
                    self.move_focus(-1)
                } else {
                    self.move_focus(1)
                }
            }
            KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Left | KeyCode::Right => {
                // on a stepper's -/+ pair, left/right steps the value
                let delta: i32 = if ev.code == KeyCode::Right { 1 } else { -1 };
                match self.focusables().get(self.focus) {
                    Some(Wid::Dec(i)) | Some(Wid::Inc(i)) => self.step(*i, delta as i64),
                    _ => self.move_focus(delta),
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(w) = self.focusables().get(self.focus).copied() {
                    self.activate(w);
                }
            }
            _ => {}
        }
    }

    fn move_focus(&mut self, delta: i32) {
        let n = self.focusables().len() as i32;
        if n == 0 {
            return;
        }
        self.focus = ((self.focus as i32 + delta).rem_euclid(n)) as usize;
    }

    pub fn on_mouse(&mut self, ev: MouseEvent) {
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let pos = Position { x: ev.column, y: ev.row };
        let hit = self.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, w)| *w);
        if let Some(w) = hit {
            if self.modal.is_some() && !matches!(w, Wid::ModalNo | Wid::ModalYes) {
                return; // modal is… modal
            }
            // clicking focuses too
            if let Some(i) = self.focusables().iter().position(|f| *f == w) {
                self.focus = i;
            }
            self.activate(w);
        }
    }

    /// 0.5 s cadence: page-file navigation + periodic autohide re-check.
    pub fn tick(&mut self) {
        if let Ok(page) = std::fs::read_to_string(page_file()) {
            let _ = std::fs::remove_file(page_file());
            self.goto_page(page.trim());
        }
        self.poll_tick += 1;
        // re-check auto-hide state every 2s while the Bar page shows
        if self.poll_tick.is_multiple_of(4) && self.page == 1 {
            self.autohide = autohide_on();
        }
        // timers
        let now = Instant::now();
        let due: Vec<usize> = self
            .timers
            .iter()
            .enumerate()
            .filter(|(_, (t, _))| *t <= now)
            .map(|(i, _)| i)
            .collect();
        for i in due.into_iter().rev() {
            let (_, action) = self.timers.remove(i);
            match action {
                TimerAction::RefreshSecurity => self.refresh_security(),
                TimerAction::RefreshWallpapers => self.refresh_wallpapers(),
            }
        }
        // firewall worker results
        if let Some(rx) = &self.fw_rx {
            match rx.try_recv() {
                Ok(msg) => {
                    if let Some(w) = msg.strip_prefix("warn:") {
                        self.notify_warning(w, 5);
                    } else if let Some(e) = msg.strip_prefix("err:") {
                        self.notify_error(e);
                    }
                    self.refresh_security();
                    self.fw_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.fw_rx = None,
            }
        }
        // toast expiry
        if let Some((_, _, at, timeout)) = &self.toast {
            if at.elapsed() > *timeout {
                self.toast = None;
            }
        }
    }

    // ---------- drawing ----------

    pub fn draw(&mut self, f: &mut Frame) {
        self.hits.clear();
        let pal = self.pal.clone();
        let area = f.area();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        // header
        let hdr = Line::from(vec![
            Span::styled(
                "󰒓  Hypr Settings",
                Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   click, or ↑↓ Tab Enter — no typing needed",
                Style::default().fg(color(&pal.subtext)),
            ),
        ]);
        f.render_widget(Paragraph::new(hdr), rows[0].inner(Margin::new(2, 0)));

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(10)])
            .split(rows[1]);
        self.draw_sidebar(f, body[0]);
        match self.page {
            0 => self.draw_appearance(f, body[1]),
            1 => self.draw_bar(f, body[1]),
            2 => self.draw_wallpaper(f, body[1]),
            3 => self.draw_security(f, body[1]),
            _ => self.draw_power(f, body[1]),
        }

        // footer
        let footer = Line::from(vec![
            Span::styled(" q ", Style::default().fg(color(&pal.accent))),
            Span::styled("Quit  ", Style::default().fg(color(&pal.subtext))),
            Span::styled("1-5 ", Style::default().fg(color(&pal.accent))),
            Span::styled("Pages  ", Style::default().fg(color(&pal.subtext))),
            Span::styled("Tab/↑↓ ", Style::default().fg(color(&pal.accent))),
            Span::styled("Focus  ", Style::default().fg(color(&pal.subtext))),
            Span::styled("Enter ", Style::default().fg(color(&pal.accent))),
            Span::styled("Activate", Style::default().fg(color(&pal.subtext))),
        ]);
        f.render_widget(Paragraph::new(footer), rows[2]);

        if let Some((msg, style, _, _)) = &self.toast {
            let w = (msg.chars().count() as u16 + 4).min(area.width.saturating_sub(2)).max(20);
            let lines = 1 + msg.chars().count() as u16 / w.saturating_sub(4).max(1);
            let h = (lines + 2).min(6);
            let rect = Rect {
                x: area.width.saturating_sub(w + 1),
                y: area.height.saturating_sub(h + 1),
                width: w,
                height: h,
            };
            f.render_widget(Clear, rect);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color(&pal.accent)));
            f.render_widget(
                Paragraph::new(msg.as_str()).style(*style).wrap(Wrap { trim: false }).block(block),
                rect,
            );
        }
        if self.modal.is_some() {
            self.draw_modal(f, area);
        }
    }

    fn draw_sidebar(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let inner = area.inner(Margin::new(1, 1));
        let mut y = inner.y;
        for (i, (_, icon, name)) in NAV.iter().enumerate() {
            if y + 1 >= inner.bottom() {
                break;
            }
            let rect = Rect { x: inner.x, y, width: inner.width, height: 2 };
            let sel = i == self.page;
            let (bar, style) = if sel {
                (
                    Span::styled("▌", Style::default().fg(color(&pal.accent))),
                    Style::default()
                        .fg(color(&pal.text))
                        .bg(color(&pal.surface))
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (Span::raw(" "), Style::default().fg(color(&pal.subtext)))
            };
            let text = format!("{icon}  {name}");
            let row = Rect { height: 1, ..rect };
            f.render_widget(
                Paragraph::new(Line::from(vec![bar, Span::styled(format!(" {text:<18}"), style)])),
                row,
            );
            self.hits.push((row, Wid::Nav(i)));
            y += 2;
        }
    }

    fn card<'a>(&self, title: &'a str, danger: bool) -> Block<'a> {
        let pal = &self.pal;
        let bcol = if danger { color(&pal.danger) } else { color(&pal.surface) };
        let tcol = if danger { color(&pal.danger) } else { color(&pal.accent) };
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(bcol))
            .title(Span::styled(
                format!(" {title} "),
                Style::default().fg(tcol).add_modifier(Modifier::BOLD),
            ))
    }

    /// A bordered mini-button; records its hit rect. Returns width used.
    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        f: &mut Frame,
        x: u16,
        y: u16,
        label: &str,
        wid: Wid,
        focused: bool,
        variant: &str,
        max_right: u16,
    ) -> u16 {
        let pal = self.pal.clone();
        let w = (label.chars().count() as u16 + 4).min(max_right.saturating_sub(x));
        if w < 4 || y + 3 > f.area().bottom() {
            return 0;
        }
        let rect = Rect { x, y, width: w, height: 3 };
        let (fg, border, bg) = match variant {
            "primary" => (color(&pal.base), color(&pal.accent), Some(color(&pal.accent))),
            "error" => (color(&pal.danger), color(&pal.danger), None),
            "warning" => (color(&pal.yellow), color(&pal.yellow), None),
            _ => (color(&pal.text), color(&pal.surface), None),
        };
        let border = if focused { color(&pal.accent) } else { border };
        let mut style = Style::default().fg(fg);
        if let Some(bg) = bg {
            style = style.bg(bg).add_modifier(Modifier::BOLD);
        }
        if focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border));
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {label} "), style)).block(block),
            rect,
        );
        self.hits.push((rect, wid));
        w + 1
    }

    fn switch(&mut self, f: &mut Frame, x: u16, y: u16, on: bool, wid: Wid, focused: bool) -> u16 {
        let pal = self.pal.clone();
        let rect = Rect { x, y, width: 10, height: 3 };
        if y + 3 > f.area().bottom() {
            return 0;
        }
        let border = if focused { color(&pal.accent) } else { color(&pal.surface) };
        let slider = if on {
            Span::styled("   ████", Style::default().fg(color(&pal.green)))
        } else {
            Span::styled("████   ", Style::default().fg(color(&pal.surface)))
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border));
        f.render_widget(Paragraph::new(Line::from(slider)).block(block), rect);
        self.hits.push((rect, wid));
        11
    }

    fn draw_appearance(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let n_rows = INTS.len() as u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(n_rows * 3 + 2),
                Constraint::Length(BOOLS.len() as u16 * 3 + 2),
                Constraint::Length(6),
                Constraint::Min(0),
            ])
            .split(content);

        let focus_wid = self.focusables().get(self.focus).copied();

        // Layout card
        let card = self.card("Layout · applies live as you click", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        for (i, (key, _, _, lo, _, label)) in INTS.iter().enumerate() {
            let y = inner.y + i as u16 * 3;
            if y + 3 > inner.bottom() + 1 {
                break;
            }
            f.render_widget(
                Paragraph::new(Span::styled(*label, Style::default().fg(color(&pal.text)))),
                Rect { x: inner.x + 1, y: y + 1, width: 30.min(inner.width), height: 1 },
            );
            let mut x = inner.x + 31;
            if let Some(raw) = self.multi.get(key) {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!("{raw} · per-side — edit in config"),
                        Style::default().fg(color(&pal.subtext)),
                    )),
                    Rect { x, y: y + 1, width: inner.width.saturating_sub(31), height: 1 },
                );
                continue;
            }
            let val = self.ints[key];
            let hi = self.limits[key];
            x += self.button(f, x, y, "-", Wid::Dec(i), focus_wid == Some(Wid::Dec(i)), "", inner.right());
            let filled = gauge_filled(val, *lo, hi);
            let gauge = Line::from(vec![
                Span::styled("█".repeat(filled), Style::default().fg(color(&pal.accent))),
                Span::styled("░".repeat(GAUGE_SLOTS - filled), Style::default().fg(color(&pal.surface))),
            ]);
            f.render_widget(
                Paragraph::new(gauge),
                Rect { x, y: y + 1, width: GAUGE_SLOTS as u16, height: 1 },
            );
            x += GAUGE_SLOTS as u16 + 1;
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("{val:>3}"),
                    Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
                )),
                Rect { x, y: y + 1, width: 4, height: 1 },
            );
            x += 4;
            self.button(f, x, y, "+", Wid::Inc(i), focus_wid == Some(Wid::Inc(i)), "", inner.right());
        }

        // Effects card
        let card = self.card("Effects", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        for (i, (key, _, _, label)) in BOOLS.iter().enumerate() {
            let y = inner.y + i as u16 * 3;
            f.render_widget(
                Paragraph::new(Span::styled(*label, Style::default().fg(color(&pal.text)))),
                Rect { x: inner.x + 1, y: y + 1, width: 30.min(inner.width), height: 1 },
            );
            self.switch(
                f,
                inner.x + 31,
                y,
                self.bools[key],
                Wid::Switch(i),
                focus_wid == Some(Wid::Switch(i)),
            );
        }

        // Save card
        let card = self.card("Keep your changes", false);
        let inner = card.inner(chunks[2]);
        f.render_widget(card, chunks[2]);
        let mut x = inner.x + 1;
        x += self.button(
            f, x, inner.y, "󰆓 Save (survives reboot)", Wid::Save,
            focus_wid == Some(Wid::Save), "primary", inner.right(),
        );
        self.button(
            f, x, inner.y, "󰑓 Revert to saved", Wid::Revert,
            focus_wid == Some(Wid::Revert), "", inner.right(),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Live tweaks are lost at logout unless saved.",
                Style::default().fg(color(&pal.subtext)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 1 },
        );
    }

    fn draw_bar(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Length(6), Constraint::Min(0)])
            .split(content);
        let focus_wid = self.focusables().get(self.focus).copied();

        let card = self.card("Top bar (waybar)", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        f.render_widget(
            Paragraph::new(Span::styled(
                "Auto-hide (reveal at top edge)",
                Style::default().fg(color(&pal.text)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 1, width: 30.min(inner.width), height: 1 },
        );
        self.switch(
            f, inner.x + 31, inner.y, self.autohide, Wid::AutohideSwitch,
            focus_wid == Some(Wid::AutohideSwitch),
        );
        let mut x = inner.x + 1;
        x += self.button(
            f, x, inner.y + 3, "󰊠 Hide / show now", Wid::BarToggle,
            focus_wid == Some(Wid::BarToggle), "", inner.right(),
        );
        self.button(
            f, x, inner.y + 3, "󰑓 Restart bar", Wid::BarRestart,
            focus_wid == Some(Wid::BarRestart), "", inner.right(),
        );

        let card = self.card("Layout", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        self.button(
            f, inner.x + 1, inner.y, "󰜬 Reorder bar buttons…", Wid::BarReorder,
            focus_wid == Some(Wid::BarReorder), "", inner.right(),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Opens the pick-and-place chooser.",
                Style::default().fg(color(&pal.subtext)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 1 },
        );
    }

    fn draw_wallpaper(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let wp_lines = self.wallpapers.len().max(1) as u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(wp_lines + 3), Constraint::Length(8), Constraint::Min(0)])
            .split(content);
        let focus_wid = self.focusables().get(self.focus).copied();

        let card = self.card("Current wallpapers", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        let mut lines: Vec<Line> = self
            .wallpapers
            .iter()
            .map(|(m, p)| {
                Line::from(vec![
                    Span::styled(m.clone(), Style::default().fg(color(&pal.accent))),
                    Span::styled(format!("  →  {p}"), Style::default().fg(color(&pal.text))),
                ])
            })
            .collect();
        if lines.is_empty() {
            lines.push(Line::from("(none set)"));
        }
        f.render_widget(Paragraph::new(lines), inner.inner(Margin::new(1, 0)));

        let card = self.card("Change", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        let mut x = inner.x + 1;
        x += self.button(
            f, x, inner.y, "󰒝 Random wallpaper", Wid::WpRandom,
            focus_wid == Some(Wid::WpRandom), "", inner.right(),
        );
        self.button(
            f, x, inner.y, "󰋩 Pick image / folder…", Wid::WpPick,
            focus_wid == Some(Wid::WpPick), "", inner.right(),
        );
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "Every change recolors the whole desktop.",
                    Style::default().fg(color(&pal.subtext)),
                ),
                Line::styled(
                    "This panel wears the new colors next time it opens.",
                    Style::default().fg(color(&pal.subtext)),
                ),
            ]),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 2 },
        );
    }

    fn draw_security(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(7), Constraint::Min(0)])
            .split(content);
        let focus_wid = self.focusables().get(self.focus).copied();

        let dot = |ok: bool, on: &str, off: &str| -> Line<'static> {
            if ok {
                Line::from(vec![
                    Span::styled("● ", Style::default().fg(color(&pal.green))),
                    Span::raw(on.to_string()),
                ])
            } else {
                Line::from(vec![
                    Span::styled("● ", Style::default().fg(color(&pal.danger))),
                    Span::raw(off.to_string()),
                ])
            }
        };

        let card = self.card("Firewall · ufw", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        f.render_widget(
            Paragraph::new(dot(
                self.fw_on,
                "Firewall is ON — incoming connections blocked",
                "Firewall is OFF — click Enable to protect this machine",
            )),
            Rect { x: inner.x + 1, y: inner.y, width: inner.width.saturating_sub(2), height: 1 },
        );
        let fw_label = if self.fw_on { "󰕥 Disable firewall" } else { "󰕥 Enable firewall" };
        let mut x = inner.x + 1;
        x += self.button(
            f, x, inner.y + 1, fw_label, Wid::FwToggle,
            focus_wid == Some(Wid::FwToggle), "", inner.right(),
        );
        self.button(
            f, x, inner.y + 1, "󰈙 View rules", Wid::FwRules,
            focus_wid == Some(Wid::FwRules), "", inner.right(),
        );

        let card = self.card("Antivirus · ClamAV", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        f.render_widget(
            Paragraph::new(dot(
                self.av_on,
                "Scanner daemon running — scans are fast",
                "Scanner daemon not running — scans still work, just slower",
            )),
            Rect { x: inner.x + 1, y: inner.y, width: inner.width.saturating_sub(2), height: 1 },
        );
        let mut x = inner.x + 1;
        x += self.button(
            f, x, inner.y + 1, "󰃤 Scan Downloads", Wid::AvScan,
            focus_wid == Some(Wid::AvScan), "", inner.right(),
        );
        x += self.button(
            f, x, inner.y + 1, "󰖟 Open ClamTk", Wid::AvGui,
            focus_wid == Some(Wid::AvGui), "", inner.right(),
        );
        self.button(
            f, x, inner.y + 1, "󰚰 Update definitions", Wid::AvUpdate,
            focus_wid == Some(Wid::AvUpdate), "", inner.right(),
        );
    }

    fn draw_power(&mut self, f: &mut Frame, area: Rect) {
        let content = area.inner(Margin::new(1, 0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Length(11), Constraint::Min(0)])
            .split(content);
        let focus_wid = self.focusables().get(self.focus).copied();

        let card = self.card("Session", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        self.button(
            f, inner.x + 1, inner.y, "󰌾 Lock screen                     ", Wid::PLock,
            focus_wid == Some(Wid::PLock), "", inner.right(),
        );
        self.button(
            f, inner.x + 1, inner.y + 3, "󰤄 Suspend (sleep)                 ", Wid::PSuspend,
            focus_wid == Some(Wid::PSuspend), "", inner.right(),
        );

        let card = self.card("Danger zone · asks before acting", true);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        self.button(
            f, inner.x + 1, inner.y, "󰍃 Logout                          ", Wid::PLogout,
            focus_wid == Some(Wid::PLogout), "warning", inner.right(),
        );
        self.button(
            f, inner.x + 1, inner.y + 3, "󰜉 Reboot                          ", Wid::PReboot,
            focus_wid == Some(Wid::PReboot), "warning", inner.right(),
        );
        self.button(
            f, inner.x + 1, inner.y + 6, "⏻ Shutdown                        ", Wid::PShutdown,
            focus_wid == Some(Wid::PShutdown), "error", inner.right(),
        );
    }

    fn draw_modal(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let Some(m) = &self.modal else { return };
        let question = m.question.clone();
        let yes_focused = m.yes_focused;
        let w = 56.min(area.width.saturating_sub(4));
        let h = 7;
        let rect = Rect {
            x: (area.width.saturating_sub(w)) / 2,
            y: (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(&pal.danger)))
            .style(Style::default().bg(color(&pal.base)));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        f.render_widget(
            Paragraph::new(question).alignment(ratatui::layout::Alignment::Center),
            Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 },
        );
        let bw = 14u16;
        let total = bw * 2 + 4;
        let bx = inner.x + (inner.width.saturating_sub(total)) / 2;
        let no_rect = Rect { x: bx, y: inner.y + 2, width: bw, height: 3 };
        let yes_rect = Rect { x: bx + bw + 4, y: inner.y + 2, width: bw, height: 3 };
        let mk = |label: &str, focused: bool, danger: bool| {
            let bcol = if danger { color(&pal.danger) } else { color(&pal.accent) };
            let mut style = Style::default().fg(bcol);
            if focused {
                style = style.add_modifier(Modifier::BOLD).add_modifier(Modifier::REVERSED);
            }
            Paragraph::new(Span::styled(label.to_string(), style))
                .alignment(ratatui::layout::Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(bcol)),
                )
        };
        f.render_widget(mk("No — stay", !yes_focused, false), no_rect);
        f.render_widget(mk("Yes", yes_focused, true), yes_rect);
        self.hits.push((no_rect, Wid::ModalNo));
        self.hits.push((yes_rect, Wid::ModalYes));
    }
}

fn color(hex: &str) -> Color {
    let (r, g, b) = colors::hex_rgb(hex);
    Color::Rgb(r, g, b)
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|d| !d.is_empty() && std::path::Path::new(d).join(bin).is_file())
}

// ---------- entry point ----------

pub fn run(args: &[&str]) -> ExitCode {
    let initial_page = args.first().copied().filter(|s| !s.is_empty());
    let mut app = App::new(initial_page);

    // HYPR_BENCH_STARTUP=1: draw one frame, then exit — used by the
    // benchmark harness to measure time-to-first-frame.
    let bench = std::env::var("HYPR_BENCH_STARTUP").map(|v| v == "1").unwrap_or(false);

    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hypr-settings: cannot init terminal: {e}");
            return util::fail_exit();
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);

    let mut last_tick = Instant::now();
    loop {
        let _ = terminal.draw(|f| app.draw(f));
        if bench {
            break;
        }
        match crossterm::event::poll(Duration::from_millis(100)) {
            Ok(true) => match crossterm::event::read() {
                Ok(Event::Key(k)) if k.kind != crossterm::event::KeyEventKind::Release => {
                    app.on_key(k)
                }
                Ok(Event::Mouse(m)) => app.on_mouse(m),
                Ok(_) => {}
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
        if last_tick.elapsed() >= Duration::from_millis(500) {
            app.tick();
            last_tick = Instant::now();
        }
        if app.should_quit {
            break;
        }
    }
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    util::ok_exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&[&str], &str)]) -> HashMap<Vec<String>, String> {
        pairs
            .iter()
            .map(|(p, v)| (p.iter().map(|s| s.to_string()).collect(), v.to_string()))
            .collect()
    }

    const CONF: &str = "\
# my config
general {
    gaps_in = 5
    gaps_out = 10   # outer
    border_size = 3
}
decoration {
    rounding = 8
    blur {
        enabled = true
    }
}
animations {
    enabled = true
}
input {
    kb_layout = us
}
";

    #[test]
    fn save_rewrites_matched_paths_only() {
        let values = vals(&[
            (&["general", "gaps_in"], "7"),
            (&["decoration", "blur", "enabled"], "false"),
        ]);
        let (out, written) = save_to_config(CONF, &values);
        assert!(out.contains("    gaps_in = 7\n"));
        assert!(out.contains("        enabled = false\n"));
        // untouched lines stay identical
        assert!(out.contains("    border_size = 3\n"));
        assert!(out.contains("    kb_layout = us\n"));
        // animations.enabled must NOT be touched by decoration.blur.enabled
        assert!(out.contains("animations {\n    enabled = true\n}\n"));
        assert_eq!(written.len(), 2);
    }

    #[test]
    fn save_preserves_inline_comments_and_indent() {
        let values = vals(&[(&["general", "gaps_out"], "20")]);
        let (out, written) = save_to_config(CONF, &values);
        assert!(out.contains("    gaps_out = 20   # outer\n"), "comment kept: {out}");
        assert_eq!(written.len(), 1);
    }

    #[test]
    fn save_reports_missing_lines() {
        let values = vals(&[(&["general", "no_such_key"], "1")]);
        let (out, written) = save_to_config(CONF, &values);
        assert_eq!(out, CONF);
        assert!(written.is_empty());
    }

    #[test]
    fn save_does_not_touch_same_key_in_other_block() {
        // "enabled" exists in decoration.blur and animations
        let values = vals(&[(&["animations", "enabled"], "false")]);
        let (out, _) = save_to_config(CONF, &values);
        assert!(out.contains("blur {\n        enabled = true\n"), "blur untouched");
        assert!(out.contains("animations {\n    enabled = false\n"), "animations changed");
    }

    #[test]
    fn save_roundtrip_no_values_is_identity() {
        let (out, written) = save_to_config(CONF, &HashMap::new());
        assert_eq!(out, CONF);
        assert!(written.is_empty());
    }

    #[test]
    fn gauge_math_matches_python() {
        // python: round((val - lo) / (hi - lo) * GAUGE_SLOTS) clamped 0..12
        assert_eq!(gauge_filled(0, 0, 40), 0);
        assert_eq!(gauge_filled(40, 0, 40), 12);
        assert_eq!(gauge_filled(20, 0, 40), 6);
        assert_eq!(gauge_filled(5, 0, 40), 2); // 1.5 rounds to 2 (py: round(1.5)=2)
        assert_eq!(gauge_filled(60, 0, 40), 12); // clamped
    }

    #[test]
    fn app_stepper_clamps_and_dirties() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut app = App::new(None);
        let start = app.ints["gaps_in"];
        app.step(0, 1);
        assert_eq!(app.ints["gaps_in"], start + 1);
        assert!(app.dirty.contains("gaps_in"));
        // clamp at lo
        for _ in 0..100 {
            app.step(0, -1);
        }
        assert_eq!(app.ints["gaps_in"], 0);
        // clamp at hi (limit = 40 for gaps_in)
        for _ in 0..100 {
            app.step(0, 1);
        }
        assert_eq!(app.ints["gaps_in"], app.limits["gaps_in"]);
    }

    #[test]
    fn app_nav_and_focus() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut app = App::new(Some("power"));
        assert_eq!(app.page, 4);
        assert_eq!(app.focusables().len(), 5);
        app.goto_page("bar");
        assert_eq!(app.page, 1);
        // focus wraps
        app.focus = 0;
        app.on_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.focus, app.focusables().len() - 1);
    }

    #[test]
    fn app_save_reports_missing_in_dryrun() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        // point HOME at a temp dir with a minimal conf missing `rounding`
        let dir = std::env::temp_dir().join(format!("hs-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".config/hypr")).unwrap();
        std::fs::write(
            dir.join(".config/hypr/hyprland.conf"),
            "general {\n    gaps_in = 5\n}\n",
        )
        .unwrap();
        let old_home = std::env::var("HOME").unwrap();
        std::env::set_var("HOME", &dir);
        let mut app = App::new(None);
        app.ints.insert("gaps_in", 9);
        app.dirty.insert("gaps_in");
        app.ints.insert("rounding", 12);
        app.dirty.insert("rounding");
        app.do_save();
        std::env::set_var("HOME", old_home);
        let saved =
            std::fs::read_to_string(dir.join(".config/hypr/hyprland.conf")).unwrap();
        assert!(saved.contains("gaps_in = 9"));
        assert_eq!(app.last_save_missing, vec!["Corner rounding".to_string()]);
        // gaps_in written → no longer dirty; rounding still dirty
        assert!(!app.dirty.contains("gaps_in"));
        assert!(app.dirty.contains("rounding"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modal_confirm_yes_records_action() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut app = App::new(None);
        app.confirm("Really reboot?", &["systemctl", "reboot"]);
        assert!(app.modal.is_some());
        app.on_key(KeyEvent::from(KeyCode::Tab)); // focus Yes
        app.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(app.modal.is_none());
        let acts = util::recorded_actions();
        assert!(
            acts.iter().any(|a| a == &vec!["systemctl".to_string(), "reboot".to_string()]),
            "recorded: {acts:?}"
        );
    }

    #[test]
    fn modal_esc_cancels() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut app = App::new(None);
        app.confirm("Really log out?", &["hyprctl", "dispatch", "exit"]);
        app.on_key(KeyEvent::from(KeyCode::Esc));
        assert!(app.modal.is_none());
    }

    #[test]
    fn draw_smoke_test_all_pages() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let backend = ratatui::backend::TestBackend::new(100, 32);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        let mut app = App::new(None);
        for i in 0..NAV.len() {
            app.page = i;
            app.focus = 0;
            term.draw(|f| app.draw(f)).unwrap();
        }
        // and the modal
        app.confirm("Really shut down?", &["systemctl", "poweroff"]);
        term.draw(|f| app.draw(f)).unwrap();
    }
}
