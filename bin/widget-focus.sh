#!/usr/bin/env python3
"""Focus widget: live countdown, plus what the week actually looked like.

focus-mode.sh appends one row per finished session to history.tsv
(start_epoch, elapsed_s, planned_s, label). Until that existed the numbers
were computed and then deleted, so the card could only ever show the
session you were already in."""
import sys, time
from pathlib import Path

STATE = Path.home() / ".local/state/focus-mode/state"
HISTORY = Path.home() / ".local/state/focus-mode/history.tsv"


def week():
    """(total_minutes, 'm0,m1,…m6') for the last 7 days, oldest first."""
    days = [0] * 7
    now = time.time()
    midnight = now - (now % 86400)          # local-ish day buckets are fine
    try:
        for line in HISTORY.read_text().splitlines():
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            try:
                start, elapsed = int(parts[0]), int(parts[1])
            except ValueError:
                continue
            idx = 6 - int((midnight - (start - start % 86400)) // 86400)
            if 0 <= idx < 7:
                days[idx] += elapsed
    except OSError:
        pass                                 # no history yet: all zeros
    mins = [d // 60 for d in days]
    return sum(mins), ",".join(str(m) for m in mins)

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
wk_total, wk_series = week()
print(f"week_total={wk_total // 60}h {wk_total % 60:02d}m" if wk_total >= 60
      else f"week_total={wk_total}m")
print(f"week={wk_series}")
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
