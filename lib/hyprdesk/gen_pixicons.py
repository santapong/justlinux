#!/usr/bin/env python3
"""Regenerate hyprdesk/pixicons.py from the pixelarticons SVG set.

Why bitmaps and not the font: a pixel icon has to land on whole pixels or
it stops being a pixel icon, and cairo will happily hint and antialias a
webfont into mush at 12px. Rasterising once, offline, at the exact size we
draw makes crispness structural instead of a rendering flag we have to keep
getting right. It also drops straight into draw_art(), which the pet and the
office already use, and takes its colour from the current cairo source —
so the icons re-theme with wallust for free.

    npm install pixelarticons --no-save     # 877 icons, MIT
    python3 gen_pixicons.py <path-to>/node_modules/pixelarticons/svg

Upstream: https://github.com/halfmage/pixelarticons — MIT, (c) 2019 Gerrit
Halfmann. Icons are 24x24 by design; we emit 24 and a 16 for tight rows.
"""
import os
import subprocess
import sys
import tempfile

# card / row  ->  upstream svg basename. Keep this list SHORT: every icon
# is bytes in a module that gets imported by every card process.
WANTED = {
    "cpu": "cpu.svg",
    "ram": "memory-stick.svg",
    "disk": "save.svg",
    "net": "cellular-signal-3.svg",
    "down": "download.svg",
    "up": "upload.svg",
    "wifi": "wifi.svg",
    "clock": "clock.svg",
    "calendar": "calendar-2.svg",
    "weather": "cloud.svg",
    "moon": "moon.svg",
    "music": "music.svg",
    "git": "git-branch.svg",
    "commit": "git-commit.svg",
    "chart": "chart-bar-big.svg",
    "shield": "shield.svg",
    "archive": "archive.svg",
    "alarm": "alarm-clock.svg",
    "terminal": "terminal.svg",
    "zap": "zap.svg",
    "robot": "robot-face-happy.svg",
    "coins": "coins.svg",
    "sun": "cloud-sun.svg",
}

# 24 is the set's native grid and the only lossless size. 16 fits a bar
# row's 19px label line; 12 was tried and dropped — halving the grid mushed
# archive/calendar/terminal into unreadable blobs.
SIZES = (24, 16)


def raster(svg_path, size):
    """SVG -> a list of strings, 'X' where the icon is opaque."""
    body = open(svg_path).read().replace("currentColor", "#000000")
    with tempfile.NamedTemporaryFile("w", suffix=".svg", delete=False) as f:
        f.write(body)
        tmp = f.name
    png = tmp + ".png"
    try:
        r = subprocess.run(
            ["magick", "-background", "none", tmp,
             "-resize", f"{size}x{size}", png],
            capture_output=True, text=True)
        if r.returncode != 0:
            return None
        from PIL import Image
        im = Image.open(png).convert("RGBA")
        if im.size != (size, size):
            im = im.resize((size, size), Image.NEAREST)
        rows = []
        for y in range(size):
            rows.append("".join(
                "X" if im.getpixel((x, y))[3] > 110 else "."
                for x in range(size)))
        return rows
    finally:
        for p in (tmp, png):
            try:
                os.unlink(p)
            except OSError:
                pass


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    src = sys.argv[1]
    out = {}
    for name, fn in sorted(WANTED.items()):
        path = os.path.join(src, fn)
        if not os.path.exists(path):
            print(f"  MISSING {fn} — skipped", file=sys.stderr)
            continue
        for size in SIZES:
            rows = raster(path, size)
            if not rows:
                print(f"  FAILED {fn}@{size}", file=sys.stderr)
                continue
            ink = sum(r.count("X") for r in rows)
            if not 8 < ink < size * size * 0.85:
                print(f"  SUSPECT {name}@{size}: {ink}px set", file=sys.stderr)
            out.setdefault(size, {})[name] = rows

    dest = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "pixicons.py")
    with open(dest, "w") as f:
        f.write('"""Pixel icons as bitmaps — GENERATED, do not hand-edit.\n\n'
                'Regenerate with gen_pixicons.py. Source: pixelarticons\n'
                '(https://github.com/halfmage/pixelarticons), MIT, (c) 2019\n'
                'Gerrit Halfmann. Rasterised offline so every icon lands on\n'
                'whole pixels; colour comes from the caller, so wallust still\n'
                'drives it.\n"""\n\n')
        for size in SIZES:
            f.write(f"ICONS_{size} = {{\n")
            for name, rows in sorted(out.get(size, {}).items()):
                f.write(f'    "{name}": (\n')
                for r in rows:
                    f.write(f'        "{r}",\n')
                f.write("    ),\n")
            f.write("}\n\n")
        f.write(
            "SIZES = {%s}\n\n\n" % ", ".join(f"{s}: ICONS_{s}" for s in SIZES) +
            "def draw(cr, name, x, y, size=16, scale=1):\n"
            '    """Blit an icon at (x, y). The caller sets the colour first —\n'
            "    cr.set_source_rgb(*rgb(PAL['accent2'])) then draw(...) — which\n"
            '    is what keeps these wallust-driven. An unknown size falls back\n'
            '    to the smallest generated one rather than raising."""\n'
            "    art = (SIZES.get(size) or SIZES[min(SIZES)]).get(name)\n"
            "    if not art:\n"
            "        return 0\n"
            "    for ry, row in enumerate(art):\n"
            "        run = 0\n"
            "        for rx, ch in enumerate(row):\n"
            "            if ch == 'X':\n"
            "                run += 1\n"
            "                continue\n"
            "            if run:            # one rect per run, not per pixel\n"
            "                cr.rectangle(x + (rx - run) * scale,\n"
            "                             y + ry * scale, run * scale, scale)\n"
            "                run = 0\n"
            "        if run:\n"
            "            cr.rectangle(x + (len(row) - run) * scale,\n"
            "                         y + ry * scale, run * scale, scale)\n"
            "    cr.fill()\n"
            "    return size * scale\n")
    total = sum(len(v) for v in out.values())
    print(f"wrote {dest}: {total} bitmaps "
          f"({', '.join(f'{len(out.get(s, {}))}@{s}' for s in SIZES)})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
