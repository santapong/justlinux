//! Label rendering — ab_glyph over the SAME face the rest of the desktop
//! uses. ab_glyph and not fontdue, from measurement rather than taste:
//! fontdue's Font::from_bytes eagerly builds outlines for every glyph,
//! which on a 12,138-glyph Nerd Font cost 49 MB of heap — the whole
//! widget measured python-sized because of its font loader. ab_glyph
//! parses lazily and keeps only the font bytes (~2 MB).
//!
//! Missing font degrades to no labels, never to a crash (the graceful-
//! fallback rule: a blank row is a bug, a dead widget is worse).

use std::cell::RefCell;
use std::collections::HashMap;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use crate::Rgb;
use tiny_skia::Pixmap;

/// One rasterized glyph: coverage bitmap + placement, colour-independent.
struct Raster {
    w: i32,
    h: i32,
    min_x: f32,
    min_y: f32,
    advance: f32,
    cov: Vec<u8>,
}

pub struct Text {
    font: Option<FontVec>,
    bold: Option<FontVec>,
    /// cairo-size → PxScale conversion, measured from the font itself:
    /// cairo's set_font_size is EM units, ab_glyph's PxScale is line
    /// height (ascent−descent). For this face height/em = 1320/1000 —
    /// a hardcoded 1.16 guess rendered every label 12% small, visible as
    /// "rust text is thinner" in the rows side-by-side.
    factor: f32,
    // (char, px*10) → coverage. The scene redraws every 160 ms and the
    // label set is nearly static; rasterizing outlines each frame measured
    // 4.7% CPU where the python office holds 0.5-2.9. Coverage is cached,
    // colour is applied at blit — one cache serves every ink.
    cache: RefCell<HashMap<(char, u32), Raster>>,
}

impl Text {
    pub fn load() -> Text {
        let load_one = |paths: &[std::path::PathBuf]| {
            for p in paths {
                if let Ok(bytes) = std::fs::read(p) {
                    if let Ok(f) = FontVec::try_from_vec(bytes) {
                        return Some(f);
                    }
                }
            }
            None
        };
        let base = crate::home().join(".local/share/fonts/JetBrainsMonoNerd");
        let font = load_one(&[
            base.join("JetBrainsMonoNerdFont-Regular.ttf"),
            "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf".into(),
        ]);
        // bold is a separate FACE, not a stroke trick — the rows renderers
        // set real weights; the fallback is just the regular face
        let bold = load_one(&[base.join("JetBrainsMonoNerdFont-Bold.ttf")]);
        let factor = font
            .as_ref()
            .and_then(|f| f.units_per_em().map(|em| f.height_unscaled() / em))
            .unwrap_or(1.32);
        Text {
            font,
            bold,
            factor,
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn face(&self, bold: bool) -> Option<&FontVec> {
        if bold {
            self.bold.as_ref().or(self.font.as_ref())
        } else {
            self.font.as_ref()
        }
    }

    /// Advance of `s` at `px` without drawing — the measure half of every
    /// right-align / ellipsize / centering decision.
    pub fn advance(&self, px: f32, bold: bool, s: &str) -> f32 {
        let Some(font) = self.face(bold) else { return 0.0 };
        let scale = PxScale::from(px * self.factor);
        let scaled = font.as_scaled(scale);
        s.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()
    }

    /// Draw `s` at (x, baseline y). Returns the advance, so callers can chain.
    pub fn draw(&self, pix: &mut Pixmap, x: f32, y: f32, px: f32, color: Rgb, s: &str) -> f32 {
        self.draw_weight(pix, x, y, px, color, s, false)
    }

    pub fn draw_weight(
        &self,
        pix: &mut Pixmap,
        x: f32,
        y: f32,
        px: f32,
        color: Rgb,
        s: &str,
        bold: bool,
    ) -> f32 {
        let Some(font) = self.face(bold) else { return 0.0 };
        let scale = PxScale::from(px * self.factor);
        let scaled = font.as_scaled(scale);
        let Rgb(cr, cg, cb) = color;
        let (w, h) = (pix.width() as i32, pix.height() as i32);
        let data = pix.data_mut();
        let mut cx = x;
        let mut cache = self.cache.borrow_mut();
        for ch in s.chars() {
            let key = (ch, (px * 10.0) as u32 * 2 + bold as u32);
            if !cache.contains_key(&key) {
                // rasterize at origin once; position is applied at blit
                let gid = font.glyph_id(ch);
                let glyph = gid.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
                let mut r = Raster {
                    w: 0,
                    h: 0,
                    min_x: 0.0,
                    min_y: 0.0,
                    advance: scaled.h_advance(gid),
                    cov: Vec::new(),
                };
                if let Some(og) = font.outline_glyph(glyph) {
                    let b = og.px_bounds();
                    r.w = b.width() as i32 + 1;
                    r.h = b.height() as i32 + 1;
                    r.min_x = b.min.x;
                    r.min_y = b.min.y;
                    r.cov = vec![0; (r.w * r.h) as usize];
                    og.draw(|gx, gy, cov| {
                        let i = (gy as i32 * r.w + gx as i32) as usize;
                        if i < r.cov.len() {
                            r.cov[i] = (cov * 255.0) as u8;
                        }
                    });
                }
                if cache.len() > 512 {
                    cache.clear();
                }
                cache.insert(key, r);
            }
            let r = &cache[&key];
            for gy in 0..r.h {
                for gx in 0..r.w {
                    let a = r.cov[(gy * r.w + gx) as usize] as u32;
                    if a == 0 {
                        continue;
                    }
                    let dx = (cx + r.min_x) as i32 + gx;
                    let dy = (y + r.min_y) as i32 + gy;
                    if dx < 0 || dy < 0 || dx >= w || dy >= h {
                        continue;
                    }
                    let i = ((dy * w + dx) * 4) as usize;
                    let inv = 255 - a;
                    // premultiplied RGBA over
                    data[i] = ((cr as u32 * a + data[i] as u32 * inv) / 255) as u8;
                    data[i + 1] = ((cg as u32 * a + data[i + 1] as u32 * inv) / 255) as u8;
                    data[i + 2] = ((cb as u32 * a + data[i + 2] as u32 * inv) / 255) as u8;
                    data[i + 3] = ((255 * a + data[i + 3] as u32 * inv) / 255) as u8;
                }
            }
            cx += r.advance;
        }
        cx - x
    }
}
