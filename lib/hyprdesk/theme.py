"""Central config + theme resolution for all python desktop widgets."""
import re
from pathlib import Path

HOME = Path.home()
WIDGETS_CONF = HOME / ".config/conky/widgets.conf"
WALLUST_COLORS = HOME / ".config/conky/colors.lua"
THEMES_DIR = HOME / ".config/conky/themes"

FALLBACK = {"bg": "#101017", "fg": "#FEFAD7", "accent": "#F2E3EF",
            "accent2": "#FCF18E", "muted": "#383940"}


def conf():
    """widgets.conf as a dict — the single settings file for the fleet."""
    d = {}
    try:
        for line in WIDGETS_CONF.read_text().splitlines():
            m = re.match(r"^(\w+)\s*=\s*(\S+)", line)
            if m:
                d[m.group(1)] = m.group(2)
    except OSError:
        pass
    return d


def conf_get(key, default=""):
    return conf().get(key, default)


def colors():
    """Active palette: theme=wallust follows the wallpaper, any other
    name loads ~/.config/conky/themes/<name>.lua."""
    src = WALLUST_COLORS
    theme = conf_get("theme", "wallust")
    if theme != "wallust":
        themed = THEMES_DIR / f"{theme}.lua"
        if themed.is_file():
            src = themed
    pal = dict(FALLBACK)
    try:
        for k, v in re.findall(r'(\w+)\s*=\s*"(#[0-9a-fA-F]{6})"',
                               src.read_text()):
            pal[k] = v
    except OSError:
        pass
    # derived ink hierarchy — keep in sync with card.lua M.colors()
    pal.setdefault("sub", _mix(pal["fg"], pal["bg"], 0.62))
    pal.setdefault("good", "#8EC07C")
    pal.setdefault("bad", "#E06C75")
    return pal


def _mix(a, b, t):
    ar, ag, ab = (int(a[i:i + 2], 16) for i in (1, 3, 5))
    br, bg, bb = (int(b[i:i + 2], 16) for i in (1, 3, 5))
    return "#%02X%02X%02X" % (round(ar * t + br * (1 - t)),
                              round(ag * t + bg * (1 - t)),
                              round(ab * t + bb * (1 - t)))


def rgb(hexstr):
    """'#RRGGBB' -> (r, g, b) floats 0..1 for cairo."""
    return tuple(int(hexstr[i:i + 2], 16) / 255 for i in (1, 3, 5))


def css_rgba(hexstr, alpha):
    """'#RRGGBB' -> 'rgba(r,g,b,a)' for GTK CSS."""
    r, g, b = (int(hexstr[i:i + 2], 16) for i in (1, 3, 5))
    return f"rgba({r},{g},{b},{alpha})"
