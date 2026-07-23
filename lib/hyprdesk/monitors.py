"""Map Gdk monitors ↔ Hyprland connector names.

Gdk knows geometry but not connector names; hyprctl knows both. Monitors
are matched on their (x, y) origin — the three identical 1600x900 heads
here differ only by offset. Falls back to hyprctl's list order, then to
a synthetic name, when geometry is ambiguous or hyprctl is unavailable.
"""
import json
import subprocess

from .confwrite import sanitize_key_fragment as sanitize   # re-export


def hypr_monitors():
    """hyprctl -j monitors as a list of dicts ([] on any failure)."""
    try:
        out = subprocess.run(["hyprctl", "-j", "monitors"],
                             capture_output=True, text=True, timeout=3).stdout
        mons = json.loads(out)
        return mons if isinstance(mons, list) else []
    except Exception:
        return []


def connector_of(gdk_monitor, hypr=None):
    """Best-effort connector name ('DP-1') for a GdkMonitor, else None."""
    geo = gdk_monitor.get_geometry()
    for m in (hypr_monitors() if hypr is None else hypr):
        if m.get("x") == geo.x and m.get("y") == geo.y:
            return m.get("name")
    return None


def gdk_monitor_map(display):
    """[(gdk_monitor, connector, sanitized)] for every connected monitor.

    Always returns one entry per Gdk monitor; the connector name degrades
    gracefully (geometry match → hyprctl list order → 'MON<i>').
    """
    hypr = hypr_monitors()
    out = []
    for i in range(display.get_n_monitors()):
        mon = display.get_monitor(i)
        name = (connector_of(mon, hypr)
                or (hypr[i].get("name") if i < len(hypr) else None)
                or f"MON{i}")
        out.append((mon, name, sanitize(name)))
    return out
