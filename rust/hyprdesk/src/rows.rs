//! The card row renderers, ported from lib/hyprdesk/rows.py. Painted by
//! the (future) rust cardhost and previews alike — same code, same pixels.
//!
//! Ink discipline (validated — don't re-derive by eye):
//!   accent2 → titles/section icons · fg → key values · sub → secondary
//!   good/bad → STATUS ONLY · muted → surfaces (bar tracks), never text ·
//!   accent → DATA FILLS only (bar fills, graph/sparkline strokes).
//!
//! Every renderer is `fn(pix, spec, ctx, y) -> height` and must also work
//! measure-only (ctx.measure = true, pix ignored) so cards can auto-size
//! before they map. MEASURE PARITY WITH THE PYTHON IS EXACT — heights
//! never depend on text extents, only on specs, files, dates and field
//! counts; `examples/rows_parity.rs` proves it per template.

use std::collections::HashMap;

use crate::cardspec::{subst, RowSpec};
use crate::draw::Text;
use crate::{Palette, Rgb};
use tiny_skia::Pixmap;

pub const PAD: f32 = 12.0;
pub const RADIUS: f32 = 12.0;
pub const GLASS_ALPHA: f32 = 0.80;
const ICON_W: f32 = 16.0;
const ICON_GAP: f32 = 5.0;

#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub pal: &'a Palette,
    pub params: &'a HashMap<String, String>,
    pub fields: &'a HashMap<String, String>,
    pub series: &'a HashMap<String, Vec<f64>>,
    pub width: f32,
    pub scale: f32,
    pub measure: bool,
    pub text: &'a Text,
    /// injected clock for parity tests; None = real localtime
    pub now: Option<(i32, u32, u32, u32, u32)>, // (year, mon, day, hour, min)
}

impl<'a> Ctx<'a> {
    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }
    fn ink(&self, slot: &str) -> Rgb {
        match slot {
            "bg" => self.pal.bg,
            "fg" => self.pal.fg,
            "accent" => self.pal.accent,
            "accent2" => self.pal.accent2,
            "muted" => self.pal.muted,
            "good" => self.pal.good,
            "bad" => self.pal.bad,
            "warn" => self.pal.warn,
            _ => self.pal.sub,
        }
    }
    fn icon_scale(&self) -> f32 {
        self.scale.round().max(1.0)
    }
    fn icon_w(&self, spec: &RowSpec) -> f32 {
        if spec.get("icon").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            0.0
        } else {
            (ICON_W + ICON_GAP) * self.icon_scale()
        }
    }
}

fn fill(pix: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgb, alpha: f32) {
    let mut p = tiny_skia::Paint::default();
    p.set_color(tiny_skia::Color::from_rgba8(
        c.0,
        c.1,
        c.2,
        (alpha * 255.0) as u8,
    ));
    if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, w, h) {
        pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
    }
}

fn spec_str<'b>(spec: &'b RowSpec, key: &str) -> &'b str {
    spec.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn spec_sub(spec: &RowSpec, key: &str, ctx: &Ctx) -> String {
    subst(spec_str(spec, key), ctx.params, Some(ctx.fields))
}

fn spec_i64(spec: &RowSpec, key: &str, default: i64) -> i64 {
    spec.get(key).and_then(|v| v.as_integer()).unwrap_or(default)
}

fn floats(text: &str) -> Vec<f64> {
    text.replace(' ', ",")
        .split(',')
        .filter_map(|t| t.parse().ok())
        .collect()
}

fn badge_slot(text: &str) -> &'static str {
    let t = text.trim();
    if t.starts_with('+') || t.starts_with('▲') || ["ok", "on", "up", "active"].contains(&t) {
        "good"
    } else if t.starts_with('-') || t.starts_with('▼') || ["down", "off", "alert"].contains(&t) {
        "bad"
    } else {
        "sub"
    }
}

fn ellipsize(ctx: &Ctx, px: f32, bold: bool, text: &str, max_w: f32) -> String {
    if ctx.text.advance(px, bold, text) <= max_w {
        return text.to_string();
    }
    let cap = ((max_w / 3.0) as usize + 4).max(1);
    let mut t: String = text.chars().take(cap).collect();
    while !t.is_empty() && ctx.text.advance(px, bold, &format!("{t}…")) > max_w {
        t.pop();
    }
    if t.is_empty() {
        String::new()
    } else {
        format!("{t}…")
    }
}

/// python _text: draw with slot ink at scaled size; returns advance.
#[allow(clippy::too_many_arguments)]
fn text(
    pix: &mut Pixmap,
    ctx: &Ctx,
    x: f32,
    y: f32,
    s: &str,
    slot: &str,
    size: f32,
    bold: bool,
    right: bool,
    max_w: Option<f32>,
) -> f32 {
    let px = size * ctx.scale;
    let s = match max_w {
        Some(mw) => ellipsize(ctx, px, bold, s, mw),
        None => s.to_string(),
    };
    let adv = ctx.text.advance(px, bold, &s);
    let x = if right { x - adv } else { x };
    ctx.text.draw_weight(pix, x, y, px, ctx.ink(slot), &s, bold);
    adv
}

fn icon(pix: &mut Pixmap, ctx: &Ctx, spec: &RowSpec, x: f32, baseline: f32, slot: &str) -> f32 {
    let name = spec_str(spec, "icon");
    if name.is_empty() {
        return 0.0;
    }
    let scale = ctx.icon_scale();
    if let Some(art) = crate::pixicons16::icon16(name) {
        let c = ctx.ink(slot);
        let ox = x.round();
        let oy = (baseline - ICON_W * scale * 0.78).round();
        for (ry, row) in art.iter().enumerate() {
            for (rx, ch) in row.chars().enumerate() {
                if ch == 'X' {
                    fill(
                        pix,
                        ox + rx as f32 * scale,
                        oy + ry as f32 * scale,
                        scale,
                        scale,
                        c,
                        1.0,
                    );
                }
            }
        }
    }
    // an unknown name still reserves its column — python parity: the space
    // is claimed by the spec, not by the bitmap lookup
    ctx.icon_w(spec)
}

// ---------------- row renderers ----------------

fn row_title(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(24.0);
    if !ctx.measure {
        let base = y + ctx.s(16.0);
        let t = spec_sub(spec, "text", ctx);
        let badge = spec_sub(spec, "badge", ctx);
        let badge_w = if badge.is_empty() {
            0.0
        } else {
            ctx.text.advance(10.0 * ctx.scale, false, &badge) + ctx.s(10.0)
        };
        let tx = icon(pix, ctx, spec, 0.0, base, "accent2");
        text(
            pix,
            ctx,
            tx,
            base,
            &t,
            "accent2",
            12.0,
            true,
            false,
            Some((ctx.width - badge_w - tx).max(ctx.s(40.0))),
        );
        if !badge.is_empty() {
            text(pix, ctx, ctx.width, base, &badge, "sub", 10.0, false, true, None);
        }
    }
    h
}

fn row_text(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(20.0);
    if !ctx.measure {
        let t = spec_sub(spec, "text", ctx);
        let ink = match spec_str(spec, "ink") {
            i @ ("fg" | "sub" | "accent2") => i,
            _ => "fg",
        };
        text(pix, ctx, 0.0, y + ctx.s(14.0), &t, ink, 11.0, false, false, Some(ctx.width));
    }
    h
}

fn row_keyval(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(21.0);
    if !ctx.measure {
        let base = y + ctx.s(15.0);
        let key = spec_sub(spec, "key", ctx);
        let val = spec_sub(spec, "value", ctx);
        let badge = spec_sub(spec, "badge", ctx);
        let key_ink = match spec_str(spec, "key_ink") {
            i @ ("sub" | "fg" | "accent2") => i,
            _ => "sub",
        };
        let kx = icon(pix, ctx, spec, 0.0, base, key_ink);
        let has_key2 = !spec_str(spec, "key2").is_empty();
        let key_budget = if has_key2 { ctx.width * 0.4 } else { ctx.width * 0.55 } - kx;
        let mut used =
            kx + text(pix, ctx, kx, base, &key, key_ink, 11.0, false, false, Some(key_budget));
        if has_key2 {
            let key2 = spec_sub(spec, "key2", ctx);
            used += ctx.s(8.0)
                + text(
                    pix,
                    ctx,
                    used + ctx.s(8.0),
                    base,
                    &key2,
                    "sub",
                    10.0,
                    false,
                    false,
                    Some(ctx.width * 0.55 - used),
                );
        }
        let mut x = ctx.width;
        if !badge.is_empty() {
            let pinned = spec_sub(spec, "badge_slot", ctx);
            let slot = if ["good", "bad", "sub", "fg", "accent2"].contains(&pinned.as_str()) {
                pinned
            } else {
                badge_slot(&badge).to_string()
            };
            x -= text(pix, ctx, x, base, &badge, &slot, 10.0, false, true, None) + ctx.s(10.0);
        }
        text(
            pix,
            ctx,
            x,
            base,
            &val,
            "fg",
            11.0,
            false,
            true,
            Some((x - used - ctx.s(12.0)).max(ctx.s(30.0))),
        );
    }
    h
}

fn row_bar(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let label = spec_str(spec, "label").to_string();
    let label_h = if label.is_empty() { 0.0 } else { ctx.s(19.0) };
    let bar_h = ctx.s(6.0);
    let h = label_h + bar_h + ctx.s(6.0);
    if !ctx.measure {
        let raw = subst(
            spec.get("value").and_then(|v| v.as_str()).unwrap_or("0"),
            ctx.params,
            Some(ctx.fields),
        );
        let vals = floats(&raw);
        let pct = vals.first().copied().unwrap_or(0.0).clamp(0.0, 100.0);
        if !label.is_empty() {
            let base = y + ctx.s(14.0);
            let lslot = match spec_str(spec, "label_ink") {
                i @ ("accent2" | "sub" | "fg") => i,
                _ => "accent2",
            };
            let lx = icon(pix, ctx, spec, 0.0, base, lslot);
            let lt = subst(&label, ctx.params, Some(ctx.fields));
            text(pix, ctx, lx, base, &lt, lslot, 11.0, false, false, None);
            let t = if spec_str(spec, "text").is_empty() {
                format!("{pct:.0}%")
            } else {
                spec_sub(spec, "text", ctx)
            };
            text(pix, ctx, ctx.width, base, &t, "fg", 11.0, false, true, None);
        }
        let by = y + label_h + ctx.s(2.0);
        fill(pix, 0.0, by, ctx.width, bar_h, ctx.ink("muted"), 1.0); // track: SURFACE
        fill(pix, 0.0, by, ctx.width * pct as f32 / 100.0, bar_h, ctx.ink("accent"), 1.0);
    }
    h
}

fn poly(
    pix: &mut Pixmap,
    ctx: &Ctx,
    vals: &[f64],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill_under: bool,
) {
    let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let rng = if hi - lo == 0.0 { 1.0 } else { hi - lo };
    let n = (vals.len() - 1).max(1);
    let pts: Vec<(f32, f32)> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                x + w * i as f32 / n as f32,
                y + h - ((v - lo) / rng) as f32 * h,
            )
        })
        .collect();
    let mut paint = tiny_skia::Paint::default();
    paint.anti_alias = true;
    if fill_under {
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(x, y + h);
        for &(px_, py_) in &pts {
            pb.line_to(px_, py_);
        }
        pb.line_to(x + w, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            let a = ctx.ink("accent");
            paint.set_color(tiny_skia::Color::from_rgba8(a.0, a.1, a.2, 56)); // 0.22
            pix.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for &(px_, py_) in &pts[1..] {
        pb.line_to(px_, py_);
    }
    if let Some(path) = pb.finish() {
        let a = ctx.ink("accent");
        paint.set_color(tiny_skia::Color::from_rgba8(a.0, a.1, a.2, 255));
        let stroke = tiny_skia::Stroke {
            width: ctx.s(1.2).max(1.0),
            ..Default::default()
        };
        pix.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
    }
}

fn row_sparkline(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(spec_i64(spec, "height", 22) as f32);
    if !ctx.measure {
        let vals = floats(&spec_sub(spec, "data", ctx));
        if vals.len() >= 2 {
            poly(pix, ctx, &vals, 0.0, y + 2.0, ctx.width, h - 6.0, false);
        } else {
            text(pix, ctx, 0.0, y + h - 6.0, "no data", "sub", 9.0, false, false, None);
        }
    }
    h + ctx.s(4.0)
}

fn row_graph(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(spec_i64(spec, "height", 28) as f32);
    if !ctx.measure {
        let series = ctx
            .series
            .get(spec_str(spec, "series"))
            .cloned()
            .unwrap_or_default();
        if series.len() >= 2 {
            poly(pix, ctx, &series, 0.0, y + 2.0, ctx.width, h - 4.0, true);
        } else {
            text(pix, ctx, 0.0, y + h - 6.0, "collecting…", "sub", 9.0, false, false, None);
        }
    }
    h + ctx.s(4.0)
}

fn row_hr(pix: &mut Pixmap, _spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let h = ctx.s(9.0);
    if !ctx.measure {
        fill(pix, 0.0, y + h / 2.0, ctx.width, 1.0, ctx.ink("sub"), 0.35);
    }
    h
}

// ---------------- date/time (libc localtime — no chrono dep) ----------------

fn now_parts(ctx: &Ctx) -> (i32, u32, u32, u32, u32) {
    if let Some(n) = ctx.now {
        return n;
    }
    unsafe {
        let t = libc_time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        localtime_r(&t, &mut tm);
        (
            tm.tm_year + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
        )
    }
}

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}
extern "C" {
    #[link_name = "time"]
    fn libc_time(t: *mut i64) -> i64;
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// day-of-week (0=Monday) via Zeller's congruence.
fn weekday(y: i32, m: u32, d: u32) -> usize {
    let (y, m) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let k = y % 100;
    let j = y / 100;
    let h = (d as i32 + 13 * (m as i32 + 1) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    ((h + 5) % 7) as usize // Zeller 0=Saturday → 0=Monday
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// python calendar.Calendar(firstweekday=6).monthdayscalendar — Sunday-
/// first weeks, zeros in out-of-month cells.
pub fn month_weeks(y: i32, m: u32) -> Vec<[u32; 7]> {
    let first_dow = weekday(y, m, 1); // 0=Mon..6=Sun
    let lead = (first_dow + 1) % 7; // column of day 1 in a Sun-first week
    let n = days_in_month(y, m);
    let mut weeks = Vec::new();
    let mut week = [0u32; 7];
    let mut col = lead;
    for day in 1..=n {
        week[col] = day;
        col += 1;
        if col == 7 {
            weeks.push(week);
            week = [0; 7];
            col = 0;
        }
    }
    if col > 0 {
        weeks.push(week);
    }
    weeks
}

fn row_clock(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let size_big = spec_i64(spec, "size_big", 52) as f32;
    let big_h = ctx.s(size_big + 6.0);
    let small_h = ctx.s(22.0);
    if !ctx.measure {
        let (yy, mm, dd, hh, mi) = now_parts(ctx);
        let dow = WEEKDAYS[weekday(yy, mm, dd)];
        // fixed formats only: every template on this box uses the defaults,
        // and a strftime engine for four fields is complexity nobody asked
        // for. A template with a custom fmt_* gets the default — visibly,
        // not silently wrong. (Documented in the plan; revisit on demand.)
        let big = format!("{hh:02}:{mi:02}");
        text(pix, ctx, ctx.width, y + ctx.s(size_big), &big, "fg", size_big, true, true, None);
        let rest = format!("{dd:02} {} {yy}", MONTHS[(mm - 1) as usize]);
        let base = y + big_h + ctx.s(12.0);
        let rest_w = ctx.text.advance(12.0 * ctx.scale, false, &rest);
        let wd_w = ctx.text.advance(12.0 * ctx.scale, false, dow);
        let x = ctx.width - rest_w - ctx.s(10.0) - wd_w;
        text(pix, ctx, x, base, dow, "accent2", 12.0, false, false, None);
        text(pix, ctx, ctx.width, base, &rest, "fg", 12.0, false, true, None);
    }
    big_h + small_h
}

fn row_calgrid(pix: &mut Pixmap, _spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let line = ctx.s(17.0);
    let (yy, mm, today, _, _) = now_parts(ctx);
    let weeks = month_weeks(yy, mm);
    let h = line * (2 + weeks.len()) as f32 + ctx.s(6.0);
    if !ctx.measure {
        let mut y = y;
        let col_w = ctx.width / 7.0;
        let title = format!("{} {yy}", MONTHS[(mm - 1) as usize]);
        let tw = ctx.text.advance(12.0 * ctx.scale, true, &title);
        text(pix, ctx, (ctx.width - tw) / 2.0, y + ctx.s(13.0), &title, "accent2", 12.0, true, false, None);
        y += line;
        let base = y + ctx.s(13.0);
        for (i, d) in ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"].iter().enumerate() {
            let ew = ctx.text.advance(10.0 * ctx.scale, true, d);
            text(
                pix,
                ctx,
                i as f32 * col_w + (col_w - ew) / 2.0,
                base,
                d,
                "sub",
                10.0,
                true,
                false,
                None,
            );
        }
        for (wi, week) in weeks.iter().enumerate() {
            let wy = base + line * (wi + 1) as f32;
            for (di, &day) in week.iter().enumerate() {
                if day == 0 {
                    continue;
                }
                let t = day.to_string();
                let is_today = day == today;
                let ew = ctx.text.advance(10.0 * ctx.scale, is_today, &t);
                let cx = di as f32 * col_w + col_w / 2.0;
                let slot = if is_today {
                    // today: accent pill, bg ink
                    let mut pb = tiny_skia::PathBuilder::new();
                    pb.push_circle(cx, wy - ctx.s(3.5), ctx.s(9.0));
                    if let Some(path) = pb.finish() {
                        let a = ctx.ink("accent");
                        let mut p = tiny_skia::Paint::default();
                        p.anti_alias = true;
                        p.set_color(tiny_skia::Color::from_rgba8(a.0, a.1, a.2, 255));
                        pix.fill_path(
                            &path,
                            &p,
                            tiny_skia::FillRule::Winding,
                            tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                    "bg"
                } else {
                    "fg"
                };
                text(pix, ctx, cx - ew / 2.0, wy, &t, slot, 10.0, is_today, false, None);
            }
        }
    }
    h
}

fn row_notesfile(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let raw = spec_sub(spec, "path", ctx);
    let path: std::path::PathBuf = if let Some(rest) = raw.strip_prefix("~/") {
        crate::home().join(rest)
    } else {
        raw.clone().into()
    };
    let max_lines = spec_i64(spec, "max_lines", 10) as usize;
    let lines: Vec<String> = std::fs::read_to_string(&path)
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim().is_empty())
                .take(max_lines)
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default();
    let line_h = ctx.s(18.0);
    let h = line_h * lines.len().max(1) as f32;
    if !ctx.measure {
        if lines.is_empty() {
            let empty = spec
                .get("empty")
                .and_then(|v| v.as_str())
                .unwrap_or("run hypr-notes to start");
            text(pix, ctx, 0.0, y + ctx.s(13.0), empty, "sub", 10.0, false, false, None);
        }
        for (i, ln) in lines.iter().enumerate() {
            text(
                pix,
                ctx,
                0.0,
                y + ctx.s(13.0) + i as f32 * line_h,
                ln,
                "fg",
                11.0,
                false,
                false,
                Some(ctx.width),
            );
        }
    }
    h
}

fn row_heatmap(pix: &mut Pixmap, spec: &RowSpec, ctx: &Ctx, y: f32) -> f32 {
    let vals: Vec<i64> = floats(&spec_sub(spec, "data", ctx))
        .iter()
        .map(|v| *v as i64)
        .collect();
    // ceil: a partial trailing week renders as a short column — dropping it
    // would hide the NEWEST days
    let weeks = vals.len().div_ceil(7).max(1);
    let cell = ctx.s(13.0).min(ctx.width / weeks as f32);
    let boxs = cell * 0.68;
    let h = cell * 7.0 + ctx.s(4.0);
    if !ctx.measure {
        if vals.is_empty() {
            text(pix, ctx, 0.0, y + ctx.s(14.0), "no data", "sub", 10.0, false, false, None);
        }
        let ramp: [(&str, f32); 5] = [
            ("sub", 0.35),
            ("sub", 0.9),
            ("fg", 0.55),
            ("fg", 1.0),
            ("accent2", 1.0),
        ];
        for (i, lv) in vals.iter().enumerate() {
            let (wk, dow) = (i / 7, i % 7);
            let (slot, alpha) = ramp[(*lv).clamp(0, 4) as usize];
            fill(
                pix,
                wk as f32 * cell,
                y + dow as f32 * cell,
                boxs,
                boxs,
                ctx.ink(slot),
                alpha,
            );
        }
    }
    h
}

// ---------------- assembly ----------------

fn repeat_groups(fields: &HashMap<String, String>, over: &str) -> Vec<usize> {
    let prefix = format!("{over}.");
    let mut idx: Vec<usize> = fields
        .keys()
        .filter_map(|k| k.strip_prefix(&prefix)?.split('.').next()?.parse::<usize>().ok())
        .collect();
    idx.sort_unstable();
    idx.dedup();
    idx
}

pub fn render_rows(pix: &mut Pixmap, rows: &[RowSpec], ctx: &Ctx, y: f32) -> f32 {
    let mut total = 0.0;
    for spec in rows {
        let rtype = spec_str(spec, "type");
        if rtype == "repeat" {
            let over = spec_str(spec, "over");
            let inner: Vec<RowSpec> = spec
                .get("rows")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_table().cloned()).collect())
                .unwrap_or_default();
            for i in repeat_groups(ctx.fields, over) {
                let mut sub_fields = ctx.fields.clone();
                let prefix = format!("{over}.{i}.");
                for (k, v) in ctx.fields {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        sub_fields.insert(format!("{over}.{rest}"), v.clone());
                    }
                }
                let sub_ctx = Ctx {
                    fields: &sub_fields,
                    ..*ctx
                };
                total += render_rows(pix, &inner, &sub_ctx, y + total);
            }
            continue;
        }
        let f = match rtype {
            "title" => row_title,
            "text" => row_text,
            "keyval" => row_keyval,
            "bar" => row_bar,
            "sparkline" => row_sparkline,
            "graph" => row_graph,
            "hr" => row_hr,
            "clock" => row_clock,
            "calgrid" => row_calgrid,
            "notesfile" => row_notesfile,
            "heatmap" => row_heatmap,
            _ => continue,
        };
        total += f(pix, spec, ctx, y + total);
    }
    total
}

pub fn draw_glass(pix: &mut Pixmap, w: f32, h: f32, pal: &Palette) {
    let r = RADIUS;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.quad_to(w, 0.0, w, r);
    pb.line_to(w, h - r);
    pb.quad_to(w, h, w - r, h);
    pb.line_to(r, h);
    pb.quad_to(0.0, h, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.quad_to(0.0, 0.0, r, 0.0);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let mut p = tiny_skia::Paint::default();
    p.anti_alias = true;
    p.set_color(tiny_skia::Color::from_rgba8(
        pal.bg.0,
        pal.bg.1,
        pal.bg.2,
        (GLASS_ALPHA * 255.0) as u8,
    ));
    pix.fill_path(&path, &p, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    p.set_color(tiny_skia::Color::from_rgba8(
        pal.accent2.0,
        pal.accent2.1,
        pal.accent2.2,
        46, // 0.18 hairline edge
    ));
    let stroke = tiny_skia::Stroke {
        width: 1.0,
        ..Default::default()
    };
    pix.stroke_path(&path, &p, &stroke, tiny_skia::Transform::identity(), None);
}

pub fn measure(rows: &[RowSpec], ctx: &Ctx) -> f32 {
    let mctx = Ctx {
        measure: true,
        ..*ctx
    };
    let mut dummy = Pixmap::new(1, 1).unwrap();
    render_rows(&mut dummy, rows, &mctx, 0.0)
}

/// Full card: glass body + padded rows. ctx.width is CONTENT width; the
/// card pixmap is width + 2*PAD wide. Returns total card height.
pub fn render_card(pix: &mut Pixmap, rows: &[RowSpec], ctx: &Ctx) -> f32 {
    let content_h = measure(rows, ctx);
    draw_glass(pix, ctx.width + 2.0 * PAD, content_h + 2.0 * PAD, ctx.pal);
    // tiny-skia has no cairo-style translate on the target, so rows render
    // into a content-sized pixmap blitted at (PAD, PAD)
    let mut content = Pixmap::new(
        (ctx.width.ceil() as u32).max(1),
        (content_h.ceil() as u32).max(1),
    )
    .unwrap();
    render_rows(&mut content, rows, ctx, 0.0);
    pix.draw_pixmap(
        PAD as i32,
        PAD as i32,
        content.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tiny_skia::Transform::identity(),
        None,
    );
    content_h + 2.0 * PAD
}
