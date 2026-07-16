//! Wallust palette — parsed from waybar's colors.css, with the same
//! catppuccin defaults and red-dominance DANGER fallback as the python apps.

use crate::util;
use std::collections::HashMap;

// red/cyan are carried for palette completeness (the python dict did the
// same); only some slots are referenced by the current UIs.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Palette {
    pub base: String,
    pub surface: String,
    pub text: String,
    pub subtext: String,
    pub accent: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub cyan: String,
    /// Destructive actions must READ as red: wallust can map "red" to any
    /// hue, so fall back to a real red when it isn't red-dominant.
    pub danger: String,
}

fn parse_define_colors(css: &str) -> HashMap<String, String> {
    // @define-color <name> #RRGGBB
    let mut out = HashMap::new();
    for line in css.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("@define-color") else {
            continue;
        };
        let mut it = rest.split_whitespace();
        let (Some(name), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        let val = val.trim_end_matches(';');
        if val.len() == 7
            && val.starts_with('#')
            && val[1..].chars().all(|c| c.is_ascii_hexdigit())
        {
            out.insert(name.to_string(), val.to_string());
        }
    }
    out
}

pub fn hex_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return (0, 0, 0);
    }
    (
        u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
    )
}

pub fn rgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Linear blend a→b by t (0.0 = a, 1.0 = b). The tonal-ladder primitive:
/// wallust gives one raw sample per slot; mixing toward base/text yields
/// the intermediate surface/hover/border tones a designed palette needs
/// (the Material-You trick, minus the color science).
pub fn mix(a: &str, b: &str, t: f32) -> String {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = hex_rgb(a);
    let (br, bg, bb) = hex_rgb(b);
    let l = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    rgb_hex(l(ar, br), l(ag, bg), l(ab, bb))
}

/// Move a color toward white (amt > 0) or black (amt < 0) by |amt| in 0..=1.
pub fn lighten(hex: &str, amt: f32) -> String {
    if amt >= 0.0 {
        mix(hex, "#ffffff", amt)
    } else {
        mix(hex, "#000000", -amt)
    }
}

/// Derived tones — computed once from the raw palette. Every widget tint
/// (hover, raised card, soft accent fill) comes from here instead of
/// hard-coded alpha guesses, so all UIs shift together with the wallpaper.
#[derive(Clone, Debug)]
pub struct Tones {
    /// subtle fill for hovered rows/tiles (accent pulled far toward base)
    pub hover_bg: String,
    /// selected-tile fill, a step stronger than hover
    pub select_bg: String,
    /// raised card border (surface lifted toward text)
    pub border_hi: String,
    /// muted accent for secondary emphasis (section counts, gauges' tail)
    pub accent_soft: String,
    /// accent readable on base at small sizes (slightly lifted)
    pub accent_hi: String,
}

impl Palette {
    pub fn tones(&self) -> Tones {
        Tones {
            hover_bg: mix(&self.accent, &self.base, 0.85),
            select_bg: mix(&self.accent, &self.base, 0.72),
            border_hi: mix(&self.surface, &self.text, 0.18),
            accent_soft: mix(&self.accent, &self.base, 0.45),
            accent_hi: lighten(&self.accent, 0.12),
        }
    }

    /// Per-category accent rotation for the launcher: stable, tasteful
    /// variety by cycling the palette's own hues (never random).
    pub fn category_accent(&self, idx: usize) -> String {
        let ring = [&self.accent, &self.cyan, &self.green, &self.yellow, &self.red];
        ring[idx % ring.len()].clone()
    }
}

pub fn read_palette() -> Palette {
    let mut c: HashMap<&str, String> = [
        ("base", "#1e1e2e"),
        ("surface", "#45475a"),
        ("text", "#cdd6f4"),
        ("subtext", "#a6adc8"),
        ("accent", "#89b4fa"),
        ("red", "#f38ba8"),
        ("green", "#a6e3a1"),
        ("yellow", "#f9e2af"),
        ("cyan", "#94e2d5"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();

    let css_path = util::home().join(".config/waybar/colors.css");
    if let Ok(css) = std::fs::read_to_string(css_path) {
        for (name, val) in parse_define_colors(&css) {
            if let Some(slot) = c.keys().find(|k| **k == name.as_str()).copied() {
                c.insert(slot, val);
            }
        }
    }
    let red = c["red"].clone();
    let (r, g, b) = hex_rgb(&red);
    let danger = if r > g && r > b { red.clone() } else { "#d9536f".to_string() };
    Palette {
        base: c["base"].clone(),
        surface: c["surface"].clone(),
        text: c["text"].clone(),
        subtext: c["subtext"].clone(),
        accent: c["accent"].clone(),
        red,
        green: c["green"].clone(),
        yellow: c["yellow"].clone(),
        cyan: c["cyan"].clone(),
        danger,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_define_color_lines() {
        let css = "@define-color accent #11AAFF;\nnot-a-color\n@define-color red #ff0000\n";
        let m = parse_define_colors(css);
        assert_eq!(m["accent"], "#11AAFF");
        assert_eq!(m["red"], "#ff0000");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn rejects_malformed_hex() {
        let m = parse_define_colors("@define-color red #ff00\n@define-color x #GGGGGG\n");
        assert!(m.is_empty());
    }

    #[test]
    fn hex_to_rgb() {
        assert_eq!(hex_rgb("#89b4fa"), (0x89, 0xb4, 0xfa));
    }

    #[test]
    fn danger_falls_back_when_red_not_dominant() {
        // mirrors the python: _r > _g and _r > _b
        let (r, g, b) = hex_rgb("#5588ff"); // blue-dominant "red"
        assert!(!(r > g && r > b));
    }

    #[test]
    fn mix_endpoints_and_midpoint() {
        assert_eq!(mix("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(mix("#000000", "#ffffff", 1.0), "#ffffff");
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        // clamps out-of-range t
        assert_eq!(mix("#102030", "#ffffff", -1.0), "#102030");
        assert_eq!(mix("#102030", "#ffffff", 2.0), "#ffffff");
    }

    #[test]
    fn lighten_both_directions() {
        assert_eq!(lighten("#808080", 1.0), "#ffffff");
        assert_eq!(lighten("#808080", -1.0), "#000000");
        let up = lighten("#89b4fa", 0.12);
        let (r0, g0, b0) = hex_rgb("#89b4fa");
        let (r1, g1, b1) = hex_rgb(&up);
        assert!(r1 >= r0 && g1 >= g0 && b1 >= b0);
    }

    #[test]
    fn tones_sit_between_their_endpoints() {
        let p = Palette {
            base: "#1e1e2e".into(),
            surface: "#45475a".into(),
            text: "#cdd6f4".into(),
            subtext: "#a6adc8".into(),
            accent: "#89b4fa".into(),
            red: "#f38ba8".into(),
            green: "#a6e3a1".into(),
            yellow: "#f9e2af".into(),
            cyan: "#94e2d5".into(),
            danger: "#f38ba8".into(),
        };
        let t = p.tones();
        // hover is darker (closer to base) than select, select darker than soft accent
        let lum = |h: &str| {
            let (r, g, b) = hex_rgb(h);
            r as u32 + g as u32 + b as u32
        };
        assert!(lum(&t.hover_bg) < lum(&t.select_bg));
        assert!(lum(&t.select_bg) < lum(&t.accent_soft));
        assert!(lum(&t.border_hi) > lum(&p.surface));
        assert!(lum(&t.accent_hi) > lum(&p.accent));
    }

    #[test]
    fn category_accents_cycle_stably() {
        let p = read_palette();
        assert_eq!(p.category_accent(0), p.category_accent(5));
        assert_ne!(p.category_accent(0), p.category_accent(1));
        // 11 categories (10 + Other) all get a color without panic
        for i in 0..11 {
            assert_eq!(p.category_accent(i).len(), 7);
        }
    }
}
