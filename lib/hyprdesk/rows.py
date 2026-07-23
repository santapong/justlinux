"""Shared cairo row renderers for the glass card fleet.

Imported by BOTH hypr-cardhost (live cards) and hypr-widgetpicker
(previews) — the same code paints both, so what you pick is exactly what
lands on the desktop.

Ink discipline (validated — don't re-derive by eye):
  accent2 → titles/section icons · fg → key values · sub → secondary
  good/bad → STATUS ONLY · muted → surfaces (bar tracks), never text ·
  accent → DATA FILLS only (bar fills, graph/sparkline strokes) — the
  wallpaper's primary accent marks measured data, never labels.

Every renderer is ``func(cr, spec, ctx, y) -> height`` and must also
work with measure-only calls (same signature, cr from a 1×1 surface,
ctx["measure"]=True) so cards can auto-size before they map.
"""
import calendar
import time
from pathlib import Path

import cairo

from .cardspec import subst
from .theme import rgb

FONT = "JetBrainsMono Nerd Font"
PAD = 12                 # card inner padding
RADIUS = 12              # card corner radius
GLASS_ALPHA = 0.80       # matches conky argb 205 tint


def _font(cr, ctx, size, bold=False):
    cr.select_font_face(FONT, cairo.FONT_SLANT_NORMAL,
                        cairo.FONT_WEIGHT_BOLD if bold
                        else cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(size * ctx.get("scale", 1.0))


def _ink(cr, ctx, slot, alpha=1.0):
    cr.set_source_rgba(*rgb(ctx["pal"][slot]), alpha)


def _s(ctx, v):
    return v * ctx.get("scale", 1.0)


def _ellipsize(cr, text, max_w):
    if cr.text_extents(text).x_advance <= max_w:
        return text
    while text and cr.text_extents(text + "…").x_advance > max_w:
        text = text[:-1]
    return text + "…" if text else ""


def _text(cr, ctx, x, y, text, slot, size, bold=False, right=False,
          max_w=None):
    _font(cr, ctx, size, bold)
    if max_w is not None:
        text = _ellipsize(cr, text, max_w)
    if right:
        x -= cr.text_extents(text).x_advance
    _ink(cr, ctx, slot)
    cr.move_to(x, y)
    cr.show_text(text)
    return cr.text_extents(text).x_advance


def _badge_slot(text):
    t = str(text).strip()
    if t.startswith(("+", "▲")) or t in ("ok", "on", "up", "active"):
        return "good"
    if t.startswith(("-", "▼")) or t in ("down", "off", "alert"):
        return "bad"
    return "sub"


def _floats(text):
    out = []
    for tok in str(text).replace(" ", ",").split(","):
        try:
            out.append(float(tok))
        except ValueError:
            pass
    return out


# ---------------- row renderers ----------------

def row_title(cr, spec, ctx, y):
    h = _s(ctx, 24)
    text = subst(spec.get("text", ""), ctx["params"], ctx["fields"])
    badge = subst(spec.get("badge", ""), ctx["params"], ctx["fields"]) \
        if spec.get("badge") else ""
    if not ctx.get("measure"):
        base = y + _s(ctx, 16)
        # measure the badge FIRST — the title's budget is whatever the
        # badge leaves over (a fixed reserve overlaps on long badges)
        badge_w = 0
        if badge:
            _font(cr, ctx, 10)
            badge_w = cr.text_extents(badge).x_advance + _s(ctx, 10)
        _text(cr, ctx, 0, base, text, "accent2", 12, bold=True,
              max_w=max(_s(ctx, 40), ctx["width"] - badge_w))
        if badge:
            _text(cr, ctx, ctx["width"], base, badge, "sub", 10, right=True)
    return h


def row_text(cr, spec, ctx, y):
    h = _s(ctx, 20)
    if not ctx.get("measure"):
        text = subst(spec.get("text", ""), ctx["params"], ctx["fields"])
        _text(cr, ctx, 0, y + _s(ctx, 14),
              text, spec.get("ink", "fg") if spec.get("ink") in
              ("fg", "sub", "accent2") else "fg",
              11, max_w=ctx["width"])
    return h


def row_keyval(cr, spec, ctx, y):
    h = _s(ctx, 21)
    if not ctx.get("measure"):
        base = y + _s(ctx, 15)
        key = subst(spec.get("key", ""), ctx["params"], ctx["fields"])
        val = subst(spec.get("value", ""), ctx["params"], ctx["fields"])
        badge = subst(spec.get("badge", ""), ctx["params"], ctx["fields"]) \
            if spec.get("badge") else ""
        # key_ink lifts a primary identifier to fg (repo names etc.);
        # key2 appends a secondary segment in sub (branch names etc.)
        key_ink = spec.get("key_ink", "sub")
        if key_ink not in ("sub", "fg", "accent2"):
            key_ink = "sub"
        used = _text(cr, ctx, 0, base, key, key_ink, 11,
                     max_w=ctx["width"] * 0.4 if spec.get("key2")
                     else ctx["width"] * 0.55)
        key2 = subst(spec.get("key2", ""), ctx["params"], ctx["fields"]) \
            if spec.get("key2") else ""
        if key2:
            used += _s(ctx, 8) + _text(
                cr, ctx, used + _s(ctx, 8), base, key2, "sub", 10,
                max_w=ctx["width"] * 0.55 - used)
        x = ctx["width"]
        if badge:
            # template may pin the status ink (badge_slot = "{fw_slot}")
            # — else it derives from the badge text (+/- etc.)
            slot = subst(spec.get("badge_slot", ""), ctx["params"],
                         ctx["fields"]) if spec.get("badge_slot") else ""
            if slot not in ("good", "bad", "sub", "fg", "accent2"):
                slot = _badge_slot(badge)
            x -= _text(cr, ctx, x, base, badge, slot, 10,
                       right=True) + _s(ctx, 10)
        # value budget = whatever the key and badge left over (no overlap)
        _text(cr, ctx, x, base, val, "fg", 11, right=True,
              max_w=max(_s(ctx, 30), x - used - _s(ctx, 12)))
    return h


def row_bar(cr, spec, ctx, y):
    label = spec.get("label", "")
    label_h = _s(ctx, 19) if label else 0
    bar_h = _s(ctx, 6)
    h = label_h + bar_h + _s(ctx, 6)
    if not ctx.get("measure"):
        pct_raw = subst(spec.get("value", "0"), ctx["params"], ctx["fields"])
        vals = _floats(pct_raw)
        pct = max(0.0, min(100.0, vals[0] if vals else 0.0))
        if label:
            base = y + _s(ctx, 14)
            # label_ink: stats-style section labels stay accent2 (their
            # twin used color1), quieter fleets (claude) choose sub
            lslot = spec.get("label_ink", "accent2")
            if lslot not in ("accent2", "sub", "fg"):
                lslot = "accent2"
            _text(cr, ctx, 0, base,
                  subst(label, ctx["params"], ctx["fields"]), lslot, 11)
            text = subst(spec.get("text", ""), ctx["params"], ctx["fields"]) \
                if spec.get("text") else f"{pct:.0f}%"
            _text(cr, ctx, ctx["width"], base, text, "fg", 11, right=True)
        by = y + label_h + _s(ctx, 2)
        _ink(cr, ctx, "muted")                       # track: SURFACE color
        cr.rectangle(0, by, ctx["width"], bar_h)
        cr.fill()
        _ink(cr, ctx, "accent")
        cr.rectangle(0, by, ctx["width"] * pct / 100.0, bar_h)
        cr.fill()
    return h


def _poly(cr, ctx, vals, x, y, w, h, fill):
    lo, hi = min(vals), max(vals)
    rng = (hi - lo) or 1.0
    pts = [(x + w * i / max(1, len(vals) - 1),
            y + h - (v - lo) / rng * h) for i, v in enumerate(vals)]
    if fill:
        cr.move_to(x, y + h)
        for px, py in pts:
            cr.line_to(px, py)
        cr.line_to(x + w, y + h)
        cr.close_path()
        _ink(cr, ctx, "accent", 0.22)
        cr.fill()
    cr.move_to(*pts[0])
    for px, py in pts[1:]:
        cr.line_to(px, py)
    _ink(cr, ctx, "accent")
    cr.set_line_width(max(1.0, _s(ctx, 1.2)))
    cr.stroke()


def row_sparkline(cr, spec, ctx, y):
    h = _s(ctx, int(spec.get("height", 22)))
    if not ctx.get("measure"):
        vals = _floats(subst(spec.get("data", ""), ctx["params"],
                             ctx["fields"]))
        if len(vals) >= 2:
            _poly(cr, ctx, vals, 0, y + 2, ctx["width"], h - 6, fill=False)
        else:
            _text(cr, ctx, 0, y + h - 6, "no data", "sub", 9)
    return h + _s(ctx, 4)


def row_graph(cr, spec, ctx, y):
    h = _s(ctx, int(spec.get("height", 28)))
    if not ctx.get("measure"):
        series = (ctx.get("series") or {}).get(spec.get("series", ""), [])
        if len(series) >= 2:
            _poly(cr, ctx, series, 0, y + 2, ctx["width"], h - 4, fill=True)
        else:
            _text(cr, ctx, 0, y + h - 6, "collecting…", "sub", 9)
    return h + _s(ctx, 4)


def row_hr(cr, spec, ctx, y):
    h = _s(ctx, 9)
    if not ctx.get("measure"):
        _ink(cr, ctx, "sub", 0.35)
        cr.rectangle(0, y + h / 2, ctx["width"], 1)
        cr.fill()
    return h


def row_clock(cr, spec, ctx, y):
    size_big = int(spec.get("size_big", 52))
    big_h = _s(ctx, size_big + 6)
    small_h = _s(ctx, 22)
    if not ctx.get("measure"):
        now = time.localtime()
        big = time.strftime(spec.get("fmt_big", "%H:%M"), now)
        _text(cr, ctx, ctx["width"], y + _s(ctx, size_big), big, "fg",
              size_big, bold=True, right=True)
        wd = time.strftime(spec.get("fmt_accent", "%A"), now)
        rest = time.strftime(spec.get("fmt_small", "%d %B %Y"), now)
        base = y + big_h + _s(ctx, 12)
        _font(cr, ctx, 12)
        rest_w = cr.text_extents(rest).x_advance
        wd_w = cr.text_extents(wd).x_advance
        x = ctx["width"] - rest_w - _s(ctx, 10) - wd_w
        _text(cr, ctx, x, base, wd, "accent2", 12)
        _text(cr, ctx, ctx["width"], base, rest, "fg", 12, right=True)
    return big_h + small_h


def row_calgrid(cr, spec, ctx, y):
    line = _s(ctx, 17)
    now = time.localtime()
    # Sunday-first like the retired conky twin (cal(1) convention)
    cal = calendar.Calendar(firstweekday=6)
    weeks = cal.monthdayscalendar(now.tm_year, now.tm_mon)
    # exact: month header + day names + this month's real week count
    # (4–6) — a fixed 9-line allocation left a blank band at the bottom
    h = line * (2 + len(weeks)) + _s(ctx, 6)
    if not ctx.get("measure"):
        col_w = ctx["width"] / 7
        # centered month header, title ink
        title = time.strftime("%B %Y", now)
        _font(cr, ctx, 12, bold=True)
        ext = cr.text_extents(title)
        _ink(cr, ctx, "accent2")
        cr.move_to((ctx["width"] - ext.x_advance) / 2, y + _s(ctx, 13))
        cr.show_text(title)
        y += line
        base = y + _s(ctx, 13)
        _font(cr, ctx, 10, bold=True)
        for i, d in enumerate(("Su", "Mo", "Tu", "We", "Th", "Fr", "Sa")):
            _ink(cr, ctx, "sub")
            ext = cr.text_extents(d)
            cr.move_to(i * col_w + (col_w - ext.x_advance) / 2, base)
            cr.show_text(d)
        for wi, week in enumerate(weeks):
            wy = base + line * (wi + 1)
            for di, day in enumerate(week):
                if not day:
                    continue
                txt = str(day)
                _font(cr, ctx, 10, bold=day == now.tm_mday)
                ext = cr.text_extents(txt)
                cx = di * col_w + col_w / 2
                if day == now.tm_mday:      # today: accent pill, bg ink
                    _ink(cr, ctx, "accent")
                    r = _s(ctx, 9)
                    cr.arc(cx, wy - _s(ctx, 3.5), r, 0, 6.2832)
                    cr.fill()
                    _ink(cr, ctx, "bg")
                else:
                    _ink(cr, ctx, "fg")
                cr.move_to(cx - ext.x_advance / 2, wy)
                cr.show_text(txt)
    return h


def row_notesfile(cr, spec, ctx, y):
    path = Path(subst(spec.get("path", ""), ctx["params"],
                      ctx["fields"])).expanduser()
    max_lines = int(spec.get("max_lines", 10))
    try:
        lines = [ln for ln in path.read_text(errors="replace").splitlines()
                 if ln.strip()][:max_lines]
    except OSError:
        lines = []
    line_h = _s(ctx, 18)
    h = line_h * max(1, len(lines))
    if not ctx.get("measure"):
        if not lines:
            _text(cr, ctx, 0, y + _s(ctx, 13),
                  spec.get("empty", "run hypr-notes to start"), "sub", 10)
        for i, ln in enumerate(lines):
            _text(cr, ctx, 0, y + _s(ctx, 13) + i * line_h, ln, "fg", 11,
                  max_w=ctx["width"])
    return h


def row_heatmap(cr, spec, ctx, y):
    """GitHub-style contribution grid: data = comma ints 0..4, date-sorted
    (7 per week, column per week). Ink ramp: sub → fg → accent2."""
    vals = [int(v) for v in _floats(subst(spec.get("data", ""),
                                          ctx["params"], ctx["fields"]))]
    # ceil: a partial trailing week renders as a short column — dropping
    # it would hide the NEWEST days
    weeks = max(1, -(-len(vals) // 7))
    cell = min(_s(ctx, 13), ctx["width"] / weeks)
    box = cell * 0.68
    h = cell * 7 + _s(ctx, 4)
    if not ctx.get("measure"):
        if not vals:
            _text(cr, ctx, 0, y + _s(ctx, 14), "no data", "sub", 10)
        ramp = (("sub", 0.35), ("sub", 0.9), ("fg", 0.55), ("fg", 1.0),
                ("accent2", 1.0))
        for i, lv in enumerate(vals):
            wk, dow = divmod(i, 7)
            slot, alpha = ramp[max(0, min(4, lv))]
            _ink(cr, ctx, slot, alpha)
            cr.rectangle(wk * cell, y + dow * cell, box, box)
            cr.fill()
    return h


RENDERERS = {
    "title": row_title, "text": row_text, "keyval": row_keyval,
    "bar": row_bar, "sparkline": row_sparkline, "graph": row_graph,
    "hr": row_hr, "clock": row_clock, "calgrid": row_calgrid,
    "notesfile": row_notesfile, "heatmap": row_heatmap,
}


def _repeat_groups(fields, over):
    """Indices i present as '<over>.<i>.<key>' fields, in order."""
    idx = set()
    prefix = over + "."
    for k in fields:
        if k.startswith(prefix):
            rest = k[len(prefix):].split(".", 1)
            if rest and rest[0].isdigit():
                idx.add(int(rest[0]))
    return sorted(idx)


def render_rows(cr, rows, ctx, y):
    """Draw (or measure) a row stack; returns total height."""
    total = 0
    for spec in rows:
        rtype = spec.get("type")
        if rtype == "repeat":
            over = spec.get("over", "")
            for i in _repeat_groups(ctx["fields"], over):
                sub_fields = dict(ctx["fields"])
                prefix = f"{over}.{i}."
                for k, v in ctx["fields"].items():
                    if k.startswith(prefix):
                        sub_fields[f"{over}.{k[len(prefix):]}"] = v
                sub_ctx = dict(ctx, fields=sub_fields)
                total += render_rows(cr, spec.get("rows") or [],
                                     sub_ctx, y + total)
            continue
        fn = RENDERERS.get(rtype)
        if fn:
            total += fn(cr, spec, ctx, y + total)
    return total


def draw_glass(cr, w, h, pal, alpha=GLASS_ALPHA, radius=RADIUS):
    """The card body: rounded frosted tint (Hyprland blurs behind it)."""
    r = radius
    cr.new_path()
    cr.arc(w - r, r, r, -1.5708, 0)
    cr.arc(w - r, h - r, r, 0, 1.5708)
    cr.arc(r, h - r, r, 1.5708, 3.1416)
    cr.arc(r, r, r, 3.1416, 4.7124)
    cr.close_path()
    cr.set_source_rgba(*rgb(pal["bg"]), alpha)
    cr.fill_preserve()
    cr.set_source_rgba(*rgb(pal["accent2"]), 0.18)   # hairline edge
    cr.set_line_width(1)
    cr.stroke()


def measure(rows, ctx):
    """Card content height using a throwaway 1×1 surface."""
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, 1, 1)
    cr = cairo.Context(surf)
    mctx = dict(ctx, measure=True)
    return render_rows(cr, rows, mctx, 0)


def render_card(cr, rows, ctx):
    """Full card: glass body + padded rows. ctx['width'] is CONTENT width;
    the card surface is width + 2*PAD wide."""
    content_h = measure(rows, ctx)
    draw_glass(cr, ctx["width"] + 2 * PAD, content_h + 2 * PAD, ctx["pal"])
    cr.save()
    cr.translate(PAD, PAD)
    render_rows(cr, rows, ctx, 0)
    cr.restore()
    return content_h + 2 * PAD
