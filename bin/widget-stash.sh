#!/usr/bin/env python3
"""Stash widget: windows parked in special:* workspaces (ALT+A stash,
ALT+SHIFT+A hide, ALT+S scratchpad) — the visible face of "where did my
window go". ALT+H restores. Dropdown terminals are features, not lost
windows, so special:term / special:claude are skipped."""
import json, os, subprocess, sys, time
from pathlib import Path

FIELDS = "--fields" in sys.argv
RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
CACHE = RUN / "widget-stash.fields"
_TTL = 4                                    # host interval 5 — stay fresher
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < _TTL:
    print(CACHE.read_text(), end=""); raise SystemExit

SKIP = {"special:term", "special:claude"}   # dropdown features, not stash

def where_label(ws):
    if ws == "special:magic":
        return "scratchpad"
    if ws == "special:hidden":
        return "hidden"
    if ws.startswith("special:stash"):
        return "stash " + ws[len("special:stash"):]
    return ws.removeprefix("special:")

try:
    clients = json.loads(subprocess.run(
        ["hyprctl", "-j", "clients"], capture_output=True, text=True,
        timeout=2).stdout)
except Exception:
    clients = []

rows = []
for c in clients:
    if not isinstance(c, dict):
        continue
    ws = (c.get("workspace") or {}).get("name", "")
    if not ws.startswith("special:") or ws in SKIP:
        continue
    title = (c.get("title") or "")[:34]
    rows.append((c.get("class") or "?", title, where_label(ws)))

out = [f"count={len(rows)}"]
if rows:
    for i, (cls, title, where) in enumerate(rows[:6]):
        out += [f"w.{i}.name={cls}", f"w.{i}.title={title}",
                f"w.{i}.where={where}"]
    out.append("hint=ALT+H restores · ALT+S scratchpad")
else:
    out += ["w.0.name=nothing hidden", "w.0.title=", "w.0.where=",
            "hint=ALT+A stash · ALT+SHIFT+A hide"]
text = "\n".join(out) + "\n"
CACHE.write_text(text)
print(text, end="")
