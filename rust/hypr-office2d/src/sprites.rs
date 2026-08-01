//! The office pixel art, byte-for-byte from bin/hypr-claude-office —
//! Clawd is the fleet's orange starburst and must look identical here.

use hyprdesk::{Palette, Rgb};
use tiny_skia::Pixmap;

pub const CLAWD_A: &[&str] = &[
    "...O..O..",
    "O..OOOO..",
    ".OOOOOOO.",
    "OOEOOOEOO",
    ".OOOOOOO.",
    "O..OOOO..",
    "...O..O..",
];
pub const CLAWD_B: &[&str] = &[
    "..O..O...",
    "..OOOO..O",
    ".OOOOOOO.",
    "OOEOOOEOO",
    ".OOOOOOO.",
    "..OOOO..O",
    "..O..O...",
];
pub const CLAWD_SLEEP: &[&str] = &[
    ".........",
    "..OOOOO..",
    ".OODOODO.",
    "..OOOOO..",
    ".........",
];
pub const DESK: &[&str] = &[
    "....SSSSSSS.....",
    "....SSSSSSS.....",
    "....SSSSSSS.....",
    "....KKKKKKK..M..",
    "DDDDDDDDDDDDDDDD",
    ".L............L.",
    ".L............L.",
];

/// Clawd's own inks are constants (he is the mascot, not a theme element);
/// desk furniture takes the palette. `screen` lets a working desk glow.
pub fn ink(ch: char, pal: &Palette, screen: Rgb, clawd_art: bool) -> Option<Rgb> {
    Some(match ch {
        'O' => Rgb(0xD9, 0x77, 0x57),
        'E' => Rgb(0x1d, 0x1d, 0x1b),
        'D' if clawd_art => Rgb(0x8f, 0x48, 0x30), // sleeping Clawd's shade
        'D' | 'L' => pal.muted,
        'K' => pal.sub,
        'M' => pal.accent,
        'S' => screen,
        _ => return None,
    })
}

pub fn blit(
    pix: &mut Pixmap,
    art: &[&str],
    x: f32,
    y: f32,
    scale: f32,
    pal: &Palette,
    screen: Rgb,
    clawd_art: bool,
    alpha: u8,
) {
    let mut paint = tiny_skia::Paint::default();
    for (ry, row) in art.iter().enumerate() {
        for (rx, ch) in row.chars().enumerate() {
            let Some(Rgb(r, g, b)) = ink(ch, pal, screen, clawd_art) else {
                continue;
            };
            paint.set_color(tiny_skia::Color::from_rgba8(r, g, b, alpha));
            if let Some(rect) = tiny_skia::Rect::from_xywh(
                x + rx as f32 * scale,
                y + ry as f32 * scale,
                scale,
                scale,
            ) {
                pix.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
            }
        }
    }
}
