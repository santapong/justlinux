//! theme.py's colors(), full palette — including the PINNED status inks
//! (status colours never ride the wallpaper) and both contrast fixes.

use std::fs;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Rgb> {
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
    pub fn ratio(a: Rgb, b: Rgb) -> f64 {
        let (la, lb) = (a.lum(), b.lum());
        let (hi, lo) = (la.max(lb), la.min(lb));
        (hi + 0.05) / (lo + 0.05)
    }
    pub fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
        let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
        Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgb,
    pub fg: Rgb,
    pub accent: Rgb,
    pub accent2: Rgb,
    pub sub: Rgb,
    pub muted: Rgb,
    pub good: Rgb,
    pub bad: Rgb,
    pub warn: Rgb,
}

pub fn colors() -> Palette {
    let mut bg = Rgb::parse("#101017").unwrap();
    let mut fg = Rgb::parse("#FEFAD7").unwrap();
    let mut accent = Rgb::parse("#F2E3EF").unwrap();
    let mut accent2 = Rgb::parse("#FCF18E").unwrap();
    let mut muted = Rgb::parse("#3D3B3A").unwrap();
    // sub comes FROM the file when wallust provides it (it does on this
    // box); the mix() below is only the fallback. Deriving unconditionally
    // read #6D6C60 where python read #A5A391 — caught by the parity test.
    let mut sub: Option<Rgb> = None;
    // status inks are pinned but THEME-OVERRIDABLE (ink table): a theme
    // file may redefine them, the wallpaper never does
    let mut good = Rgb::parse("#8EC07C").unwrap();
    let mut bad = Rgb::parse("#E06C75").unwrap();
    let mut warn = Rgb::parse("#E0B25C").unwrap();

    let theme = crate::conf_get("theme", "wallust");
    let mut src = crate::home().join(".config/conky/colors.lua");
    if theme != "wallust" {
        let themed = crate::home().join(format!(".config/conky/themes/{theme}.lua"));
        if themed.is_file() {
            src = themed;
        }
    }
    if let Ok(text) = fs::read_to_string(&src) {
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
                    "muted" => muted = c,
                    "sub" => sub = Some(c),
                    "good" => good = c,
                    "bad" => bad = c,
                    "warn" => warn = c,
                    _ => {}
                }
            }
        }
    }
    // A TIER THAT IS NOT DISTINCT IS NOT A TIER (theme.py): accent2 rides
    // the wallpaper and can land indistinguishable from fg.
    if Rgb::ratio(accent2, fg) < 1.6 {
        accent2 = accent;
    }
    // muted is a SURFACE tone; a wallpaper can hand one invisible on bg
    if Rgb::ratio(muted, bg) < 1.35 {
        muted = Rgb::mix(bg, fg, 0.22);
    }
    Palette {
        bg,
        fg,
        accent,
        accent2,
        // python's _mix(fg,bg,t) weights its FIRST argument: bg + (fg-bg)*t.
        // Rgb::mix weights the second, so the arguments swap here — the
        // parity test read #6D6C60 against python's #A5A391 until they did.
        sub: sub.unwrap_or_else(|| Rgb::mix(bg, fg, 0.62)),
        muted,
        good,
        bad,
        warn,
    }
}
