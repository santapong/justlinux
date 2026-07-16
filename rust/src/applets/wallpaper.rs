//! wallpaper — set wallpaper + recolor the whole desktop. Port of wallpaper.sh.
//!
//!   wallpaper.sh                     random image, all monitors
//!   wallpaper.sh <image>             image on all monitors
//!   wallpaper.sh <image> <monitor>   image on one monitor (e.g. DP-1)
//!
//! Recolors: waybar / rofi / swaync / wlogout / hyprland borders.

use crate::{hypr, proc, util};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const IMG_EXTS: &[&str] = &["jpg", "jpeg", "png"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMG_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// find <dir> -type f (jpg|jpeg|png), recursive.
pub fn find_images(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() && is_image(&p) => out.push(p),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Existing (monitor → path) assignments + fallback path from hyprpaper.conf.
/// Mirrors the bash reader: pairs of `monitor =` / `path =` lines, a path
/// with no preceding monitor (or an empty monitor value) is the fallback.
pub fn read_assignments(conf_text: &str) -> (Vec<(String, String)>, String) {
    let mut assign: Vec<(String, String)> = Vec::new();
    let mut fallback = String::new();
    let mut cur_mon: Option<String> = None;
    for line in conf_text.lines() {
        let t = line.trim();
        let Some(eq) = t.find('=') else { continue };
        let (key, val) = (t[..eq].trim(), t[eq + 1..].trim());
        if key.starts_with("monitor") {
            cur_mon = Some(val.to_string());
        } else if key.starts_with("path") {
            match cur_mon.take() {
                Some(m) if !m.is_empty() => {
                    if let Some(slot) = assign.iter_mut().find(|(mm, _)| *mm == m) {
                        slot.1 = val.to_string();
                    } else {
                        assign.push((m, val.to_string()));
                    }
                }
                _ => fallback = val.to_string(),
            }
        }
    }
    (assign, fallback)
}

fn write_block(out: &mut String, monitor: &str, path: &str) {
    out.push_str("wallpaper {\n");
    out.push_str(&format!("    monitor  = {monitor}\n"));
    out.push_str(&format!("    path     = {path}\n"));
    out.push_str("    fit_mode = cover\n");
    if Path::new(path).is_dir() {
        out.push_str("    timeout   = 300\n");
        out.push_str("    order     = random\n");
        out.push_str("    recursive = true\n");
    }
    out.push_str("}\n");
}

/// hyprpaper 0.8+ block format, exactly as wallpaper.sh emitted it.
pub fn render_conf(fallback: &str, assign: &[(String, String)]) -> String {
    let mut out = String::from(
        "# Managed by wallpaper.sh — run 'wallpaper.sh <image|folder> [monitor]' to change.\n",
    );
    write_block(&mut out, "", fallback);
    for (m, p) in assign {
        write_block(&mut out, m, p);
    }
    out.push_str("splash = false\n");
    out.push_str("ipc = true\n");
    out
}

pub fn run(args: &[&str]) -> ExitCode {
    let walldir = util::home().join("Pictures/wallpaper");
    let paper_conf = util::home().join(".config/hypr/hyprpaper.conf");
    let mon = args.get(1).copied().unwrap_or("all");

    // pick the wallpaper (argument or random)
    let wall: PathBuf = match args.first() {
        Some(w) if !w.is_empty() => PathBuf::from(w),
        _ => {
            let imgs = find_images(&walldir);
            if imgs.is_empty() {
                eprintln!("No such image or folder: ");
                return util::fail_exit();
            }
            imgs[util::random_index(imgs.len())].clone()
        }
    };
    if !wall.exists() {
        println!("No such image or folder: {}", wall.display());
        return util::fail_exit();
    }
    let wall = std::fs::canonicalize(&wall).unwrap_or(wall);
    let wall_s = wall.to_string_lossy().into_owned();
    let slideshow = wall.is_dir();
    println!(
        "Wallpaper: {}  (target: {}{})",
        wall_s,
        mon,
        if slideshow { ", slideshow" } else { "" }
    );

    // read existing assignments, update, persist
    let text = std::fs::read_to_string(&paper_conf).unwrap_or_default();
    let (mut assign, mut fallback) = read_assignments(&text);
    if mon == "all" {
        fallback = wall_s.clone();
        assign.clear(); // "all" really means all
    } else {
        if let Some(slot) = assign.iter_mut().find(|(m, _)| m == mon) {
            slot.1 = wall_s.clone();
        } else {
            assign.push((mon.to_string(), wall_s.clone()));
        }
        if fallback.is_empty() {
            fallback = wall_s.clone();
        }
    }
    if let Some(parent) = paper_conf.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&paper_conf, render_conf(&fallback, &assign)).is_err() {
        eprintln!("cannot write {}", paper_conf.display());
        return util::fail_exit();
    }

    // apply live
    let recolor: PathBuf = if slideshow {
        // directories need a config reload — restart hyprpaper
        proc::pkill_comm("hyprpaper", libc::SIGTERM);
        std::thread::sleep(std::time::Duration::from_millis(500));
        hypr::dispatch("exec hyprpaper");
        let imgs = find_images(&wall);
        if imgs.is_empty() {
            eprintln!("No images inside {wall_s}");
            return util::fail_exit();
        }
        imgs[util::random_index(imgs.len())].clone()
    } else {
        if mon == "all" {
            hypr::hyprpaper(&["wallpaper", &format!(", {wall_s}")]);
            for m in hypr::monitors() {
                if let Some(name) = m.get("name").and_then(Value::as_str) {
                    hypr::hyprpaper(&["wallpaper", &format!("{name}, {wall_s}")]);
                }
            }
        } else {
            hypr::hyprpaper(&["wallpaper", &format!("{mon}, {wall_s}")]);
        }
        wall.clone()
    };

    // recolor the desktop
    let recolor_s = recolor.to_string_lossy();
    if util::dry() {
        util::record_action(&["wallust", "run", &recolor_s]);
    } else if !util::run_capture(&["wallust", "run", &recolor_s]).0 {
        eprintln!("wallust failed");
        return util::fail_exit();
    }
    hypr::reload(); // window borders
    proc::pkill_comm("waybar", libc::SIGUSR2); // waybar restyles in place
    if util::dry() {
        util::record_action(&["swaync-client", "-rs"]);
    } else {
        util::run_capture(&["swaync-client", "-rs"]); // ignore failure
    }

    println!("Desktop recolored ✔");
    util::ok_exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_and_block_assignments() {
        let conf = "\
# comment
monitor = DP-1
path = /a.png
wallpaper {
    monitor  =
    path     = /fallback.png
    fit_mode = cover
}
wallpaper {
    monitor  = HDMI-A-1
    path     = /b.png
}
";
        let (assign, fallback) = read_assignments(conf);
        assert_eq!(fallback, "/fallback.png");
        assert_eq!(
            assign,
            vec![
                ("DP-1".to_string(), "/a.png".to_string()),
                ("HDMI-A-1".to_string(), "/b.png".to_string())
            ]
        );
    }

    #[test]
    fn renders_block_format_byte_exact() {
        let out = render_conf("/w.png", &[("DP-1".into(), "/x.png".into())]);
        // NB: bash `printf '    monitor  = %s\n' ""` leaves a trailing space —
        // we reproduce that byte-exactly.
        let expected = "\
# Managed by wallpaper.sh — run 'wallpaper.sh <image|folder> [monitor]' to change.
wallpaper {
    monitor  = 
    path     = /w.png
    fit_mode = cover
}
wallpaper {
    monitor  = DP-1
    path     = /x.png
    fit_mode = cover
}
splash = false
ipc = true
";
        assert_eq!(out, expected);
    }

    #[test]
    fn roundtrip_written_conf_parses_back() {
        let rendered = render_conf("/fb.png", &[("DP-1".into(), "/m1.png".into())]);
        let (assign, fallback) = read_assignments(&rendered);
        assert_eq!(fallback, "/fb.png");
        assert_eq!(assign, vec![("DP-1".to_string(), "/m1.png".to_string())]);
    }

    #[test]
    fn slideshow_dir_gets_timeout_keys() {
        let tmp = std::env::temp_dir();
        let out = render_conf(&tmp.to_string_lossy(), &[]);
        assert!(out.contains("timeout   = 300"));
        assert!(out.contains("order     = random"));
        assert!(out.contains("recursive = true"));
    }

    #[test]
    fn image_extension_filter_is_case_insensitive() {
        assert!(is_image(Path::new("/x/a.PNG")));
        assert!(is_image(Path::new("/x/a.Jpeg")));
        assert!(!is_image(Path::new("/x/a.webp"))); // wallpaper.sh only took jpg/jpeg/png
        assert!(!is_image(Path::new("/x/noext")));
    }
}
