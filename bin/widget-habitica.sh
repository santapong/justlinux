#!/usr/bin/env python3
"""Habitica card: dailies done/due, streaks, HP/XP/gold. Cache 5 min.
Credentials: ~/.config/hyprdesk/habitica.conf with two lines:
    user=<your Habitica User ID>
    token=<your API Token>
(find both at habitica.com → Settings → Site Data)"""
import json, os, time, urllib.request
from pathlib import Path

CREDS = Path.home() / ".config/hyprdesk/habitica.conf"
CACHE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "widget-habitica.cache"
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < 300:
    print(CACHE.read_text(), end=""); raise SystemExit


def creds():
    d = {}
    try:
        for line in CREDS.read_text().splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                d[k.strip()] = v.strip()
    except OSError:
        pass
    return d.get("user"), d.get("token")


user, token = creds()
if not user or not token:
    print("${color1}󱇧 HABITICA${color}\n${color3}add credentials to\n"
          "~/.config/hyprdesk/habitica.conf\n(user= and token= — see "
          "Settings → Site Data)${color}")
    raise SystemExit


def get(path):
    req = urllib.request.Request(
        f"https://habitica.com/api/v3{path}",
        headers={"x-api-user": user, "x-api-key": token,
                 "x-client": f"{user}-hyprdesk-widget"})
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.loads(r.read())["data"]


try:
    u = get("/user?userFields=stats")["stats"]
    dailies = get("/tasks/user?type=dailys")
    due = [t for t in dailies if t.get("isDue")]
    done = [t for t in due if t.get("completed")]
    best = max((t.get("streak", 0) for t in dailies), default=0)
    hp_pct = max(0, min(10, round(u["hp"] / u["maxHealth"] * 10)))
    xp_pct = max(0, min(10, round(u["exp"] / max(1, u["toNextLevel"]) * 10)))
    rows = [
        "${color1}󱇧  HABITICA${color}${alignr}${color3}lvl "
        + str(u["lvl"]) + "${color}",
        "${color3}${hr}${color}",
        f"${{color3}}dailies${{color}}${{alignr}}${{color2}}{len(done)}/{len(due)} done${{color}}",
        f"${{color3}}best streak${{color}}${{alignr}}${{color2}}󰈸 {best}${{color}}",
        f"${{color1}}hp${{color}} {'█' * hp_pct}{'░' * (10 - hp_pct)}"
        f"${{alignr}}{round(u['hp'])}/{round(u['maxHealth'])}",
        f"${{color2}}xp${{color}} {'█' * xp_pct}{'░' * (10 - xp_pct)}"
        f"${{alignr}}{round(u['exp'])}/{u['toNextLevel']}",
        f"${{color3}}gold${{color}}${{alignr}}${{color2}}"
        f"{round(u['gp'])} 󰆼${{color}}",
    ]
    text = "\n".join(rows)
    CACHE.write_text(text)
except Exception:
    text = CACHE.read_text() if CACHE.exists() else \
        "${color3}󱇧 habitica unreachable${color}"
print(text)
