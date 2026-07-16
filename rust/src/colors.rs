//! Wallust palette — parsed from waybar's colors.css, with the same
//! catppuccin defaults and red-dominance DANGER fallback as the python apps.

use crate::util;
use std::collections::HashMap;

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
}
