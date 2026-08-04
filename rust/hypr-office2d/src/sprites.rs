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

// ---------------- the design-handoff sprite sheet ----------------
// (docs/design-brief-terminals.md + ~/Pictures/design-brief/handoff)
// Every fill is a palette role; `State` is the parameterized slot the
// worker body and the desk monitor screen wear. 4 px per sprite pixel.

/// Which role a rect wears. State resolves at blit time.
#[derive(Clone, Copy)]
pub enum R {
    Sub,
    Fg,
    Bg,
    Muted,
    State,
}

pub type SpriteRect = (f32, f32, f32, f32, R);

/// worker 12×15 — antenna sub, head fg, eye holes bg, body+arms STATE,
/// legs sub. Frame B swaps the arm rows for the typing bob.
pub const WORKER: &[SpriteRect] = &[
    (5.0, 0.0, 2.0, 1.0, R::Sub),
    (3.0, 1.0, 6.0, 5.0, R::Fg),
    (4.0, 3.0, 1.0, 1.0, R::Bg),
    (7.0, 3.0, 1.0, 1.0, R::Bg),
    (2.0, 6.0, 8.0, 5.0, R::State),
    (1.0, 7.0, 1.0, 3.0, R::State),
    (10.0, 7.0, 1.0, 3.0, R::State),
    (3.0, 11.0, 2.0, 4.0, R::Sub),
    (7.0, 11.0, 2.0, 4.0, R::Sub),
];
/// arms one pixel up — the 2-frame typing bob (160 ms alternation on a
/// tiny region: motion, not a luminance flash; WCAG area threshold holds)
pub const WORKER_B: &[SpriteRect] = &[
    (5.0, 0.0, 2.0, 1.0, R::Sub),
    (3.0, 1.0, 6.0, 5.0, R::Fg),
    (4.0, 3.0, 1.0, 1.0, R::Bg),
    (7.0, 3.0, 1.0, 1.0, R::Bg),
    (2.0, 6.0, 8.0, 5.0, R::State),
    (1.0, 6.0, 1.0, 3.0, R::State),
    (10.0, 6.0, 1.0, 3.0, R::State),
    (3.0, 11.0, 2.0, 4.0, R::Sub),
    (7.0, 11.0, 2.0, 4.0, R::Sub),
];
/// asleep: head drops one pixel, antenna folds
pub const WORKER_ASLEEP: &[SpriteRect] = &[
    (3.0, 2.0, 6.0, 5.0, R::Fg),
    (2.0, 7.0, 8.0, 4.0, R::State),
    (1.0, 8.0, 1.0, 2.0, R::State),
    (10.0, 8.0, 1.0, 2.0, R::State),
    (3.0, 11.0, 2.0, 4.0, R::Sub),
    (7.0, 11.0, 2.0, 4.0, R::Sub),
];
pub const SUBAGENT: &[SpriteRect] = &[
    (2.0, 0.0, 4.0, 4.0, R::Fg),
    (3.0, 2.0, 1.0, 1.0, R::Bg),
    (5.0, 2.0, 1.0, 1.0, R::Bg),
    (1.0, 4.0, 6.0, 4.0, R::State),
    (2.0, 8.0, 1.0, 2.0, R::Sub),
    (5.0, 8.0, 1.0, 2.0, R::Sub),
];
/// desk 22×13 — frame/stand/top/legs muted, the SCREEN is the state light
pub const DESK2: &[SpriteRect] = &[
    (6.0, 0.0, 10.0, 4.0, R::Muted),
    (7.0, 1.0, 8.0, 2.0, R::State),
    (10.0, 4.0, 2.0, 2.0, R::Muted),
    (0.0, 6.0, 22.0, 2.0, R::Muted),
    (1.0, 8.0, 2.0, 5.0, R::Muted),
    (19.0, 8.0, 2.0, 5.0, R::Muted),
];
pub const CHAIR: &[SpriteRect] = &[
    (0.0, 0.0, 8.0, 4.0, R::Muted),
    (3.0, 4.0, 2.0, 4.0, R::Muted),
    (1.0, 8.0, 6.0, 2.0, R::Muted),
];
pub const PLANT: &[SpriteRect] = &[
    (1.0, 0.0, 6.0, 3.0, R::Sub),
    (0.0, 2.0, 8.0, 2.0, R::Sub),
    (2.0, 4.0, 4.0, 2.0, R::Sub),
    (3.0, 6.0, 2.0, 1.0, R::Sub),
    (2.0, 7.0, 4.0, 5.0, R::Muted),
];
pub const DOOR: &[SpriteRect] = &[
    (0.0, 0.0, 12.0, 22.0, R::Muted),
    (2.0, 2.0, 8.0, 20.0, R::Bg),
    (8.0, 11.0, 1.0, 2.0, R::Sub),
];

/// Blit a role sprite at 4 px per pixel (or any scale), resolving R::State
/// to `state` — the one slot that changes with what the session is doing.
pub fn blit_role(
    pix: &mut Pixmap,
    art: &[SpriteRect],
    x: f32,
    y: f32,
    scale: f32,
    pal: &Palette,
    state: Rgb,
    alpha: u8,
) {
    let mut paint = tiny_skia::Paint::default();
    for &(rx, ry, rw, rh, role) in art {
        let Rgb(r, g, b) = match role {
            R::Sub => pal.sub,
            R::Fg => pal.fg,
            R::Bg => pal.bg,
            R::Muted => pal.muted,
            R::State => state,
        };
        paint.set_color(tiny_skia::Color::from_rgba8(r, g, b, alpha));
        if let Some(rect) =
            tiny_skia::Rect::from_xywh(x + rx * scale, y + ry * scale, rw * scale, rh * scale)
        {
            pix.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
        }
    }
}
