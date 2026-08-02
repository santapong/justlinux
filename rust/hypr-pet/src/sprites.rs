//! The creatures' pixel art, byte-for-byte from bin/hypr-pet. All frames
//! are drawn FACING LEFT — the blitter mirrors them when a creature moves
//! right, exactly like the python draw_frame.

use hyprdesk::Rgb;
use tiny_skia::Pixmap;

pub const CAT_WALK_A: &[&str] = &[
    ".B..B...........",
    ".BBBBB..........",
    ".BEBNB.......T..",
    ".BBBBB......TT..",
    "..BBBBBBBBBBT...",
    "..BBBBBBBBBB....",
    "..BBBBBBBBBB....",
    "..B..B...B..B...",
    "..B..B...B..B...",
];
pub const CAT_WALK_B: &[&str] = &[
    ".B..B...........",
    ".BBBBB..........",
    ".BEBNB......T...",
    ".BBBBB......T...",
    "..BBBBBBBBBBT...",
    "..BBBBBBBBBB....",
    "..BBBBBBBBBB....",
    "...B..B.B..B....",
    "...B..B.B..B....",
];
pub const CAT_SIT_A: &[&str] = &[
    ".B..B....",
    ".BBBBB...",
    ".BEBNB..T",
    ".BBBBB..T",
    "..BBBBBBT",
    "..BBBBBB.",
    "..BBBBBB.",
    "..BBBBBB.",
    "..BB.BB..",
];
// python derives SIT_B from SIT_A with two replaces (the tail flicks)
pub const CAT_SIT_B: &[&str] = &[
    ".B..B....",
    ".BBBBB...",
    ".BEBNB.T.",
    ".BBBBB.T.",
    "..BBBBBT.",
    "..BBBBBB.",
    "..BBBBBB.",
    "..BBBBBB.",
    "..BB.BB..",
];
pub const CAT_SLEEP: &[&str] = &[
    ".........",
    "..BBBBB..",
    ".BBBBBBB.",
    ".BDBBDBB.",
    ".BBBBBBB.",
    "..BBBBB..",
];

pub const CLAWD_WALK_A: &[&str] = &[
    "....O..O....",
    ".O..OOOO..O.",
    "..OOOOOOOO..",
    "...OOOOOO...",
    "OOOEOOOOEOOO",
    "...OOOOOO...",
    "..OOOOOOOO..",
    ".O..OOOO..O.",
    "....O..O....",
];
pub const CLAWD_WALK_B: &[&str] = &[
    "....O..O....",
    "....OOOO....",
    ".O.OOOOOO.O.",
    "..OOOOOOOO..",
    ".OOEOOOOEOO.",
    "..OOOOOOOO..",
    ".O.OOOOOO.O.",
    "....OOOO....",
    "....O..O....",
];
pub const CLAWD_SLEEP: &[&str] = &[
    "............",
    "....OOOO....",
    "..OOOOOOOO..",
    ".OODOOOODOO.",
    "..OOOOOOOO..",
    "....OOOO....",
];

/// Per-creature ink map, resolved once at startup (SIGUSR2 re-resolves).
pub type Ink = fn(char, &hyprdesk::Palette) -> Option<Rgb>;

pub fn cat_ink(ch: char, pal: &hyprdesk::Palette) -> Option<Rgb> {
    Some(match ch {
        'B' | 'T' => pal.fg,
        'D' => pal.muted,
        'E' => pal.accent2,
        'N' => pal.accent,
        _ => return None,
    })
}

pub fn clawd_ink(ch: char, _pal: &hyprdesk::Palette) -> Option<Rgb> {
    Some(match ch {
        'O' => Rgb(0xD9, 0x77, 0x57),
        'D' => Rgb(0x8f, 0x48, 0x30),
        'E' => Rgb(0x1d, 0x1d, 0x1b),
        _ => return None,
    })
}

pub fn blit(
    pix: &mut Pixmap,
    art: &[&str],
    ink: Ink,
    pal: &hyprdesk::Palette,
    x: f32,
    y: f32,
    scale: f32,
    facing_right: bool,
) {
    let w = art.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut paint = tiny_skia::Paint::default();
    for (ry, row) in art.iter().enumerate() {
        for (rx, ch) in row.chars().enumerate() {
            let Some(Rgb(r, g, b)) = ink(ch, pal) else {
                continue;
            };
            // art faces LEFT — mirror when the creature moves right
            let px = if facing_right { w - 1 - rx } else { rx };
            paint.set_color(tiny_skia::Color::from_rgba8(r, g, b, 255));
            if let Some(rc) = tiny_skia::Rect::from_xywh(
                x + px as f32 * scale,
                y + ry as f32 * scale,
                scale,
                scale,
            ) {
                pix.fill_rect(rc, &paint, tiny_skia::Transform::identity(), None);
            }
        }
    }
}
