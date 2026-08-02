//! .desktop app catalogue + PNG icon cache, ported from bin/hypr-appdock.
//! Icons come pre-resolved by hypr-launcher (~/.cache/hypr-launcher/
//! resolve.json) — on this fleet they are all PNG, which tiny-skia
//! decodes natively. An unloadable icon degrades to a 2-letter label.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tiny_skia::{Pixmap, PixmapPaint, Transform};

#[derive(Clone)]
pub struct AppEntry {
    pub stem: String,
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub terminal: bool,
}

fn app_dirs() -> Vec<PathBuf> {
    let home = hyprdesk::home();
    vec![
        home.join(".local/share/applications"),
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        home.join(".local/share/flatpak/exports/share/applications"),
    ]
}

/// {stem: entry} for every visible .desktop app, first dir wins.
pub fn all_apps() -> HashMap<String, AppEntry> {
    let mut apps = HashMap::new();
    for dir in app_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        paths.sort();
        for p in paths {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if apps.contains_key(&stem) {
                continue;
            }
            if let Some(entry) = parse_desktop(&p) {
                apps.insert(stem, entry);
            }
        }
    }
    apps
}

fn parse_desktop(path: &Path) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut e = AppEntry {
        stem: path.file_stem()?.to_string_lossy().to_string(),
        name: String::new(),
        exec: String::new(),
        icon: String::new(),
        terminal: false,
    };
    for line in text.lines() {
        if line.starts_with('[') && !line.contains("[Desktop Entry]") {
            break;
        }
        let low: String = line.trim().to_lowercase().replace(' ', "");
        if low == "nodisplay=true" || low == "hidden=true" {
            return None;
        }
        for (key, field) in [
            ("Name=", &mut e.name as *mut String),
            ("Exec=", &mut e.exec as *mut String),
            ("Icon=", &mut e.icon as *mut String),
        ] {
            if let Some(rest) = line.strip_prefix(key) {
                let f = unsafe { &mut *field };
                if f.is_empty() {
                    *f = rest.trim().to_string();
                }
            }
        }
        if line.starts_with("Terminal=true") {
            e.terminal = true;
        }
    }
    if e.exec.is_empty() {
        None
    } else {
        Some(e)
    }
}

pub fn icon_path(icon_name: &str) -> Option<String> {
    if icon_name.is_empty() {
        return None;
    }
    if icon_name.starts_with('/') && Path::new(icon_name).is_file() {
        return Some(icon_name.to_string());
    }
    let resolve = hyprdesk::home().join(".cache/hypr-launcher/resolve.json");
    let map: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(resolve).ok()?).ok()?;
    let p = map.get(icon_name)?.as_str()?;
    if Path::new(p).is_file() {
        Some(p.to_string())
    } else {
        None
    }
}

/// Decode + scale an icon to size×size; never crash on a broken file.
pub fn load_icon(path: &str, size: u32) -> Option<Pixmap> {
    let src = Pixmap::decode_png(&std::fs::read(path).ok()?).ok()?;
    if src.width() == 0 || src.height() == 0 {
        return None;
    }
    let mut out = Pixmap::new(size, size)?;
    let s = size as f32 / src.width().max(src.height()) as f32;
    let dx = (size as f32 - src.width() as f32 * s) / 2.0;
    let dy = (size as f32 - src.height() as f32 * s) / 2.0;
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    out.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        Transform::from_scale(s, s).post_translate(dx, dy),
        None,
    );
    Some(out)
}

fn split_argv(cmd: &str) -> Vec<String> {
    // shlex-lite: quotes + backslash escapes, enough for Exec= lines
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut esc = false;
    for ch in cmd.chars() {
        if esc {
            cur.push(ch);
            esc = false;
        } else if ch == '\\' && quote != Some('\'') {
            esc = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn launch(entry: &AppEntry) {
    let argv: Vec<String> = split_argv(&entry.exec)
        .into_iter()
        .filter(|a| !a.starts_with('%') && !a.starts_with("@@"))
        .collect();
    if argv.is_empty() {
        return;
    }
    let mut full: Vec<String> = if entry.terminal {
        vec!["kitty".into(), "-e".into()]
    } else {
        vec![]
    };
    full.extend(argv);
    let _ = std::process::Command::new("setsid")
        .args(&full)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
