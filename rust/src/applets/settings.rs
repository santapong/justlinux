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
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const GAUGE_SLOTS: usize = 12;

/// (key, hyprctl option, config path, min, max, label)
pub type IntSpec = (
    &'static str,
    &'static str,
    [&'static str; 2],
    i64,
    i64,
    &'static str,
);

pub const INTS: [IntSpec; 4] = [
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

pub const NAV: [(&str, &str, &str); 7] = [
    ("appearance", "󰉼", "Appearance"),
    ("theme", "󰏘", "Theme"),
    ("bar", "󰜬", "Bar"),
    ("wallpaper", "󰸉", "Wallpaper"),
    ("security", "󰕥", "Security"),
    ("system", "󰍛", "System"),
    ("power", "⏻", "Power"),
];

// page indices (NAV order)
const PG_APPEARANCE: usize = 0;
const PG_THEME: usize = 1;
const PG_BAR: usize = 2;
const PG_WALLPAPER: usize = 3;
const PG_SECURITY: usize = 4;
const PG_SYSTEM: usize = 5;
const PG_POWER: usize = 6;

/// Animation profiles shipped in ~/.config/hypr/animations/ (any extra
/// user .conf files are appended at runtime).
pub const ANIM_PROFILES: [&str; 4] = ["default", "snappy", "smooth", "off"];

/// Preferred order for waybar styles in ~/.config/waybar/styles/.
pub const BAR_STYLES: [&str; 4] = ["default", "islands", "glass", "minimal"];

pub const PRESET_SLOTS: usize = 3;

fn page_file() -> PathBuf {
    util::xdg_runtime().join("hypr-settings.page")
}

fn waybar_dir() -> PathBuf {
    util::home().join(".config/waybar")
}

fn animations_dir() -> PathBuf {
    util::home().join(".config/hypr/animations")
}

fn presets_dir() -> PathBuf {
    util::home().join(".config/hypr/presets")
}

/// "/* waybar-style: glass */" / "# animation profile: snappy" → "glass"/"snappy".
pub fn parse_marker(first_line: &str, key: &str) -> Option<String> {
    let pos = first_line.find(key)?;
    let rest = &first_line[pos + key.len()..];
    let name: String = rest
        .trim_start_matches(':')
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn marker_of(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_marker(text.lines().next()?, key)
}

/// Styles present in styles/, preferred order first, extras appended.
fn list_variants(dir: &Path, preferred: &[&str]) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("css")
                        || p.extension().and_then(|x| x.to_str()) == Some("conf")
                    {
                        p.file_stem().and_then(|s| s.to_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .filter(|n| n != "current")
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    let mut out: Vec<String> = Vec::new();
    for p in preferred {
        if found.iter().any(|f| f == p) {
            out.push(p.to_string());
        }
    }
    for f in found {
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---------- system metrics (System page) ----------

/// (total, idle) jiffies from a /proc/stat "cpu ..." first line.
pub fn parse_proc_stat(line: &str) -> Option<(u64, u64)> {
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let vals: Vec<u64> = it.filter_map(|t| t.parse().ok()).collect();
    if vals.len() < 5 {
        return None;
    }
    let total: u64 = vals.iter().sum();
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0); // idle + iowait
    Some((total, idle))
}

/// (total_kb, available_kb) from /proc/meminfo text.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut avail = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = rest.split_whitespace().next()?.parse().ok();
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail = rest.split_whitespace().next()?.parse().ok();
        }
        if total.is_some() && avail.is_some() {
            break;
        }
    }
    Some((total?, avail?))
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
    AnimProfile(usize),
    PresetApply(usize),
    PresetSave(usize),
    BarStyle(usize),
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
    // theme page
    pub anim_profiles: Vec<String>,
    pub anim_current: Option<String>,
    pub preset_summaries: Vec<Option<String>>,
    // bar page
    pub bar_styles: Vec<String>,
    pub bar_style_current: Option<String>,
    // system page
    cpu_hist: VecDeque<u64>,
    ram_hist: VecDeque<u64>,
    last_cpu_raw: Option<(u64, u64)>,
    ram_text: String,
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
            anim_profiles: Vec::new(),
            anim_current: None,
            preset_summaries: vec![None; PRESET_SLOTS],
            bar_styles: Vec::new(),
            bar_style_current: None,
            cpu_hist: VecDeque::with_capacity(120),
            ram_hist: VecDeque::with_capacity(120),
            last_cpu_raw: None,
            ram_text: String::new(),
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
        app.refresh_theme();
        app.refresh_bar_styles();
        app.sample_system();
        app
    }

    pub fn refresh_theme(&mut self) {
        self.anim_profiles = list_variants(&animations_dir(), &ANIM_PROFILES);
        self.anim_current = marker_of(&animations_dir().join("current.conf"), "animation profile");
        for i in 0..PRESET_SLOTS {
            self.preset_summaries[i] = self.preset_summary(i);
        }
    }

    pub fn refresh_bar_styles(&mut self) {
        self.bar_styles = list_variants(&waybar_dir().join("styles"), &BAR_STYLES);
        self.bar_style_current = marker_of(&waybar_dir().join("style.css"), "waybar-style");
    }

    fn preset_path(i: usize) -> PathBuf {
        presets_dir().join(format!("slot{}.json", i + 1))
    }

    fn preset_summary(&self, i: usize) -> Option<String> {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(Self::preset_path(i)).ok()?).ok()?;
        let g = |k: &str| v.pointer(&format!("/ints/{k}")).and_then(serde_json::Value::as_i64);
        let b = |k: &str| {
            v.pointer(&format!("/bools/{k}"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let fx = |on: bool| if on { "✓" } else { "✗" };
        Some(format!(
            "gaps {}/{} · border {} · round {} · blur{} shadow{} anim{}",
            g("gaps_in")?,
            g("gaps_out")?,
            g("border_size")?,
            g("rounding")?,
            fx(b("blur")),
            fx(b("shadow")),
            fx(b("animations")),
        ))
    }

    pub fn save_preset(&mut self, i: usize) {
        let ints: serde_json::Map<String, serde_json::Value> = INTS
            .iter()
            .map(|(k, ..)| (k.to_string(), serde_json::json!(self.ints[k])))
            .collect();
        let bools: serde_json::Map<String, serde_json::Value> = BOOLS
            .iter()
            .map(|(k, ..)| (k.to_string(), serde_json::json!(self.bools[k])))
            .collect();
        let _ = std::fs::create_dir_all(presets_dir());
        let data = serde_json::json!({ "ints": ints, "bools": bools });
        if std::fs::write(Self::preset_path(i), serde_json::to_string_pretty(&data).unwrap_or_default())
            .is_ok()
        {
            self.preset_summaries[i] = self.preset_summary(i);
            self.notify(&format!("Saved current appearance to preset {}", i + 1));
        } else {
            self.notify_error(&format!("Could not write preset {}", i + 1));
        }
    }

    pub fn apply_preset(&mut self, i: usize) {
        let Ok(text) = std::fs::read_to_string(Self::preset_path(i)) else {
            self.notify(&format!("Preset {} is empty — Save stores the current look", i + 1));
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            self.notify_error(&format!("Preset {} is unreadable", i + 1));
            return;
        };
        for (key, opt, _, lo, hi, _) in INTS {
            if self.multi.contains_key(key) {
                continue;
            }
            if let Some(val) = v.pointer(&format!("/ints/{key}")).and_then(serde_json::Value::as_i64)
            {
                let val = val.max(lo);
                self.limits.insert(key, hi.max(val));
                self.ints.insert(key, val);
                self.dirty.insert(key);
                hypr::keyword(opt, &val.to_string());
            }
        }
        for (key, opt, _, _) in BOOLS {
            if let Some(val) =
                v.pointer(&format!("/bools/{key}")).and_then(serde_json::Value::as_bool)
            {
                self.bools.insert(key, val);
                self.dirty.insert(key);
                hypr::keyword(opt, if val { "1" } else { "0" });
            }
        }
        self.notify(&format!("Preset {} applied live — Save keeps it after reboot", i + 1));
    }

    pub fn apply_anim_profile(&mut self, idx: usize) {
        let Some(name) = self.anim_profiles.get(idx).cloned() else { return };
        let src = animations_dir().join(format!("{name}.conf"));
        let dst = animations_dir().join("current.conf");
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                self.anim_current = Some(name.clone());
                hypr::reload();
                self.notify(&format!("Animation profile: {}", cap_first(&name)));
            }
            Err(e) => self.notify_error(&format!("Could not apply profile {name}: {e}")),
        }
    }

    pub fn apply_bar_style(&mut self, idx: usize) {
        let Some(name) = self.bar_styles.get(idx).cloned() else { return };
        let src = waybar_dir().join("styles").join(format!("{name}.css"));
        let dst = waybar_dir().join("style.css");
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                self.bar_style_current = Some(name.clone());
                util::spawn_detached(&[
                    &util::local_bin("hypr-tools.sh").to_string_lossy(),
                    "restart-bar",
                ]);
                self.notify(&format!("Bar style: {} — restarting waybar", cap_first(&name)));
            }
            Err(e) => self.notify_error(&format!("Could not apply bar style {name}: {e}")),
        }
    }

    /// Sample CPU/RAM for the System page sparklines (500 ms cadence).
    fn sample_system(&mut self) {
        if let Ok(stat) = std::fs::read_to_string("/proc/stat") {
            if let Some(raw) = stat.lines().next().and_then(parse_proc_stat) {
                if let Some((pt, pi)) = self.last_cpu_raw {
                    let dt = raw.0.saturating_sub(pt);
                    let di = raw.1.saturating_sub(pi);
                    if dt > 0 {
                        let pct = (100 * (dt - di.min(dt))) / dt;
                        self.cpu_hist.push_back(pct);
                        if self.cpu_hist.len() > 120 {
                            self.cpu_hist.pop_front();
                        }
                    }
                }
                self.last_cpu_raw = Some(raw);
            }
        }
        if let Ok(mi) = std::fs::read_to_string("/proc/meminfo") {
            if let Some((total, avail)) = parse_meminfo(&mi) {
                let used = total.saturating_sub(avail);
                let pct = if total > 0 { used * 100 / total } else { 0 };
                self.ram_hist.push_back(pct);
                if self.ram_hist.len() > 120 {
                    self.ram_hist.pop_front();
                }
                self.ram_text = format!(
                    "{:.1} / {:.1} GB ({pct}%)",
                    used as f64 / 1048576.0,
                    total as f64 / 1048576.0
                );
            }
        }
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
        self.refresh_theme();
        self.refresh_bar_styles();
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
            PG_APPEARANCE => {
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
            PG_THEME => {
                for i in 0..self.anim_profiles.len() {
                    v.push(Wid::AnimProfile(i));
                }
                for i in 0..PRESET_SLOTS {
                    v.push(Wid::PresetApply(i));
                    v.push(Wid::PresetSave(i));
                }
            }
            PG_BAR => {
                v.push(Wid::AutohideSwitch);
                v.push(Wid::BarToggle);
                v.push(Wid::BarRestart);
                for i in 0..self.bar_styles.len() {
                    v.push(Wid::BarStyle(i));
                }
                v.push(Wid::BarReorder);
            }
            PG_WALLPAPER => {
                v.push(Wid::WpRandom);
                v.push(Wid::WpPick);
            }
            PG_SECURITY => {
                v.push(Wid::FwToggle);
                v.push(Wid::FwRules);
                v.push(Wid::AvScan);
                v.push(Wid::AvGui);
                v.push(Wid::AvUpdate);
            }
            PG_SYSTEM => {} // read-only page
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
            Wid::AnimProfile(i) => self.apply_anim_profile(i),
            Wid::PresetApply(i) => self.apply_preset(i),
            Wid::PresetSave(i) => self.save_preset(i),
            Wid::BarStyle(i) => self.apply_bar_style(i),
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
            KeyCode::Char(c @ '1'..='7') => {
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
        // CPU/RAM history for the System page (cheap: two /proc reads)
        self.sample_system();
        // re-check auto-hide state every 2s while the Bar page shows
        if self.poll_tick.is_multiple_of(4) && self.page == PG_BAR {
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
            PG_APPEARANCE => self.draw_appearance(f, body[1]),
            PG_THEME => self.draw_theme(f, body[1]),
            PG_BAR => self.draw_bar(f, body[1]),
            PG_WALLPAPER => self.draw_wallpaper(f, body[1]),
            PG_SECURITY => self.draw_security(f, body[1]),
            PG_SYSTEM => self.draw_system(f, body[1]),
            PG_POWER => self.draw_power(f, body[1]),
            _ => {}
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
        // border_hi: raised-card tone from the tonal ladder, not raw surface
        let tones = pal.tones();
        let bcol = if danger { color(&pal.danger) } else { color(&tones.border_hi) };
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

    /// Row of variant buttons where the active one is marked ●.
    #[allow(clippy::too_many_arguments)]
    fn variant_row(
        &mut self,
        f: &mut Frame,
        x0: u16,
        y: u16,
        names: &[String],
        current: Option<&str>,
        wid_of: fn(usize) -> Wid,
        focus_wid: Option<Wid>,
        max_right: u16,
    ) {
        let mut x = x0;
        for (i, name) in names.iter().enumerate() {
            let active = current == Some(name.as_str());
            let label = if active {
                format!("● {}", cap_first(name))
            } else {
                cap_first(name)
            };
            let variant = if active { "primary" } else { "" };
            let w = self.button(f, x, y, &label, wid_of(i), focus_wid == Some(wid_of(i)), variant, max_right);
            if w == 0 {
                break;
            }
            x += w;
        }
    }

    fn draw_theme(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Length(PRESET_SLOTS as u16 * 3 + 3),
                Constraint::Min(0),
            ])
            .split(content);
        let focus_wid = self.focusables().get(self.focus).copied();

        // Animation profile card
        let card = self.card("Animations · swaps the whole profile live", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        let names = self.anim_profiles.clone();
        let current = self.anim_current.clone();
        self.variant_row(
            f, inner.x + 1, inner.y, &names, current.as_deref(),
            Wid::AnimProfile, focus_wid, inner.right(),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "default = original feel · snappy = short, no overshoot · smooth = floaty",
                Style::default().fg(color(&pal.subtext)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 1 },
        );

        // Presets card
        let card = self.card("Presets · bundles of gaps / borders / effects", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        for i in 0..PRESET_SLOTS {
            let y = inner.y + (i as u16) * 3;
            let summary = self
                .preset_summaries
                .get(i)
                .cloned()
                .flatten()
                .unwrap_or_else(|| "(empty — Save stores the current look)".into());
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("Slot {} ", i + 1),
                        Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(summary, Style::default().fg(color(&pal.subtext))),
                ])),
                Rect { x: inner.x + 1, y: y + 1, width: inner.width.saturating_sub(24), height: 1 },
            );
            let bx = inner.right().saturating_sub(22);
            let mut x = bx;
            x += self.button(
                f, x, y, "Apply", Wid::PresetApply(i),
                focus_wid == Some(Wid::PresetApply(i)), "", inner.right(),
            );
            self.button(
                f, x, y, "Save", Wid::PresetSave(i),
                focus_wid == Some(Wid::PresetSave(i)), "", inner.right(),
            );
        }
        f.render_widget(
            Paragraph::new(Span::styled(
                "Apply changes the live session; use Appearance → Save to persist.",
                Style::default().fg(color(&pal.subtext)),
            )),
            Rect {
                x: inner.x + 1,
                y: inner.y + PRESET_SLOTS as u16 * 3,
                width: inner.width.saturating_sub(2),
                height: 1,
            },
        );
    }

    fn draw_system(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let content = area.inner(Margin::new(1, 0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(7), Constraint::Min(0)])
            .split(content);

        let cpu_now = self.cpu_hist.back().copied().unwrap_or(0);
        let card = self.card("CPU", false);
        let inner = card.inner(chunks[0]);
        f.render_widget(card, chunks[0]);
        let cpu_data: Vec<u64> = self
            .cpu_hist
            .iter()
            .rev()
            .take(inner.width.saturating_sub(8) as usize)
            .rev()
            .copied()
            .collect();
        f.render_widget(
            ratatui::widgets::Sparkline::default()
                .data(&cpu_data)
                .max(100)
                .style(Style::default().fg(color(&pal.accent))),
            Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(8),
                height: inner.height.min(4),
            },
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{cpu_now:>3}%"),
                Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
            )),
            Rect { x: inner.right().saturating_sub(5), y: inner.y + 1, width: 5, height: 1 },
        );

        let card = self.card("Memory", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        let ram_data: Vec<u64> = self
            .ram_hist
            .iter()
            .rev()
            .take(inner.width.saturating_sub(2) as usize)
            .rev()
            .copied()
            .collect();
        f.render_widget(
            ratatui::widgets::Sparkline::default()
                .data(&ram_data)
                .max(100)
                .style(Style::default().fg(color(&pal.green))),
            Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: inner.height.min(3),
            },
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                self.ram_text.clone(),
                Style::default().fg(color(&pal.text)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 1 },
        );
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
                    // accent_hi: lifted accent, readable at small sizes
                    Style::default()
                        .fg(color(&pal.tones().accent_hi))
                        .add_modifier(Modifier::BOLD),
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
            .constraints([
                Constraint::Length(8),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Min(0),
            ])
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

        // Bar style card (ML4W/JaKooLit-style runtime switcher)
        let card = self.card("Bar style · applies & restarts the bar", false);
        let inner = card.inner(chunks[1]);
        f.render_widget(card, chunks[1]);
        let names = self.bar_styles.clone();
        let current = self.bar_style_current.clone();
        self.variant_row(
            f, inner.x + 1, inner.y, &names, current.as_deref(),
            Wid::BarStyle, focus_wid, inner.right(),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "All styles wear the wallust palette — they recolor with the wallpaper.",
                Style::default().fg(color(&pal.subtext)),
            )),
            Rect { x: inner.x + 1, y: inner.y + 3, width: inner.width.saturating_sub(2), height: 1 },
        );

        let card = self.card("Layout", false);
        let inner = card.inner(chunks[2]);
        f.render_widget(card, chunks[2]);
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

    /// Tests that repoint $HOME must not overlap (env is process-global).
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_temp_home<R>(tag: &str, f: impl FnOnce(&Path) -> R) -> R {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("hs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config/hypr")).unwrap();
        let old_home = std::env::var("HOME").unwrap();
        std::env::set_var("HOME", &dir);
        let r = f(&dir);
        std::env::set_var("HOME", old_home);
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    #[test]
    fn app_nav_and_focus() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut app = App::new(Some("power"));
        assert_eq!(app.page, PG_POWER);
        assert_eq!(app.focusables().len(), 5);
        app.goto_page("bar");
        assert_eq!(app.page, PG_BAR);
        app.goto_page("system");
        assert_eq!(app.page, PG_SYSTEM);
        assert!(app.focusables().is_empty()); // read-only page
        app.goto_page("appearance");
        // focus wraps
        app.focus = 0;
        app.on_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.focus, app.focusables().len() - 1);
    }

    #[test]
    fn app_save_reports_missing_in_dryrun() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        with_temp_home("save", |dir| {
            // minimal conf missing `rounding`
            std::fs::write(
                dir.join(".config/hypr/hyprland.conf"),
                "general {\n    gaps_in = 5\n}\n",
            )
            .unwrap();
            let mut app = App::new(None);
            app.ints.insert("gaps_in", 9);
            app.dirty.insert("gaps_in");
            app.ints.insert("rounding", 12);
            app.dirty.insert("rounding");
            app.do_save();
            let saved =
                std::fs::read_to_string(dir.join(".config/hypr/hyprland.conf")).unwrap();
            assert!(saved.contains("gaps_in = 9"));
            assert_eq!(app.last_save_missing, vec!["Corner rounding".to_string()]);
            // gaps_in written → no longer dirty; rounding still dirty
            assert!(!app.dirty.contains("gaps_in"));
            assert!(app.dirty.contains("rounding"));
        });
    }

    #[test]
    fn marker_parsing_both_kinds() {
        assert_eq!(
            parse_marker("/* waybar-style: glass */", "waybar-style"),
            Some("glass".into())
        );
        assert_eq!(
            parse_marker("# animation profile: snappy", "animation profile"),
            Some("snappy".into())
        );
        assert_eq!(parse_marker("@import \"colors.css\";", "waybar-style"), None);
        assert_eq!(parse_marker("/* waybar-style:   */", "waybar-style"), None);
    }

    #[test]
    fn proc_stat_and_meminfo_parsers() {
        let (total, idle) =
            parse_proc_stat("cpu  100 0 50 800 50 0 0 0 0 0").unwrap();
        assert_eq!(total, 1000);
        assert_eq!(idle, 850); // idle + iowait
        assert!(parse_proc_stat("cpu0 1 2 3 4 5").is_none());
        let (t, a) = parse_meminfo("MemTotal: 16000000 kB\nMemFree: 1 kB\nMemAvailable: 4000000 kB\n").unwrap();
        assert_eq!(t, 16000000);
        assert_eq!(a, 4000000);
    }

    #[test]
    fn preset_save_apply_roundtrip() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        with_temp_home("preset", |_dir| {
            let mut app = App::new(None);
            app.ints.insert("gaps_in", 12);
            app.ints.insert("gaps_out", 24);
            app.bools.insert("blur", false);
            app.save_preset(0);
            assert!(app.preset_summaries[0].as_deref().unwrap().contains("gaps 12/24"));
            // change live values, then apply the preset back
            app.ints.insert("gaps_in", 0);
            app.bools.insert("blur", true);
            app.dirty.clear();
            app.apply_preset(0);
            assert_eq!(app.ints["gaps_in"], 12);
            assert!(!app.bools["blur"]);
            assert!(app.dirty.contains("gaps_in"));
            // applying an empty slot is a friendly no-op
            let before = app.ints.clone();
            app.apply_preset(2);
            assert_eq!(app.ints, before);
        });
    }

    #[test]
    fn bar_style_and_anim_profile_switch() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        with_temp_home("variants", |dir| {
            // waybar styles
            let wb = dir.join(".config/waybar");
            std::fs::create_dir_all(wb.join("styles")).unwrap();
            std::fs::write(wb.join("style.css"), "/* waybar-style: default */\nbody{}\n").unwrap();
            std::fs::write(wb.join("styles/default.css"), "/* waybar-style: default */\nbody{}\n").unwrap();
            std::fs::write(wb.join("styles/glass.css"), "/* waybar-style: glass */\nwin{}\n").unwrap();
            // animation profiles
            let an = dir.join(".config/hypr/animations");
            std::fs::create_dir_all(&an).unwrap();
            std::fs::write(an.join("current.conf"), "# animation profile: default\nanimations{}\n").unwrap();
            std::fs::write(an.join("default.conf"), "# animation profile: default\nanimations{}\n").unwrap();
            std::fs::write(an.join("snappy.conf"), "# animation profile: snappy\nanimations{}\n").unwrap();

            let mut app = App::new(None);
            app.refresh_bar_styles();
            app.refresh_theme();
            assert_eq!(app.bar_styles, vec!["default".to_string(), "glass".to_string()]);
            assert_eq!(app.bar_style_current.as_deref(), Some("default"));
            assert_eq!(app.anim_profiles, vec!["default".to_string(), "snappy".to_string()]);

            let gi = app.bar_styles.iter().position(|s| s == "glass").unwrap();
            app.apply_bar_style(gi);
            assert_eq!(app.bar_style_current.as_deref(), Some("glass"));
            let css = std::fs::read_to_string(wb.join("style.css")).unwrap();
            assert!(css.starts_with("/* waybar-style: glass */"));

            let si = app.anim_profiles.iter().position(|s| s == "snappy").unwrap();
            app.apply_anim_profile(si);
            assert_eq!(app.anim_current.as_deref(), Some("snappy"));
            let cur = std::fs::read_to_string(an.join("current.conf")).unwrap();
            assert!(cur.starts_with("# animation profile: snappy"));
        });
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
