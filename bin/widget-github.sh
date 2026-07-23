#!/usr/bin/env python3
"""GitHub contributions heatmap as a conky block grid.
User from widgets.conf github_user (default santapong). Cache 1 h."""
import os, re, sys, time, urllib.request
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".local/lib"))
from hyprdesk import conf_get

FIELDS = "--fields" in sys.argv
if "--user" in sys.argv:
    _i = sys.argv.index("--user")
    USER = sys.argv[_i + 1] if len(sys.argv) > _i + 1 else "santapong"
else:
    USER = conf_get("github_user", "santapong")
WEEKS = 16
import hashlib
_tag = hashlib.md5(USER.encode()).hexdigest()[:8]
RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
CACHE = RUN / (f"widget-github-{_tag}.fields" if FIELDS
               else "widget-github.cache")
# fields TTL < host interval (3600) — equal TTL halves the refresh rate
_TTL = 3300 if FIELDS else 3600
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < _TTL:
    print(CACHE.read_text(), end=""); raise SystemExit

SHADE = {0: "${color3}·${color}", 1: "${color3}▪${color}",
         2: "${color2}▪${color}", 3: "${color2}■${color}",
         4: "${color1}■${color}"}

try:
    url = f"https://github.com/users/{USER}/contributions"
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=8) as r:
        html = r.read().decode()
    # cells carry data-date + data-level; tooltips carry the counts
    cells = re.findall(r'data-date="(\d{4}-\d{2}-\d{2})"[^>]*data-level="(\d)"',
                       html)
    if not cells:
        cells = [(m.group(2), m.group(1)) for m in
                 re.finditer(r'data-level="(\d)"[^>]*data-date="(\d{4}-\d{2}-\d{2})"', html)]
        cells = [(d, l) for d, l in cells]
    cells.sort()
    m = re.search(r'([\d,]+)\s+contributions', html)
    count = m.group(1) if m else "?"
    days = cells[-WEEKS * 7:]
    if FIELDS:
        text = "\n".join([
            f"user={USER}",
            f"count={count}",
            "levels=" + ",".join(str(min(4, int(lv))) for _d, lv in days),
        ])
        CACHE.write_text(text)
        print(text)
        raise SystemExit
    disp = USER if len(USER) <= 16 else USER[:15] + "…"
    rows = [f"${{color1}}  {disp}${{color}}${{alignr}}"
            f"${{color2}}{count} this year${{color}}",
            "${color3}${hr}${color}"]
    for dow in range(7):
        line = "".join(SHADE[min(4, int(lv))] + " "
                       for _d, lv in days[dow::7])
        rows.append(line.rstrip())
    text = "\n".join(rows)
    CACHE.write_text(text)
except SystemExit:
    raise
except Exception:
    if FIELDS:
        sys.exit(1)          # host keeps last fields + shows (stale)
    text = (CACHE.read_text() + "\n${color3}(stale)${color}") \
        if CACHE.exists() else "${color3} github unreachable${color}"
print(text)
