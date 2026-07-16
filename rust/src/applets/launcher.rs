//! Hypr Launcher — app / window / wallpaper / tools pickers (ratatui).
//! Port of the python/textual launcher.
//!
//!   hypr-launcher apps        launch an application   (icon grid)
//!   hypr-launcher windows     focus an open window    (list)
//!   hypr-launcher wallpaper   pick wallpaper          (thumbnail grid + LIVE preview)
//!   hypr-launcher menu        tools hub               (tile grid)
//!
//! Apps & wallpaper draw real icons / thumbnails via the kitty graphics
//! protocol (kitty_img.rs); elsewhere tiles fall back to big glyphs.
//! State files (~/.local/state/hypr-launcher/state.json favorites/recent,
//! ~/.cache/hypr-launcher/*) keep the SAME format as the python version,
//! so favorites and caches survive the migration.

use crate::applets::kitty_img;
use crate::{colors, hypr, proc, util};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const IMG_EXTS: [&str; 6] = [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif"];
const MAX_TILES: usize = 160;
const FAV_TITLE: &str = "★ Favorites";
const REC_TITLE: &str = "󰥔 Recent";

fn cache_dir() -> PathBuf {
    util::home().join(".cache/hypr-launcher")
}
fn icon_cache() -> PathBuf {
    cache_dir().join("icons")
}
fn thumb_cache() -> PathBuf {
    cache_dir().join("thumbs")
}
fn state_file() -> PathBuf {
    util::home().join(".local/state/hypr-launcher/state.json")
}
fn apps_cache_file() -> PathBuf {
    cache_dir().join("apps.json")
}
fn resolve_cache_file() -> PathBuf {
    cache_dir().join("resolve.json")
}

fn load_json(path: &Path, default: Value) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(default)
}

fn save_json(path: &Path, data: &Value) {
    if util::dry() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string(data).unwrap_or_default());
}

// ---------------- state (favorites / recent) ----------------

pub struct AppState {
    pub favorites: Vec<String>,
    pub recent: HashMap<String, f64>,
}

fn load_state() -> AppState {
    let v = load_json(&state_file(), json!({}));
    let favorites = v
        .get("favorites")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let recent = v
        .get("recent")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f))).collect()
        })
        .unwrap_or_default();
    AppState { favorites, recent }
}

fn save_state(st: &AppState) {
    let mut rec = serde_json::Map::new();
    for (k, v) in &st.recent {
        rec.insert(k.clone(), json!(v));
    }
    save_json(&state_file(), &json!({ "favorites": st.favorites, "recent": rec }));
}

// ---------------- icons ----------------

fn flatpak_bases() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/var/lib/flatpak/exports/share"),
        util::home().join(".local/share/flatpak/exports/share"),
    ]
}

#[derive(Default, Clone)]
struct IconCand {
    svg: Option<String>,
    png: Option<String>,
    png_size: u32,
}

/// icon-name -> candidate paths; Flat-Remix SVG preferred.
fn build_icon_index() -> HashMap<String, IconCand> {
    let mut idx: HashMap<String, IconCand> = HashMap::new();
    let fr = Path::new("/usr/share/icons/Flat-Remix-Blue-Dark/apps/scalable");
    if fr.is_dir() {
        if let Ok(rd) = std::fs::read_dir(fr) {
            for f in rd.flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) == Some("svg") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        idx.entry(stem.to_string()).or_default().svg =
                            Some(p.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    let mut png_bases = vec![
        PathBuf::from("/usr/share/icons/hicolor"),
        util::home().join(".local/share/icons/hicolor"),
    ];
    png_bases.extend(flatpak_bases().into_iter().map(|b| b.join("icons/hicolor")));
    for base in png_bases {
        if !base.is_dir() {
            continue;
        }
        let mut stack = vec![base];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                    continue;
                };
                match p.extension().and_then(|e| e.to_str()) {
                    Some("png") => {
                        // size from …/64x64/… path component
                        let size = p
                            .to_string_lossy()
                            .split('/')
                            .find_map(|c| {
                                let (a, b) = c.split_once('x')?;
                                if a == b {
                                    a.parse::<u32>().ok()
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(32);
                        let d = idx.entry(stem).or_default();
                        if (24..=256).contains(&size) && size >= d.png_size {
                            d.png = Some(p.to_string_lossy().into_owned());
                            d.png_size = size;
                        }
                    }
                    Some("svg") => {
                        let d = idx.entry(stem).or_default();
                        if d.svg.is_none() {
                            d.svg = Some(p.to_string_lossy().into_owned());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let pm = Path::new("/usr/share/pixmaps");
    if pm.is_dir() {
        if let Ok(rd) = std::fs::read_dir(pm) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|e| e.to_str()) == Some("png") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        let d = idx.entry(stem.to_string()).or_default();
                        if d.png.is_none() {
                            d.png = Some(p.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    idx
}

fn stable_key(s: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// SVG -> cached 72px PNG via ImageMagick (one-time per icon).
fn rasterize_svg(svg_path: &str) -> Option<String> {
    if util::dry() {
        return None;
    }
    let out = icon_cache().join(format!("{}.png", stable_key(svg_path)));
    if out.exists() {
        return Some(out.to_string_lossy().into_owned());
    }
    let _ = std::fs::create_dir_all(icon_cache());
    let out_s = out.to_string_lossy().into_owned();
    let ok = std::process::Command::new("magick")
        .args(["-background", "none", svg_path, "-resize", "72x72", &out_s])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok && out.exists() {
        Some(out_s)
    } else {
        None
    }
}

/// Wallpaper image -> cached ~320x180 thumbnail PNG (magick instead of PIL).
fn make_thumb(img_path: &str) -> Option<String> {
    if util::dry() || img_path.is_empty() {
        return None;
    }
    let meta = std::fs::metadata(img_path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = stable_key(&format!("{img_path}:{mtime}"));
    let out = thumb_cache().join(format!("{key}.png"));
    if out.exists() {
        return Some(out.to_string_lossy().into_owned());
    }
    let _ = std::fs::create_dir_all(thumb_cache());
    let out_s = out.to_string_lossy().into_owned();
    let ok = std::process::Command::new("magick")
        .args([img_path, "-resize", "320x180", &out_s])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok && out.exists() {
        Some(out_s)
    } else {
        None
    }
}

/// Icon resolver with the persistent resolve.json cache (same file the
/// python launcher used — old entries stay valid).
struct IconResolver {
    idx: Option<HashMap<String, IconCand>>,
    cache: serde_json::Map<String, Value>,
    dirty: bool,
    default_icon: Option<Option<String>>,
}

impl IconResolver {
    fn new() -> Self {
        let cache = load_json(&resolve_cache_file(), json!({}))
            .as_object()
            .cloned()
            .unwrap_or_default();
        IconResolver { idx: None, cache, dirty: false, default_icon: None }
    }

    fn resolve(&mut self, icon_value: &str) -> Option<String> {
        if icon_value.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.get(icon_value) {
            match hit {
                Value::Null => return None,
                Value::String(s) if Path::new(s).exists() => return Some(s.clone()),
                _ => {}
            }
        }
        let result = self.resolve_uncached(icon_value);
        self.cache.insert(
            icon_value.to_string(),
            result.clone().map(Value::String).unwrap_or(Value::Null),
        );
        self.dirty = true;
        result
    }

    fn resolve_uncached(&mut self, icon_value: &str) -> Option<String> {
        let p = Path::new(icon_value);
        if p.is_absolute() && p.exists() {
            return if p.extension().and_then(|e| e.to_str()) == Some("svg") {
                rasterize_svg(icon_value)
            } else {
                Some(icon_value.to_string())
            };
        }
        if self.idx.is_none() {
            self.idx = Some(build_icon_index());
        }
        let idx = self.idx.as_ref().unwrap();
        let cand = idx
            .get(icon_value)
            .or_else(|| idx.get(icon_value.to_lowercase().as_str()))?
            .clone();
        if let Some(svg) = &cand.svg {
            if let Some(png) = rasterize_svg(svg) {
                return Some(png);
            }
        }
        cand.png
    }

    /// Theme's generic app icon, for apps whose own icon can't be found.
    fn default_app_icon(&mut self) -> Option<String> {
        if let Some(v) = &self.default_icon {
            return v.clone();
        }
        let mut found = None;
        for name in ["application-default-icon", "application-x-executable", "system-run"] {
            found = self.resolve(name);
            if found.is_some() {
                break;
            }
        }
        self.default_icon = Some(found.clone());
        found
    }

    fn flush(&mut self) {
        if self.dirty {
            save_json(&resolve_cache_file(), &Value::Object(self.cache.clone()));
            self.dirty = false;
        }
    }
}

// ---------------- entries ----------------

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub kind: &'static str, // app | dir | up | img | mon | tool | win
    pub label: String,
    pub detail: String,
    pub icon: String,
    pub exec: String,
    pub terminal: bool,
    pub cats: String,
    pub glyph: String,
    pub sub: String,
    pub path: String,   // dir/img/mon current wallpaper path
    pub target: String, // mon target
    pub addr: String,   // window address
    pub cmd: Vec<String>,
    pub fav: bool,
    pub grp: String,
}

fn glyph_for(kind: &str) -> &'static str {
    match kind {
        "app" => "󰘔",
        "dir" => "󰉋",
        "up" => "󰁍",
        "img" => "󰋩",
        "mon" => "󰍹",
        "tool" => "󰒓",
        _ => "󰘔",
    }
}

/// All .desktop locations: system, XDG_DATA_DIRS, flatpak, user (last wins).
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];
    for d in std::env::var("XDG_DATA_DIRS").unwrap_or_default().split(':') {
        if !d.is_empty() {
            dirs.push(PathBuf::from(d).join("applications"));
        }
    }
    dirs.extend(flatpak_bases().into_iter().map(|b| b.join("applications")));
    dirs.push(util::home().join(".local/share/applications"));
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// Parse one .desktop file's [Desktop Entry] section (first key wins).
pub fn parse_desktop_entry(text: &str) -> HashMap<String, String> {
    let mut in_main = false;
    let mut e: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if in_main {
            if let Some((k, v)) = line.split_once('=') {
                e.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
            }
        }
    }
    e
}

fn desktop_entries() -> Vec<Entry> {
    let dirs = app_dirs();
    let stamp: Vec<Value> = dirs
        .iter()
        .map(|d| {
            let mtime = std::fs::metadata(d)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|dur| dur.as_secs_f64())
                .unwrap_or(0.0);
            json!([d.to_string_lossy(), mtime])
        })
        .collect();
    let stamp = Value::Array(stamp);
    let cached = load_json(&apps_cache_file(), Value::Null);
    if cached.get("stamp") == Some(&stamp) {
        if let Some(arr) = cached.get("entries").and_then(Value::as_array) {
            let mut out = Vec::new();
            for v in arr {
                out.push(Entry {
                    kind: "app",
                    label: v.get("label").and_then(Value::as_str).unwrap_or("").into(),
                    detail: v.get("detail").and_then(Value::as_str).unwrap_or("").into(),
                    icon: v.get("icon").and_then(Value::as_str).unwrap_or("").into(),
                    exec: v.get("exec").and_then(Value::as_str).unwrap_or("").into(),
                    terminal: v.get("terminal").and_then(Value::as_bool).unwrap_or(false),
                    cats: v.get("cats").and_then(Value::as_str).unwrap_or("").into(),
                    ..Default::default()
                });
            }
            return out;
        }
    }
    let mut entries: HashMap<String, Entry> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for d in &dirs {
        // later dirs override earlier (user overrides system)
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
            .collect();
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let e = parse_desktop_entry(&text);
            if e.get("Type").map(String::as_str) != Some("Application") {
                continue;
            }
            let nodisplay = e.get("NoDisplay").map(|s| s.to_lowercase()).unwrap_or_default();
            let hidden = e.get("Hidden").map(|s| s.to_lowercase()).unwrap_or_default();
            if nodisplay == "true" || hidden == "true" {
                continue;
            }
            let (Some(name), Some(exec)) = (e.get("Name"), e.get("Exec")) else { continue };
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if !entries.contains_key(&stem) {
                order.push(stem.clone());
            }
            entries.insert(
                stem.clone(),
                Entry {
                    kind: "app",
                    label: name.clone(),
                    detail: stem,
                    icon: e.get("Icon").cloned().unwrap_or_default(),
                    exec: exec.clone(),
                    terminal: e.get("Terminal").map(|s| s.to_lowercase()) == Some("true".into()),
                    cats: e.get("Categories").cloned().unwrap_or_default(),
                    ..Default::default()
                },
            );
        }
    }
    let mut out: Vec<Entry> = order.into_iter().filter_map(|k| entries.remove(&k)).collect();
    out.sort_by_key(|e| e.label.to_lowercase());
    let cache_entries: Vec<Value> = out
        .iter()
        .map(|e| {
            json!({
                "kind": "app", "label": e.label, "detail": e.detail, "icon": e.icon,
                "exec": e.exec, "terminal": e.terminal, "cats": e.cats,
            })
        })
        .collect();
    save_json(&apps_cache_file(), &json!({ "stamp": stamp, "entries": cache_entries }));
    out
}

// (match order matters: first hit wins)
pub const CATEGORY_GROUPS: [(&str, &[&str]); 10] = [
    ("󰕥 Security", &["Security", "X-Kali"]),
    ("󰖟 Internet", &["Network", "WebBrowser", "Email", "Chat", "InstantMessaging"]),
    (" Development", &["Development", "IDE", "TextEditor"]),
    ("󰝚 Media", &["AudioVideo", "Audio", "Video", "Player", "Recorder"]),
    ("󰋩 Graphics", &["Graphics", "Photography"]),
    ("󰊗 Games", &["Game"]),
    ("󰈙 Office", &["Office", "WordProcessor", "Spreadsheet", "Presentation"]),
    ("󰭹 Science", &["Science", "Education", "Math"]),
    (" System", &["Settings", "System", "Monitor", "PackageManager"]),
    ("󰘵 Utilities", &["Utility", "Accessibility", "FileManager", "TerminalEmulator", "Archiving"]),
];

pub fn category_of(cats: &str) -> (usize, &'static str) {
    for (i, (title, needles)) in CATEGORY_GROUPS.iter().enumerate() {
        if needles.iter().any(|n| cats.contains(n)) {
            return (i, title);
        }
    }
    (CATEGORY_GROUPS.len(), "󰘔 Other")
}

/// shlex.split-alike, enough for .desktop Exec lines.
pub fn shlex_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                has_token = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                has_token = true;
            }
            '\\' if !in_single => {
                if let Some(nc) = chars.next() {
                    cur.push(nc);
                    has_token = true;
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

pub fn launch_argv(entry: &Entry) -> Vec<String> {
    let mut argv: Vec<String> =
        shlex_split(&entry.exec).into_iter().filter(|a| !a.starts_with('%')).collect();
    if entry.terminal {
        let mut v = vec!["kitty".to_string(), "-e".to_string()];
        v.append(&mut argv);
        argv = v;
    }
    argv
}

fn launch_app(entry: &Entry) {
    let argv = launch_argv(entry);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    util::spawn_detached(&refs);
}

// ---------------- windows ----------------

fn window_entries() -> Vec<Entry> {
    if util::dry() {
        return vec![
            Entry {
                kind: "win",
                label: "kitty — ~".into(),
                detail: "ws 1".into(),
                icon: "".into(),
                addr: "0xfake1".into(),
                ..Default::default()
            },
            Entry {
                kind: "win",
                label: "Brave — GitHub".into(),
                detail: "ws 2".into(),
                icon: "󰖟".into(),
                addr: "0xfake2".into(),
                ..Default::default()
            },
        ];
    }
    let mut out = Vec::new();
    for c in hypr::clients() {
        let ws = c
            .pointer("/workspace/name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        if ws == "special:hidden" {
            continue; // hidden windows have their own restore flow
        }
        let title = c
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| c.get("class").and_then(Value::as_str))
            .unwrap_or("?");
        let class = c.get("class").and_then(Value::as_str).unwrap_or("?");
        out.push(Entry {
            kind: "win",
            label: title.chars().take(60).collect(),
            detail: format!("{class} · ws {ws}"),
            icon: "󰖯".into(),
            addr: c.get("address").and_then(Value::as_str).unwrap_or("").into(),
            ..Default::default()
        });
    }
    out.sort_by(|a, b| a.detail.cmp(&b.detail));
    out
}

// ---------------- wallpaper ----------------

/// [(monitor_or_empty, full_path), …] from hyprpaper.conf
fn current_wallpapers() -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let Ok(text) = std::fs::read_to_string(util::home().join(".config/hypr/hyprpaper.conf"))
    else {
        return pairs;
    };
    let mut mon: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("monitor") {
            mon = Some(line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("").to_string());
        } else if line.starts_with("path") {
            if let Some(m) = mon.take() {
                pairs.push((
                    m,
                    line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("").to_string(),
                ));
            }
        }
    }
    pairs
}

/// [{name, x, y, focused}, …]
fn monitor_info() -> Vec<(String, i64, i64, bool)> {
    if util::dry() {
        return vec![
            ("DP-1".into(), 0, 0, true),
            ("HDMI-A-2".into(), 1600, 0, false),
            ("HDMI-A-1".into(), 0, 900, false),
        ];
    }
    hypr::monitors()
        .into_iter()
        .filter_map(|m| {
            Some((
                m.get("name")?.as_str()?.to_string(),
                m.get("x").and_then(Value::as_i64).unwrap_or(0),
                m.get("y").and_then(Value::as_i64).unwrap_or(0),
                m.get("focused").and_then(Value::as_bool).unwrap_or(false),
            ))
        })
        .collect()
}

fn wallpaper_root() -> PathBuf {
    for cand in [
        util::home().join("Pictures/wallpaper"),
        util::home().join("Pictures/Wallpapers"),
        util::home().join("Pictures"),
        util::home(),
    ] {
        if cand.is_dir() {
            return cand;
        }
    }
    util::home()
}

/// [.., folders…, images…] as tiles.
fn folder_contents(d: &Path) -> Vec<Entry> {
    let mut objs = Vec::new();
    if d != Path::new("/") && d.parent().is_some() {
        objs.push(Entry {
            kind: "up",
            label: "..".into(),
            path: d.parent().unwrap().to_string_lossy().into_owned(),
            ..Default::default()
        });
    }
    let mut items: Vec<PathBuf> = std::fs::read_dir(d)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    items.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
    for p in &items {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            objs.push(Entry {
                kind: "dir",
                label: name,
                path: p.to_string_lossy().into_owned(),
                ..Default::default()
            });
        }
    }
    for p in &items {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let lower = name.to_lowercase();
        if IMG_EXTS.iter().any(|e| lower.ends_with(e)) && !p.is_dir() {
            objs.push(Entry {
                kind: "img",
                label: name,
                path: p.to_string_lossy().into_owned(),
                ..Default::default()
            });
        }
    }
    objs.truncate(MAX_TILES);
    objs
}

// ---------------- tools menu ----------------

fn tool_entries() -> Vec<Entry> {
    let tools = util::local_bin("hypr-tools.sh").to_string_lossy().into_owned();
    let wallpaper_sh = util::local_bin("wallpaper.sh").to_string_lossy().into_owned();
    let bar_toggle = util::local_bin("bar-toggle.sh").to_string_lossy().into_owned();
    let fw = if !util::dry() && crate::applets::fw_status::ufw_enabled_in_conf() { "ON" } else { "OFF" };
    let ah = if !util::dry() && !proc::pids_with_cmdline("waybar-autohide.sh").is_empty() {
        "ON"
    } else {
        "OFF"
    };
    let fw_sub = format!("ufw is {fw}");
    let ah_sub = format!("now {ah} · toggles");
    let t: Vec<(&str, &str, String, Vec<String>)> = vec![
        ("󰀻", "App launcher", "ALT+R".into(), vec![tools.clone(), "apps".into()]),
        ("󰖯", "Window switcher", "ALT+W".into(), vec![tools.clone(), "windows".into()]),
        ("󰉋", "File manager", "ALT+E".into(), vec!["thunar".into()]),
        ("󰸉", "Pick wallpaper", "ALT+SHIFT+W random".into(), vec![tools.clone(), "wallpaper".into()]),
        ("󰒝", "Random wallpaper", "recolors everything".into(), vec![wallpaper_sh]),
        ("", "Settings panel", "ALT+X".into(), vec![tools.clone(), "settings".into()]),
        ("⏻", "Power", "ALT+ESC".into(), vec![tools.clone(), "power".into()]),
        ("󰅶", "New reminder", "ALT+T".into(), vec![tools.clone(), "remind".into()]),
        ("󰃰", "List reminders", "view / cancel".into(), vec![tools.clone(), "reminders".into()]),
        ("󰅐", "Clock & timezone", "click the clock too".into(), vec![tools.clone(), "clock".into()]),
        ("󰕥", "Firewall", fw_sub, vec![tools.clone(), "firewall".into()]),
        ("󰃤", "Antivirus scan", "ClamAV".into(), vec![tools.clone(), "clamav".into()]),
        ("󰌌", "Keybinds", "ALT+K · view / edit".into(), vec![tools.clone(), "keys".into()]),
        ("󰘸", "Stash workspace", "ALT+A · hide all".into(), vec![tools.clone(), "stash".into()]),
        ("󰖰", "Hide focused window", "ALT+SHIFT+A".into(), vec![tools.clone(), "hide-window".into()]),
        ("󰗐", "Unhide window…", "ALT+CTRL+A".into(), vec![tools.clone(), "unhide-window".into()]),
        ("󰜬", "Reorder bar buttons", "pick & place".into(), vec![tools.clone(), "reorder".into()]),
        ("󰊠", "Hide / show bar", "ALT+B".into(), vec![bar_toggle]),
        ("󰗕", "Bar auto-hide", ah_sub, vec![tools.clone(), "autohide".into()]),
        ("󰑓", "Restart bar", "waybar + auto-hide".into(), vec![tools.clone(), "restart-bar".into()]),
        ("", "Edit Hyprland config", "opens nvim + reload".into(), vec![tools.clone(), "edit-hypr".into()]),
        ("", "Edit bar config", "opens nvim + restart".into(), vec![tools.clone(), "edit-bar".into()]),
        ("󰑓", "Reload Hyprland", "apply config changes".into(), vec![tools, "reload".into()]),
    ];
    t.into_iter()
        .map(|(g, n, h, c)| Entry {
            kind: "tool",
            glyph: g.into(),
            label: n.into(),
            detail: h.clone(),
            sub: h,
            cmd: c,
            ..Default::default()
        })
        .collect()
}

// ---------------- filtering ----------------

/// q's chars appear in order in s ("tv" matches "television").
pub fn is_subsequence(q: &str, s: &str) -> bool {
    let mut it = s.chars();
    q.chars().all(|c| it.any(|sc| sc == c))
}

/// python rank(): 0 prefix · 1 word-prefix · 2 substring (or detail hit),
/// plus a new tier 3: fuzzy subsequence (so "ffx" still finds Firefox).
pub fn rank(label: &str, detail: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return Some(3);
    }
    let nl = label.to_lowercase();
    if nl.starts_with(q) {
        return Some(0);
    }
    if nl.split_whitespace().any(|w| w.starts_with(q)) {
        return Some(1);
    }
    if nl.contains(q) {
        return Some(2);
    }
    if detail.to_lowercase().contains(q) {
        return Some(2);
    }
    if q.len() >= 2 && is_subsequence(q, &nl) {
        return Some(3);
    }
    None
}

// ---------------- the TUI ----------------

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Apps,
    Windows,
    Wallpaper,
    Menu,
}

struct Group {
    title: Option<String>,
    entries: Vec<Entry>,
    expanded: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum HitKind {
    Tile(usize),
    Header(usize),
    BtnSet,
    BtnFolder,
    BtnCancel,
}

pub struct Launcher {
    pal: colors::Palette,
    tones: colors::Tones,
    mode: Mode,
    entries: Vec<Entry>,
    state: AppState,
    expanded: HashSet<String>,
    filter: String,
    groups: Vec<Group>,
    tiles: Vec<Entry>,           // flat, visible (expanded) tiles
    tile_imgs: Vec<Option<String>>,
    tile_group: Vec<usize>,      // flat tile -> group index (accent hue)
    hover: Option<usize>,
    sel: usize,
    cols: u16,
    scroll: u16,
    focus_section: Option<String>,
    // windows-mode list
    win_matches: Vec<Entry>,
    win_index: usize,
    // wallpaper state
    wp_target: Option<String>,
    wp_orig: Vec<(String, String)>,
    wp_previewed: bool,
    cwd: PathBuf,
    picking_monitor: bool,
    resolver: IconResolver,
    toast: Option<(String, Instant)>,
    hits: Vec<(Rect, HitKind)>,
    img_placements: Vec<(String, u16, u16, u16, u16)>,
    canvas: kitty_img::Canvas,
    images_on: bool,
    last_click: Option<(Instant, HitKind)>,
    pub should_quit: bool,
}

fn color(hex: &str) -> Color {
    let (r, g, b) = colors::hex_rgb(hex);
    Color::Rgb(r, g, b)
}

impl Launcher {
    pub fn new(mode: Mode) -> Self {
        let pal = colors::read_palette();
        let tones = pal.tones();
        let mut l = Launcher {
            pal,
            tones,
            mode,
            entries: Vec::new(),
            state: load_state(),
            expanded: [FAV_TITLE.to_string(), REC_TITLE.to_string()].into_iter().collect(),
            filter: String::new(),
            groups: Vec::new(),
            tiles: Vec::new(),
            tile_imgs: Vec::new(),
            tile_group: Vec::new(),
            hover: None,
            sel: 0,
            cols: 4,
            scroll: 0,
            focus_section: None,
            win_matches: Vec::new(),
            win_index: 0,
            wp_target: None,
            wp_orig: current_wallpapers(),
            wp_previewed: false,
            cwd: wallpaper_root(),
            picking_monitor: mode == Mode::Wallpaper,
            resolver: IconResolver::new(),
            toast: None,
            hits: Vec::new(),
            img_placements: Vec::new(),
            canvas: kitty_img::Canvas::new(),
            images_on: kitty_img::enabled(),
            last_click: None,
            should_quit: false,
        };
        match mode {
            Mode::Apps => {
                l.entries = desktop_entries();
                l.refilter();
            }
            Mode::Menu => {
                l.entries = tool_entries();
                l.refilter();
            }
            Mode::Windows => {
                l.entries = window_entries();
                l.refilter_list();
            }
            Mode::Wallpaper => l.show_monitors(),
        }
        l
    }

    fn notify(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
    }

    // ---------- grid building ----------

    fn set_groups(&mut self, groups: Vec<(Option<String>, Vec<Entry>)>) {
        // cap total tiles across all groups
        let mut budget = MAX_TILES;
        let mut capped: Vec<Group> = Vec::new();
        for (title, gobjs) in groups {
            let take: Vec<Entry> = gobjs.into_iter().take(budget).collect();
            budget -= take.len();
            if !take.is_empty() {
                let expanded =
                    title.is_none() || self.expanded.contains(title.as_deref().unwrap_or(""));
                capped.push(Group { title, entries: take, expanded });
            }
        }
        self.tiles = Vec::new();
        self.tile_group = Vec::new();
        for (gi, g) in capped.iter().enumerate() {
            if g.expanded {
                for e in &g.entries {
                    self.tiles.push(e.clone());
                    // untitled (flat) groups use the default accent
                    self.tile_group.push(if g.title.is_some() { gi } else { usize::MAX });
                }
            }
        }
        self.groups = capped;
        // resolve images (8 worker threads, like the python ThreadPoolExecutor)
        self.tile_imgs = self.resolve_images();
        self.resolver.flush();
        self.hover = None;
        self.sel = 0;
        self.scroll = 0;
        if let Some(want) = self.focus_section.take() {
            if let Some(i) = self.tiles.iter().position(|t| t.grp == want) {
                self.select(i, false);
            }
        }
        if self.mode == Mode::Wallpaper && !self.tiles.is_empty() {
            // python selected tile 0 with preview on folder view
            self.select(0, true);
        }
    }

    fn resolve_images(&mut self) -> Vec<Option<String>> {
        if !self.images_on {
            return vec![None; self.tiles.len()];
        }
        // resolve icons via the (cached) resolver on this thread; thumbnails
        // (independent magick calls) across worker threads
        let mut out: Vec<Option<String>> = vec![None; self.tiles.len()];
        let mut thumb_jobs: Vec<(usize, String)> = Vec::new();
        for (i, t) in self.tiles.iter().enumerate() {
            match t.kind {
                "app" => {
                    out[i] = self
                        .resolver
                        .resolve(&t.icon)
                        .or_else(|| self.resolver.default_app_icon());
                }
                "img" | "mon" => {
                    if !t.path.is_empty() {
                        thumb_jobs.push((i, t.path.clone()));
                    }
                }
                _ => {}
            }
        }
        let results: Vec<(usize, Option<String>)> = if thumb_jobs.len() <= 1 {
            thumb_jobs.iter().map(|(i, p)| (*i, make_thumb(p))).collect()
        } else {
            let chunk = thumb_jobs.len().div_ceil(8);
            std::thread::scope(|s| {
                let mut handles = Vec::new();
                for jobs in thumb_jobs.chunks(chunk) {
                    handles.push(s.spawn(move || {
                        jobs.iter().map(|(i, p)| (*i, make_thumb(p))).collect::<Vec<_>>()
                    }));
                }
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            })
        };
        for (i, r) in results {
            out[i] = r;
        }
        out
    }

    pub fn refilter(&mut self) {
        let q = self.filter.to_lowercase().trim().to_string();
        let mut ranked: Vec<(u8, String, usize)> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(r) = rank(&e.label, &e.detail, &q) {
                // apps: alphabetical; menu: keep the curated grouping order
                let key2 = if self.mode == Mode::Apps {
                    e.label.to_lowercase()
                } else {
                    format!("{i:08}")
                };
                ranked.push((r, key2, i));
            }
        }
        ranked.sort();
        let matches: Vec<Entry> =
            ranked.into_iter().map(|(_, _, i)| self.entries[i].clone()).collect();
        if self.mode == Mode::Apps && q.is_empty() {
            let fav_set: HashSet<&String> = self.state.favorites.iter().collect();
            let mut group_list: Vec<(Option<String>, Vec<Entry>)> = Vec::new();
            let mut favs: Vec<Entry> = Vec::new();
            let mut recs: Vec<Entry> = Vec::new();
            let mut with_flags: Vec<Entry> = Vec::new();
            for mut e in matches {
                e.fav = fav_set.contains(&e.detail);
                with_flags.push(e);
            }
            for e in &with_flags {
                if e.fav {
                    let mut e2 = e.clone();
                    e2.grp = FAV_TITLE.into();
                    favs.push(e2);
                }
            }
            if !favs.is_empty() {
                group_list.push((Some(FAV_TITLE.to_string()), favs));
            }
            for e in &with_flags {
                if !e.fav && self.state.recent.contains_key(&e.detail) {
                    recs.push(e.clone());
                }
            }
            recs.sort_by(|a, b| {
                let ra = self.state.recent.get(&a.detail).copied().unwrap_or(0.0);
                let rb = self.state.recent.get(&b.detail).copied().unwrap_or(0.0);
                rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
            });
            recs.truncate(8);
            for r in &mut recs {
                r.grp = REC_TITLE.into();
            }
            if !recs.is_empty() {
                group_list.push((Some(REC_TITLE.to_string()), recs));
            }
            let mut cats: Vec<(usize, &str, Vec<Entry>)> = Vec::new();
            for e in &with_flags {
                let (idx2, title) = category_of(&e.cats);
                let mut e2 = e.clone();
                e2.grp = title.to_string();
                match cats.iter_mut().find(|(i, _, _)| *i == idx2) {
                    Some((_, _, v)) => v.push(e2),
                    None => cats.push((idx2, title, vec![e2])),
                }
            }
            cats.sort_by_key(|(i, _, _)| *i);
            for (_, title, es) in cats {
                group_list.push((Some(title.to_string()), es));
            }
            self.set_groups(group_list);
        } else {
            self.set_groups(vec![(None, matches)]);
        }
    }

    fn refilter_list(&mut self) {
        let q = self.filter.to_lowercase().trim().to_string();
        self.win_matches = self
            .entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.label.to_lowercase().contains(&q)
                    || e.detail.to_lowercase().contains(&q)
            })
            .take(MAX_TILES)
            .cloned()
            .collect();
        self.win_index = 0;
    }

    // ---------- geometry ----------

    fn desired_cols(&self, width: u16) -> u16 {
        let desired: u16 = match self.mode {
            Mode::Apps => 10,
            Mode::Menu => 6,
            _ => 4,
        };
        let desired = match std::env::var("HYPR_LAUNCHER_COLS")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
        {
            Some(v) => v.clamp(2, 12),
            None => desired,
        };
        desired.min((width / 14).max(2)).max(2)
    }

    fn img_rows(&self) -> u16 {
        match self.mode {
            Mode::Apps => 3,
            Mode::Menu => 2,
            _ => 6,
        }
    }

    fn tile_height(&self) -> u16 {
        // border(2) + image/glyph + name + optional sub line
        let sub = match self.mode {
            Mode::Menu => 1,
            Mode::Wallpaper if self.picking_monitor => 1,
            _ => 0,
        };
        2 + self.img_rows() + 1 + sub
    }

    // ---------- selection / activation ----------

    fn select(&mut self, i: usize, preview: bool) {
        if self.tiles.is_empty() {
            return;
        }
        let i = i.min(self.tiles.len() - 1);
        self.sel = i;
        if preview && self.mode == Mode::Wallpaper && self.tiles[i].kind == "img" {
            let path = self.tiles[i].path.clone();
            self.preview(&path);
        }
    }

    fn preview(&mut self, path: &str) {
        self.wp_previewed = true;
        let targets: Vec<String> = match &self.wp_target {
            Some(t) => vec![t.clone()],
            None => {
                let ms: Vec<String> = self.wp_orig.iter().map(|(m, _)| m.clone()).collect();
                if ms.is_empty() {
                    vec![String::new()]
                } else {
                    ms
                }
            }
        };
        for m in targets {
            hypr::hyprpaper(&["wallpaper", &format!("{m},{path}")]);
        }
    }

    fn restore_original(&mut self) {
        if !self.wp_previewed {
            return;
        }
        for (m, p) in self.wp_orig.clone() {
            if !p.is_empty() {
                hypr::hyprpaper(&["wallpaper", &format!("{m},{p}")]);
            }
        }
    }

    fn activate(&mut self, i: usize) {
        if i >= self.tiles.len() {
            return;
        }
        let obj = self.tiles[i].clone();
        match (self.mode, obj.kind) {
            (Mode::Apps, _) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                self.state.recent.insert(obj.detail.clone(), now);
                if self.state.recent.len() > 20 {
                    // keep the 20 most recent
                    let mut by_time: Vec<(String, f64)> =
                        self.state.recent.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    by_time.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                    for (k, _) in by_time.into_iter().take(self.state.recent.len() - 20) {
                        self.state.recent.remove(&k);
                    }
                }
                save_state(&self.state);
                launch_app(&obj);
                self.should_quit = true;
            }
            (_, "tool") => {
                let refs: Vec<&str> = obj.cmd.iter().map(String::as_str).collect();
                util::spawn_detached(&refs);
                self.should_quit = true;
            }
            (_, "mon") => {
                self.wp_target =
                    if obj.target.is_empty() { None } else { Some(obj.target.clone()) };
                self.picking_monitor = false;
                self.show_folder();
            }
            (_, "dir") | (_, "up") => {
                self.cwd = PathBuf::from(&obj.path);
                self.show_folder();
            }
            (_, "img") => {
                let mut args: Vec<String> = vec![
                    util::local_bin("wallpaper.sh").to_string_lossy().into_owned(),
                    obj.path.clone(),
                ];
                if let Some(t) = &self.wp_target {
                    args.push(t.clone());
                }
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                util::spawn_detached(&refs);
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn toggle_favorite(&mut self, i: usize) {
        if self.mode != Mode::Apps || i >= self.tiles.len() {
            return;
        }
        let d = self.tiles[i].detail.clone();
        let label = self.tiles[i].label.clone();
        let note = if let Some(pos) = self.state.favorites.iter().position(|f| *f == d) {
            self.state.favorites.remove(pos);
            "removed from ★ Favorites"
        } else {
            self.state.favorites.push(d);
            "added to ★ Favorites"
        };
        save_state(&self.state);
        self.notify(format!("{label} {note}"));
        self.refilter();
    }

    fn toggle_section(&mut self, gi: usize) {
        let Some(title) = self.groups.get(gi).and_then(|g| g.title.clone()) else { return };
        if self.expanded.contains(&title) {
            self.expanded.remove(&title);
        } else {
            self.expanded.insert(title.clone());
            self.focus_section = Some(title);
        }
        self.refilter();
    }

    // ---------- wallpaper steps ----------

    fn show_monitors(&mut self) {
        self.picking_monitor = true;
        let cur: HashMap<String, String> = self.wp_orig.iter().cloned().collect();
        let mons = monitor_info();
        let xs: Vec<i64> = {
            let mut v: Vec<i64> = mons.iter().map(|m| m.1).collect();
            v.sort();
            v.dedup();
            v
        };
        let ys: Vec<i64> = {
            let mut v: Vec<i64> = mons.iter().map(|m| m.2).collect();
            v.sort();
            v.dedup();
            v
        };
        let fallback = cur
            .get("")
            .cloned()
            .or_else(|| self.wp_orig.first().map(|(_, p)| p.clone()))
            .unwrap_or_default();
        let mut objs = vec![Entry {
            kind: "mon",
            label: "All monitors".into(),
            target: "".into(),
            sub: "same image everywhere".into(),
            path: fallback,
            ..Default::default()
        }];
        for (name, x, y, focused) in mons {
            let mut pos: Vec<&str> = Vec::new();
            if ys.len() > 1 {
                pos.push(if y == ys[0] { "top" } else { "bottom" });
            }
            if xs.len() > 1 {
                pos.push(if x == xs[0] {
                    "left"
                } else if x == *xs.last().unwrap() {
                    "right"
                } else {
                    "middle"
                });
            }
            let mut sub =
                if pos.is_empty() { "only monitor".to_string() } else { pos.join(" ") };
            if focused {
                sub += " · ★ you are here";
            }
            let path = cur.get(&name).cloned().or_else(|| cur.get("").cloned()).unwrap_or_default();
            objs.push(Entry {
                kind: "mon",
                label: name.clone(),
                target: name,
                sub,
                path,
                ..Default::default()
            });
        }
        self.set_groups(vec![(None, objs)]);
    }

    fn show_folder(&mut self) {
        self.picking_monitor = false;
        let contents = folder_contents(&self.cwd.clone());
        self.set_groups(vec![(None, contents)]);
    }

    // ---------- input ----------

    pub fn on_key(&mut self, ev: KeyEvent) {
        match ev.code {
            KeyCode::Esc => {
                if self.mode == Mode::Wallpaper {
                    self.restore_original();
                }
                self.should_quit = true;
            }
            KeyCode::Enter => match self.mode {
                Mode::Windows => {
                    if let Some(e) = self.win_matches.get(self.win_index) {
                        hypr::dispatch(&format!("focuswindow address:{}", e.addr));
                        self.should_quit = true;
                    }
                }
                _ => self.activate(self.sel),
            },
            KeyCode::Down => self.move_sel(self.cols as i32),
            KeyCode::Up => self.move_sel(-(self.cols as i32)),
            KeyCode::Left => {
                if self.filter.is_empty() || self.mode == Mode::Wallpaper {
                    self.move_sel(-1);
                }
            }
            KeyCode::Right => {
                if self.filter.is_empty() || self.mode == Mode::Wallpaper {
                    self.move_sel(1);
                }
            }
            KeyCode::Backspace => {
                if self.mode != Mode::Wallpaper && !self.filter.is_empty() {
                    self.filter.pop();
                    self.apply_filter();
                }
            }
            KeyCode::Char(c) => {
                if self.mode != Mode::Wallpaper {
                    self.filter.push(c);
                    self.apply_filter();
                }
            }
            _ => {}
        }
    }

    fn apply_filter(&mut self) {
        match self.mode {
            Mode::Windows => self.refilter_list(),
            _ => self.refilter(),
        }
    }

    fn move_sel(&mut self, delta: i32) {
        match self.mode {
            Mode::Windows => {
                if self.win_matches.is_empty() {
                    return;
                }
                let step = if delta > 0 { 1 } else { -1 };
                let n = self.win_matches.len() as i32;
                self.win_index = (self.win_index as i32 + step).clamp(0, n - 1) as usize;
            }
            _ => {
                if self.tiles.is_empty() {
                    return;
                }
                let n = self.tiles.len() as i32;
                let i = (self.sel as i32 + delta).clamp(0, n - 1) as usize;
                self.select(i, true);
            }
        }
    }

    pub fn on_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            MouseEventKind::Moved => {
                let pos = Position { x: ev.column, y: ev.row };
                self.hover = self.hits.iter().find_map(|(r, h)| match h {
                    HitKind::Tile(i) if r.contains(pos) => Some(*i),
                    _ => None,
                });
            }
            MouseEventKind::ScrollDown => {
                self.scroll = self.scroll.saturating_add(3);
            }
            MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(3);
            }
            MouseEventKind::Down(btn) => {
                let pos = Position { x: ev.column, y: ev.row };
                let hit = self.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
                let Some(hit) = hit else { return };
                match (hit, btn) {
                    (HitKind::Tile(i), MouseButton::Right) => self.toggle_favorite(i),
                    (HitKind::Tile(i), MouseButton::Left) => {
                        let dbl = self
                            .last_click
                            .map(|(t, h)| {
                                h == HitKind::Tile(i) && t.elapsed() < Duration::from_millis(400)
                            })
                            .unwrap_or(false);
                        self.last_click = Some((Instant::now(), hit));
                        if dbl {
                            self.select(i, false);
                            self.activate(i);
                        } else {
                            self.select(i, true);
                        }
                    }
                    (HitKind::Header(gi), MouseButton::Left) => self.toggle_section(gi),
                    (HitKind::BtnSet, MouseButton::Left) => self.activate(self.sel),
                    (HitKind::BtnFolder, MouseButton::Left) => {
                        let mut args: Vec<String> = vec![
                            util::local_bin("wallpaper.sh").to_string_lossy().into_owned(),
                            self.cwd.to_string_lossy().into_owned(),
                        ];
                        if let Some(t) = &self.wp_target {
                            args.push(t.clone());
                        }
                        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                        util::spawn_detached(&refs);
                        self.should_quit = true;
                    }
                    (HitKind::BtnCancel, MouseButton::Left) => {
                        self.restore_original();
                        self.should_quit = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // ---------- drawing ----------

    pub fn draw(&mut self, f: &mut Frame) {
        self.hits.clear();
        self.img_placements.clear();
        let pal = self.pal.clone();
        let area = f.area();

        let (icon, title) = match self.mode {
            Mode::Apps => ("󰀻", "Apps — double-click or Enter to open"),
            Mode::Windows => ("󰖯", "Windows — click or type to filter"),
            Mode::Wallpaper => ("󰸉", "Wallpaper"),
            Mode::Menu => ("󰍜", "Tools — double-click or Enter to run"),
        };
        let mut y = area.y + 1;
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{icon}  {title}"),
                Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
            ))),
            Rect { x: area.x + 2, y, width: area.width.saturating_sub(4), height: 1 },
        );
        y += 1;

        match self.mode {
            Mode::Windows => self.draw_windows(f, area, y),
            Mode::Wallpaper => self.draw_grid_mode(f, area, y, true),
            _ => self.draw_grid_mode(f, area, y, false),
        }

        // footer
        let footer = Line::from(vec![
            Span::styled(" esc ", Style::default().fg(color(&pal.accent))),
            Span::styled("Cancel  ", Style::default().fg(color(&pal.subtext))),
            Span::styled("enter ", Style::default().fg(color(&pal.accent))),
            Span::styled("Open", Style::default().fg(color(&pal.subtext))),
        ]);
        f.render_widget(
            Paragraph::new(footer),
            Rect { x: area.x, y: area.bottom().saturating_sub(1), width: area.width, height: 1 },
        );

        // toast
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() > Duration::from_secs(4) {
                self.toast = None;
            } else {
                let w = (msg.chars().count() as u16 + 4).min(area.width.saturating_sub(2));
                let rect = Rect {
                    x: area.width.saturating_sub(w + 1),
                    y: area.bottom().saturating_sub(4),
                    width: w,
                    height: 3,
                };
                f.render_widget(Clear, rect);
                f.render_widget(
                    Paragraph::new(msg.as_str()).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(color(&pal.accent))),
                    ),
                    rect,
                );
            }
        }

        // one placement sync per frame (tiles + preview pane together)
        if self.images_on {
            let placements = std::mem::take(&mut self.img_placements);
            self.canvas.sync(&placements);
        }
    }

    fn draw_filter(&mut self, f: &mut Frame, area: Rect, y: u16, placeholder: &str) -> u16 {
        let pal = self.pal.clone();
        let rect = Rect {
            x: area.x + 2,
            y,
            width: area.width.saturating_sub(4),
            height: 3,
        };
        let (text, style) = if self.filter.is_empty() {
            (placeholder.to_string(), Style::default().fg(color(&pal.subtext)))
        } else {
            (self.filter.clone(), Style::default().fg(color(&pal.text)))
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {text}"), style)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color(&pal.accent))),
            ),
            rect,
        );
        y + 3
    }

    fn draw_windows(&mut self, f: &mut Frame, area: Rect, mut y: u16) {
        let pal = self.pal.clone();
        y = self.draw_filter(f, area, y, "filter…  (↑↓ move · Enter focus · Esc close)");
        y += 1;
        let list_bottom = area.bottom().saturating_sub(1);
        let visible = (list_bottom.saturating_sub(y)) as usize;
        let start = self.win_index.saturating_sub(visible.saturating_sub(1));
        for (row, e) in self.win_matches.iter().enumerate().skip(start).take(visible) {
            let selected = row == self.win_index;
            let rect =
                Rect { x: area.x + 2, y, width: area.width.saturating_sub(4), height: 1 };
            let (bar, style) = if selected {
                (
                    Span::styled("▌", Style::default().fg(color(&pal.accent))),
                    Style::default().fg(color(&pal.text)).bg(color(&pal.surface)),
                )
            } else {
                (Span::raw(" "), Style::default().fg(color(&pal.text)))
            };
            let label = format!("{}  {}", e.icon, e.label);
            let detail = &e.detail;
            let pad = (rect.width as usize)
                .saturating_sub(label.chars().count() + detail.chars().count() + 3);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    bar,
                    Span::styled(format!(" {label}"), style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(detail.clone(), Style::default().fg(color(&pal.subtext))),
                ])),
                rect,
            );
            y += 1;
        }
    }

    fn draw_grid_mode(&mut self, f: &mut Frame, area: Rect, mut y: u16, wallpaper: bool) {
        let pal = self.pal.clone();
        if wallpaper {
            // wp-where hint
            let hint = if self.picking_monitor {
                "Which monitor? Each box shows what that monitor wears right now.".to_string()
            } else {
                let where_ = self.wp_target.clone().unwrap_or_else(|| "all monitors".into());
                let pretty = util::tilde(&self.cwd.to_string_lossy());
                format!("{where_} · {pretty} — select previews live · Enter keeps it · Esc restores")
            };
            f.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(color(&pal.subtext)))),
                Rect { x: area.x + 3, y, width: area.width.saturating_sub(5), height: 1 },
            );
            y += 1;
        } else {
            let ph = match self.mode {
                Mode::Apps => {
                    "filter…  (Enter open · right-click a box = ★ favorite · Esc close)"
                }
                _ => "filter…  (Enter open · Esc close)",
            };
            y = self.draw_filter(f, area, y, ph);
        }
        let grid_top = y + 1;
        let actions_h = if wallpaper && !self.picking_monitor { 3 } else { 0 };
        let grid_bottom = area.bottom().saturating_sub(1 + actions_h);
        if grid_bottom <= grid_top {
            return;
        }
        let full = Rect {
            x: area.x + 2,
            y: grid_top,
            width: area.width.saturating_sub(3),
            height: grid_bottom - grid_top,
        };
        // wallpaper folder view: grid on the left, live preview pane right
        let show_pane = wallpaper && !self.picking_monitor && full.width >= 64;
        let (grid_area, pane_area) = if show_pane {
            let pane_w = (full.width * 2 / 5).clamp(24, 46);
            (
                Rect { width: full.width - pane_w - 1, ..full },
                Some(Rect { x: full.right() - pane_w, width: pane_w, ..full }),
            )
        } else {
            (full, None)
        };
        self.cols = self.desired_cols(grid_area.width);
        self.draw_tiles(f, grid_area);
        if let Some(pane) = pane_area {
            self.draw_preview_pane(f, pane);
        }

        if actions_h > 0 {
            let by = grid_bottom;
            let mut x = area.x + 2;
            for (label, kind) in [
                ("󰸉 Set selected", HitKind::BtnSet),
                ("󰒝 Slideshow from this folder", HitKind::BtnFolder),
                ("󰕌 Restore & close", HitKind::BtnCancel),
            ] {
                let w = label.chars().count() as u16 + 4;
                if x + w > area.right() {
                    break;
                }
                let rect = Rect { x, y: by, width: w, height: 3 };
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!(" {label} "),
                        Style::default().fg(color(&pal.text)),
                    ))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(color(&pal.surface))),
                    ),
                    rect,
                );
                self.hits.push((rect, kind));
                x += w + 1;
            }
        }
    }

    /// Right-side live preview of the selected wallpaper (bigger thumb +
    /// file facts). The real desktop previews simultaneously via hyprpaper.
    fn draw_preview_pane(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let tones = self.tones.clone();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(&tones.border_hi)))
            .title(Span::styled(
                " Preview ",
                Style::default().fg(color(&pal.accent)).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let Some(obj) = self.tiles.get(self.sel).cloned() else { return };
        let img_h = inner.height.saturating_sub(3).max(1);
        let img = self.tile_imgs.get(self.sel).cloned().flatten();
        if obj.kind == "img" {
            if let (Some(path), true) = (img, self.images_on) {
                self.img_placements.push((
                    path,
                    inner.x + 1,
                    inner.y,
                    inner.width.saturating_sub(2),
                    img_h,
                ));
            } else {
                f.render_widget(
                    Paragraph::new(Span::styled("󰋩", Style::default().fg(color(&pal.accent))))
                        .alignment(ratatui::layout::Alignment::Center),
                    Rect { x: inner.x, y: inner.y + img_h / 2, width: inner.width, height: 1 },
                );
            }
            let size_kb = std::fs::metadata(&obj.path).map(|m| m.len() / 1024).unwrap_or(0);
            let name: String = obj.label.chars().take(inner.width as usize - 2).collect();
            f.render_widget(
                Paragraph::new(vec![
                    Line::styled(name, Style::default().fg(color(&pal.text))),
                    Line::styled(
                        format!("{size_kb} KB · Enter keeps it · Esc restores"),
                        Style::default().fg(color(&pal.subtext)),
                    ),
                ]),
                Rect {
                    x: inner.x + 1,
                    y: inner.bottom().saturating_sub(2),
                    width: inner.width.saturating_sub(2),
                    height: 2,
                },
            );
        } else {
            let hint = match obj.kind {
                "dir" => "a folder — Enter opens it",
                "up" => "go up one folder",
                _ => "",
            };
            f.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        format!("{}  {}", glyph_for(obj.kind), obj.label),
                        Style::default().fg(color(&pal.text)),
                    ),
                    Line::styled(hint, Style::default().fg(color(&pal.subtext))),
                ]),
                Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: 2 },
            );
        }
    }

    /// Grid with section headers, tile-row scrolling and image placement.
    fn draw_tiles(&mut self, f: &mut Frame, area: Rect) {
        let pal = self.pal.clone();
        let cols = self.cols as usize;
        let tile_h = self.tile_height();
        let tile_w = area.width / self.cols;

        // virtual rows: (kind, height): header(group idx) | tiles(range start)
        enum VRow {
            Header(usize),
            Tiles(usize, usize), // group idx, row-within-group
        }
        let mut vrows: Vec<(VRow, u16)> = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            if g.title.is_some() {
                vrows.push((VRow::Header(gi), 2));
            }
            if g.expanded {
                let nrows = g.entries.len().div_ceil(cols);
                for r in 0..nrows {
                    vrows.push((VRow::Tiles(gi, r), tile_h));
                }
            }
        }
        let total_h: u16 = vrows.iter().map(|(_, h)| *h).sum();
        let max_scroll = total_h.saturating_sub(area.height);
        // keep the selected tile's row scrolled into view
        {
            let mut flat_start = 0usize;
            let mut vy = 0u16;
            for (row, h) in &vrows {
                match row {
                    VRow::Header(_) => vy += h,
                    VRow::Tiles(gi, r) => {
                        let g = &self.groups[*gi];
                        let start_in_group = r * cols;
                        let count = cols.min(g.entries.len() - start_in_group);
                        if self.sel >= flat_start && self.sel < flat_start + count {
                            // scroll so this vrow is visible
                            if vy < self.scroll {
                                self.scroll = vy;
                            } else if vy + h > self.scroll + area.height {
                                self.scroll = (vy + h).saturating_sub(area.height);
                            }
                        }
                        flat_start += count;
                        vy += h;
                    }
                }
            }
        }
        self.scroll = self.scroll.min(max_scroll);

        // render visible vrows
        let mut vy: i32 = -(self.scroll as i32);
        let mut flat_start = 0usize;
        for (row, h) in &vrows {
            let h = *h;
            let screen_y = area.y as i32 + vy;
            match row {
                VRow::Header(gi) => {
                    let g = &self.groups[*gi];
                    if screen_y + 1 >= area.y as i32 && screen_y + 1 < area.bottom() as i32 {
                        let title = g.title.clone().unwrap_or_default();
                        let arrow = if g.expanded { "▾" } else { "▸" };
                        let action = if g.expanded { "hide" } else { "show" };
                        // each category wears its own hue from the palette ring
                        let hue = pal.category_accent(*gi);
                        let rect = Rect {
                            x: area.x + 1,
                            y: (screen_y + 1) as u16,
                            width: area.width.saturating_sub(2),
                            height: 1,
                        };
                        f.render_widget(
                            Paragraph::new(Line::from(vec![
                                Span::styled(
                                    format!("{arrow} {title}  "),
                                    Style::default()
                                        .fg(color(&hue))
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    format!("{} apps — click to {action}", g.entries.len()),
                                    // accent_soft: muted accent for secondary emphasis
                                    Style::default().fg(color(&self.tones.accent_soft)),
                                ),
                            ])),
                            rect,
                        );
                        self.hits.push((rect, HitKind::Header(*gi)));
                    }
                }
                VRow::Tiles(gi, r) => {
                    let g = &self.groups[*gi];
                    let start_in_group = r * cols;
                    let count = cols.min(g.entries.len() - start_in_group);
                    if screen_y + (h as i32) > area.y as i32 && screen_y < area.bottom() as i32 {
                        let fully_visible = screen_y >= area.y as i32
                            && screen_y + (h as i32) <= area.bottom() as i32;
                        for c in 0..count {
                            let flat_i = flat_start + c;
                            let x = area.x + (c as u16) * tile_w;
                            if screen_y >= area.y as i32 && screen_y + (h as i32) <= area.bottom() as i32 {
                                let rect = Rect { x, y: screen_y as u16, width: tile_w, height: h };
                                self.draw_tile(f, rect, flat_i, fully_visible);
                            }
                        }
                    }
                    flat_start += count;
                }
            }
            vy += h as i32;
        }

    }

    fn draw_tile(&mut self, f: &mut Frame, rect: Rect, i: usize, fully_visible: bool) {
        let pal = self.pal.clone();
        let tones = self.tones.clone();
        let Some(obj) = self.tiles.get(i) else { return };
        let obj = obj.clone();
        let selected = i == self.sel;
        let hovered = self.hover == Some(i) && !selected;
        // tile hue: its category's accent (flat groups → default accent)
        let group_hue = match self.tile_group.get(i) {
            Some(&gi) if gi != usize::MAX => pal.category_accent(gi),
            _ => pal.accent.clone(),
        };
        let border_style = if selected {
            Style::default().fg(color(&pal.accent))
        } else if hovered {
            Style::default().fg(color(&tones.border_hi))
        } else {
            Style::default().fg(Color::Reset).add_modifier(Modifier::DIM)
        };
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style);
        // tonal fill: stronger for selected, whisper for hovered
        if selected {
            block = block.style(Style::default().bg(color(&tones.select_bg)));
        } else if hovered {
            block = block.style(Style::default().bg(color(&tones.hover_bg)));
        }
        let inner = block.inner(rect);
        if selected || hovered {
            f.render_widget(block, rect);
        }
        self.hits.push((rect, HitKind::Tile(i)));

        let img_rows = self.img_rows();
        let img = self.tile_imgs.get(i).cloned().flatten();
        if let (Some(path), true, true) = (img, self.images_on, fully_visible) {
            // reserve cells; kitty draws on top
            let iw = inner.width.saturating_sub(2).max(1);
            self.img_placements.push((
                path,
                inner.x + (inner.width.saturating_sub(iw)) / 2,
                inner.y,
                iw,
                img_rows,
            ));
        } else {
            let glyph = if obj.glyph.is_empty() { glyph_for(obj.kind).to_string() } else { obj.glyph.clone() };
            let gy = inner.y + img_rows / 2;
            f.render_widget(
                Paragraph::new(Span::styled(
                    glyph,
                    Style::default().fg(color(&group_hue)),
                ))
                .alignment(ratatui::layout::Alignment::Center),
                Rect { x: inner.x, y: gy.min(inner.bottom().saturating_sub(1)), width: inner.width, height: 1 },
            );
        }
        let star = if obj.fav { "★ " } else { "" };
        let name: String = format!("{star}{}", obj.label).chars().take(36).collect();
        f.render_widget(
            Paragraph::new(Span::styled(name, Style::default().fg(color(&pal.text))))
                .alignment(ratatui::layout::Alignment::Center),
            Rect {
                x: inner.x,
                y: inner.y + img_rows,
                width: inner.width,
                height: 1,
            },
        );
        if !obj.sub.is_empty() && inner.height > img_rows + 1 {
            let sub: String = obj.sub.chars().take(36).collect();
            f.render_widget(
                Paragraph::new(Span::styled(sub, Style::default().fg(color(&pal.subtext))))
                    .alignment(ratatui::layout::Alignment::Center),
                Rect {
                    x: inner.x,
                    y: inner.y + img_rows + 1,
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }
}

// ---------- entry point ----------

pub fn run(args: &[&str]) -> ExitCode {
    let mode = match args.first().copied().unwrap_or("apps") {
        "windows" => Mode::Windows,
        "wallpaper" => Mode::Wallpaper,
        "menu" => Mode::Menu,
        _ => Mode::Apps,
    };
    let mut app = Launcher::new(mode);

    let bench = std::env::var("HYPR_BENCH_STARTUP").map(|v| v == "1").unwrap_or(false);

    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hypr-launcher: cannot init terminal: {e}");
            return util::fail_exit();
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);

    loop {
        let _ = terminal.draw(|f| app.draw(f));
        if bench {
            break;
        }
        match crossterm::event::poll(Duration::from_millis(120)) {
            Ok(true) => match crossterm::event::read() {
                Ok(Event::Key(k)) if k.kind != crossterm::event::KeyEventKind::Release => {
                    app.on_key(k)
                }
                Ok(Event::Mouse(m)) => app.on_mouse(m),
                Ok(Event::Resize(_, _)) => {}
                Ok(_) => {}
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
        if app.should_quit {
            break;
        }
    }
    app.canvas.clear();
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    util::ok_exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_matches_python_tiers() {
        assert_eq!(rank("Firefox", "firefox", ""), Some(3));
        assert_eq!(rank("Firefox", "firefox", "fire"), Some(0));
        assert_eq!(rank("GNU Image Editor", "gimp", "image"), Some(1));
        assert_eq!(rank("LibreOffice Writer", "writer", "offi"), Some(2)); // substring only — no word starts with it
        assert_eq!(rank("Files", "org.gnome.Nautilus", "nautilus"), Some(2));
        assert_eq!(rank("Weather", "weather", "xyz"), None);
        assert_eq!(rank("VLC media player", "vlc", "med"), Some(1));
        assert_eq!(rank("VLC media player", "vlc", "dia"), Some(2)); // substring, not word start
    }

    #[test]
    fn fuzzy_tier_catches_subsequences_only_as_last_resort() {
        // "ffx" is not a prefix/word/substring of firefox — fuzzy tier 3
        assert_eq!(rank("Firefox", "firefox", "ffx"), Some(3));
        // exact tiers still win over fuzzy
        assert_eq!(rank("Firefox", "firefox", "fox"), Some(2));
        // single char never fuzzy-matches (too noisy)
        assert_eq!(rank("Weather", "weather", "z"), None);
        // chars out of order don't match
        assert_eq!(rank("Firefox", "firefox", "xf"), None);
        assert!(is_subsequence("tv", "television"));
        assert!(!is_subsequence("vt", "television"));
    }

    #[test]
    fn tile_groups_track_category_headers() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut l = Launcher::new(Mode::Apps);
        l.entries = vec![
            Entry {
                kind: "app",
                label: "Browser".into(),
                detail: "browser".into(),
                cats: "Network;".into(),
                exec: "true".into(),
                ..Default::default()
            },
            Entry {
                kind: "app",
                label: "Game".into(),
                detail: "game".into(),
                cats: "Game;".into(),
                exec: "true".into(),
                ..Default::default()
            },
        ];
        l.expanded.insert("󰖟 Internet".into());
        l.expanded.insert("󰊗 Games".into());
        l.filter.clear();
        l.refilter();
        assert_eq!(l.tiles.len(), 2);
        assert_eq!(l.tile_group.len(), 2);
        // two different groups → two different accent hues
        let h0 = l.pal.category_accent(l.tile_group[0]);
        let h1 = l.pal.category_accent(l.tile_group[1]);
        assert_ne!(l.tile_group[0], l.tile_group[1]);
        assert_ne!(h0, h1);
        // filtered mode collapses to one flat (default-accent) group
        l.filter = "browser".into();
        l.refilter();
        assert_eq!(l.tile_group, vec![usize::MAX]);
    }

    #[test]
    fn draw_smoke_wallpaper_preview_pane() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut l = Launcher::new(Mode::Wallpaper);
        // move into folder view with a fake selection
        l.picking_monitor = false;
        l.tiles = vec![Entry {
            kind: "img",
            label: "sunset.png".into(),
            path: "/nonexistent/sunset.png".into(),
            ..Default::default()
        }];
        l.tile_group = vec![usize::MAX];
        l.tile_imgs = vec![None];
        l.sel = 0;
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| l.draw(f)).unwrap();
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Preview"), "preview pane rendered");
        assert!(text.contains("sunset.png"));
    }

    #[test]
    fn desktop_entry_parser_first_key_wins_and_sections() {
        let text = "\
[Desktop Entry]
Name=Alpha
Name=Beta
Type=Application
Exec=alpha %U
[Desktop Action new]
Name=New Window
Exec=alpha --new
";
        let e = parse_desktop_entry(text);
        assert_eq!(e["Name"], "Alpha"); // setdefault: first wins
        assert_eq!(e["Exec"], "alpha %U"); // action section ignored
    }

    #[test]
    fn category_mapping_first_hit_wins() {
        assert_eq!(category_of("Network;WebBrowser;").1, "󰖟 Internet");
        assert_eq!(category_of("Security;Network;").1, "󰕥 Security"); // Security checked first
        assert_eq!(category_of("Game;").1, "󰊗 Games");
        assert_eq!(category_of("SomethingElse;").1, "󰘔 Other");
        assert_eq!(category_of("X-Kali-tools;").1, "󰕥 Security"); // substring match like python
    }

    #[test]
    fn shlex_and_field_code_stripping() {
        let e = Entry {
            kind: "app",
            exec: r#"env FOO="bar baz" myapp %U --flag"#.into(),
            terminal: false,
            ..Default::default()
        };
        assert_eq!(launch_argv(&e), vec!["env", "FOO=bar baz", "myapp", "--flag"]);
        let t = Entry {
            kind: "app",
            exec: "htop".into(),
            terminal: true,
            ..Default::default()
        };
        assert_eq!(launch_argv(&t), vec!["kitty", "-e", "htop"]);
    }

    #[test]
    fn recent_trims_to_20() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut l = Launcher::new(Mode::Menu);
        l.mode = Mode::Apps;
        l.tiles = (0..25)
            .map(|i| Entry {
                kind: "app",
                label: format!("App{i}"),
                detail: format!("app{i}"),
                exec: "true".into(),
                ..Default::default()
            })
            .collect();
        for i in 0..25 {
            l.state.recent.insert(format!("app{i}"), i as f64);
        }
        l.activate(0); // touches app0, then trims
        assert_eq!(l.state.recent.len(), 20);
        // app0 was just refreshed to "now" — must survive the trim
        assert!(l.state.recent.contains_key("app0"));
        // the oldest (app1..app5) fell out
        assert!(!l.state.recent.contains_key("app1"));
    }

    #[test]
    fn dry_run_menu_mode_has_23_tools() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let l = Launcher::new(Mode::Menu);
        assert_eq!(l.entries.len(), 23);
        assert_eq!(l.tiles.len(), 23);
        assert!(l.tiles.iter().any(|t| t.label == "Firewall" && t.sub == "ufw is OFF"));
    }

    #[test]
    fn dry_run_windows_mode_uses_fixtures() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let l = Launcher::new(Mode::Windows);
        assert_eq!(l.win_matches.len(), 2);
        assert_eq!(l.win_matches[0].detail, "ws 1");
    }

    #[test]
    fn dry_run_wallpaper_monitor_tiles() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let l = Launcher::new(Mode::Wallpaper);
        // All monitors + 3 fake monitors
        assert_eq!(l.tiles.len(), 4);
        assert_eq!(l.tiles[0].label, "All monitors");
        let dp1 = l.tiles.iter().find(|t| t.label == "DP-1").unwrap();
        assert!(dp1.sub.contains("top"), "sub: {}", dp1.sub);
        assert!(dp1.sub.contains("left"));
        assert!(dp1.sub.contains("★ you are here"));
        let hdmi2 = l.tiles.iter().find(|t| t.label == "HDMI-A-2").unwrap();
        assert!(hdmi2.sub.contains("right"));
        let hdmi1 = l.tiles.iter().find(|t| t.label == "HDMI-A-1").unwrap();
        assert!(hdmi1.sub.contains("bottom"));
    }

    #[test]
    fn wallpaper_preview_and_restore_record_hyprpaper_calls() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        std::env::set_var("HYPR_ACTIONS_FILE", "");
        let mut l = Launcher::new(Mode::Wallpaper);
        l.wp_orig = vec![("DP-1".into(), "/old.png".into())];
        l.wp_target = Some("DP-1".into());
        l.preview("/new.png");
        assert!(l.wp_previewed);
        l.restore_original();
        let acts = util::recorded_actions();
        let flat: Vec<String> = acts.iter().map(|a| a.join(" ")).collect();
        assert!(flat.iter().any(|a| a.contains("hyprpaper wallpaper DP-1,/new.png")), "{flat:?}");
        assert!(flat.iter().any(|a| a.contains("hyprpaper wallpaper DP-1,/old.png")), "{flat:?}");
    }

    #[test]
    fn filter_groups_collapse_to_flat_list() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        let mut l = Launcher::new(Mode::Menu);
        l.filter = "wall".into();
        l.refilter();
        assert!(l.tiles.iter().all(|t| t.label.to_lowercase().contains("wall")
            || t.detail.to_lowercase().contains("wall")));
        assert!(l.tiles.len() >= 2); // Pick wallpaper + Random wallpaper
        assert_eq!(l.groups.len(), 1);
        assert!(l.groups[0].title.is_none());
    }

    #[test]
    fn draw_smoke_all_modes() {
        std::env::set_var("HYPRSETTINGS_DRYRUN", "1");
        for mode in [Mode::Menu, Mode::Windows, Mode::Wallpaper] {
            let mut l = Launcher::new(mode);
            let backend = ratatui::backend::TestBackend::new(120, 40);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| l.draw(f)).unwrap();
        }
    }

    #[test]
    fn folder_contents_orders_up_dirs_images() {
        let base = std::env::temp_dir().join(format!("hl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("zdir")).unwrap();
        std::fs::create_dir_all(base.join("adir")).unwrap();
        std::fs::write(base.join("b.png"), "x").unwrap();
        std::fs::write(base.join("a.jpg"), "x").unwrap();
        std::fs::write(base.join(".hidden.png"), "x").unwrap();
        std::fs::write(base.join("notes.txt"), "x").unwrap();
        let objs = folder_contents(&base);
        let kinds: Vec<&str> = objs.iter().map(|o| o.kind).collect();
        let labels: Vec<&str> = objs.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(kinds, vec!["up", "dir", "dir", "img", "img"]);
        assert_eq!(labels, vec!["..", "adir", "zdir", "a.jpg", "b.png"]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
