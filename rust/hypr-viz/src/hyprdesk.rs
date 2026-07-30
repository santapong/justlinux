//! The fleet contracts, in Rust — parsing the SAME files lib/hyprdesk
//! parses. The files are the interface (docs/rust-migration.md): these two
//! implementations cannot drift apart without a visible symptom.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

fn widgets_conf() -> PathBuf {
    home().join(".config/conky/widgets.conf")
}

/// widgets.conf as key/value pairs — flat `key = value`, keys `\w+`,
/// values `\S+`, exactly theme.py's grammar. Uncached on purpose: every
/// read picks up edits, which is what makes SIGUSR1 reposition work.
pub fn conf_get(key: &str, default: &str) -> String {
    if let Ok(text) = fs::read_to_string(widgets_conf()) {
        for line in text.lines() {
            let mut parts = line.splitn(2, '=');
            let k = parts.next().unwrap_or("").trim();
            if k == key && k.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if let Some(v) = parts.next() {
                    let v = v.trim().split_whitespace().next().unwrap_or("");
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
    }
    default.to_string()
}

/// One-key conf write honouring the confwrite discipline: flock on the
/// sidecar lock, previous content one-deep in .undo, temp + rename so no
/// reader ever sees a half-written file. Order and comments preserved.
pub fn conf_set(key: &str, value: &str) {
    use fs2::FileExt;
    let conf = widgets_conf();
    let lock_path = conf.with_file_name("widgets.conf.lock");
    let Ok(lock) = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
    else {
        return;
    };
    let _ = lock.lock_exclusive(); // best effort, like the python OSError path
    let old = fs::read_to_string(&conf).unwrap_or_default();
    let _ = fs::write(conf.with_file_name("widgets.conf.undo"), &old);
    let mut out = String::new();
    let mut seen = false;
    for line in old.lines() {
        let k = line.splitn(2, '=').next().unwrap_or("").trim();
        if k == key {
            out.push_str(&format!("{key} = {value}\n"));
            seen = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !seen {
        out.push_str(&format!("{key} = {value}\n"));
    }
    let tmp = conf.with_file_name("widgets.conf.tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        if f.write_all(out.as_bytes()).is_ok() {
            let _ = fs::rename(&tmp, &conf);
        }
    }
    let _ = fs2::FileExt::unlock(&lock);
}

#[derive(Clone, Copy, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn parse(s: &str) -> Option<Rgb> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 {
            return None;
        }
        Some(Rgb(
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ))
    }
    fn lum(self) -> f64 {
        let f = |c: u8| {
            let x = c as f64 / 255.0;
            if x <= 0.03928 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(self.0) + 0.7152 * f(self.1) + 0.0722 * f(self.2)
    }
    fn ratio(a: Rgb, b: Rgb) -> f64 {
        let (la, lb) = (a.lum(), b.lum());
        let (hi, lo) = (la.max(lb), la.min(lb));
        (hi + 0.05) / (lo + 0.05)
    }
    fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
        let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
        Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgb,
    pub accent: Rgb,
    pub accent2: Rgb,
}

/// theme.py's colors(), reduced to the roles viz consumes — including the
/// accent2 contrast fix, because on real wallpapers accent2 collapses into
/// fg and the gradient's far end would vanish.
pub fn colors() -> Palette {
    let mut bg = Rgb::parse("#101017").unwrap();
    let mut fg = Rgb::parse("#FEFAD7").unwrap();
    let mut accent = Rgb::parse("#F2E3EF").unwrap();
    let mut accent2 = Rgb::parse("#FCF18E").unwrap();

    let theme = conf_get("theme", "wallust");
    let mut src = home().join(".config/conky/colors.lua");
    if theme != "wallust" {
        let themed = home().join(format!(".config/conky/themes/{theme}.lua"));
        if themed.is_file() {
            src = themed;
        }
    }
    if let Ok(text) = fs::read_to_string(&src) {
        // name = "#RRGGBB" pairs, same regex-shaped scan as theme.py
        for line in text.lines() {
            let Some((name, rest)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let Some(hexpos) = rest.find('#') else { continue };
            let hex: String = rest[hexpos..].chars().take(7).collect();
            if let Some(c) = Rgb::parse(&hex) {
                match name {
                    "bg" => bg = c,
                    "fg" => fg = c,
                    "accent" => accent = c,
                    "accent2" => accent2 = c,
                    _ => {}
                }
            }
        }
    }
    // A TIER THAT IS NOT DISTINCT IS NOT A TIER (theme.py, verbatim rule):
    // accent2 rides the wallpaper and can land indistinguishable from fg.
    if Rgb::ratio(accent2, fg) < 1.6 {
        accent2 = accent;
    }
    let _ = Rgb::mix; // sub derivation lives here when a port needs it
    Palette { bg, accent, accent2 }
}
