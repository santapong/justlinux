#!/usr/bin/env python3
"""Contrast check for the generated palettes — the guard on the bug that
kept coming back.

wallust maps whatever is in the wallpaper into ansi slots named `red`,
`yellow`, `magenta`… The names are decorative: @red has resolved to a dark
teal measuring 1.00:1 against the background, i.e. invisible. That shipped
in three places (the bar's power button and firewall warning, the Habitica
board, the Settings error line) before anyone noticed, because nothing ever
checked.

This checks. It reads the generated CSS, works out which colours are used
as TEXT, and measures each against the background.

    check-contrast.sh            report, exit 1 if anything used as text fails
    check-contrast.sh --quiet    exit code only (for hooks)
    check-contrast.sh --notify   desktop notification when something fails

WCAG AA is 4.5:1 for body text and 3:1 for large text. Below 3:1 is treated
as a failure outright; 3-4.5 is reported as marginal.
"""
import re
import subprocess
import sys
from pathlib import Path

HOME = Path.home()
TARGETS = [
    (HOME / ".config/waybar/colors.css", HOME / ".config/waybar/style.css"),
    (HOME / ".config/swaync/colors.css", HOME / ".config/swaync/style.css"),
]
FAIL, MARGINAL = 3.0, 4.5


def lum(hexcol):
    h = hexcol.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))

    def f(c):
        return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)


def ratio(a, b):
    la, lb = lum(a), lum(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def defined(css_text):
    return dict(re.findall(r"@define-color\s+([\w-]+)\s+(#[0-9a-fA-F]{6})",
                           css_text))


def text_pairs(style_text):
    """[(fg_slot, bg_slot_or_None, selector)] for every rule that sets a
    text colour.

    Measuring every `color:` against @base is wrong and produced a false
    alarm: `background: @status-bad; color: @base;` is dark-on-red, which
    is perfectly readable — its ground is the status colour, not the page.
    So each rule is judged against ITS OWN background where it declares one.
    """
    out = []
    for m in re.finditer(r"([^{}]+)\{([^{}]*)\}", style_text):
        sel, body = m.group(1).strip(), m.group(2)
        fg = re.search(r"(?<!-)\bcolor\s*:\s*[^;]*?@([\w-]+)", body)
        if not fg:
            continue
        bg = re.search(r"\bbackground(?:-color)?\s*:\s*[^;]*?@([\w-]+)", body)
        out.append((fg.group(1), bg.group(1) if bg else None,
                    " ".join(sel.split())[:40]))
    return out


def main():
    quiet = "--quiet" in sys.argv
    notify = "--notify" in sys.argv
    problems = []
    for colors_css, style_css in TARGETS:
        try:
            C = defined(colors_css.read_text())
            style = style_css.read_text()
        except OSError:
            continue
        base = C.get("base") or C.get("background")
        if not base:
            continue
        if not quiet:
            print(f"\n{colors_css.parent.name}  (page ground {base})")
        seen = set()
        for fg, bg, sel in text_pairs(style):
            col, ground = C.get(fg), C.get(bg) if bg else base
            if not col or not ground:
                continue
            key = (fg, bg)
            if key in seen:
                continue
            seen.add(key)
            r = ratio(col, ground)
            tag = ("FAIL" if r < FAIL else
                   "marginal" if r < MARGINAL else "ok")
            if r < FAIL:
                problems.append(f"{colors_css.parent.name}: @{fg} on "
                                f"@{bg or 'base'} = {r:.2f}:1  ({sel})")
            if not quiet:
                on = f" on @{bg}" if bg else ""
                print(f"  @{fg}{on:<14} {col}  {r:5.2f}:1  {tag}")
    if problems:
        msg = ("Unreadable colours in the generated palette:\n  "
               + "\n  ".join(problems)
               + "\n\nThese are wallust ansi slots used as text. Point them "
                 "at @text/@subtext or the pinned @status-* trio.")
        if not quiet:
            print("\n" + msg)
        if notify:
            subprocess.run(["notify-send", "-u", "critical",
                            "Palette contrast", msg[:400]],
                           capture_output=True)
        return 1
    if not quiet:
        print("\nEvery colour used as text is readable against its ground.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
