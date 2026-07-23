#!/usr/bin/env python3
"""Focus widget: live countdown of the focus-mode.sh session."""
import sys, time
from pathlib import Path

STATE = Path.home() / ".local/state/focus-mode/state"

end = start = 0
label = ""
try:
    for line in STATE.read_text().splitlines():
        k, _, v = line.partition("=")
        if k == "end":
            end = int(v)
        elif k == "start":
            start = int(v)
        elif k == "label":
            label = v
except (OSError, ValueError):
    pass

now = time.time()
if end and now < end:
    remaining = int(end - now)
    total = max(1, end - start)
    pct = 100.0 * (now - start) / total
    print(f"state=focus")
    print(f"remaining={remaining // 60}:{remaining % 60:02d}")
    print(f"pct={pct:.0f}")
    print(f"label={label} — notifications muted")
else:
    print("state=idle")
    print("remaining=idle")
    print("pct=0")
    print("label=ALT+SHIFT+F starts a 25 min session")
