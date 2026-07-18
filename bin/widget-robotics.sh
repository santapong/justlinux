#!/usr/bin/env python3
"""Robotics / embedded dev widget: USB serial boards (hotplug-aware),
docker containers, and CAN interfaces. No extra packages — sysfs + CLI."""
import os, subprocess, time
from pathlib import Path

CACHE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "widget-robotics.cache"
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < 10:
    print(CACHE.read_text(), end=""); raise SystemExit

rows = ["${color1}󰚩  ROBOTICS BENCH${color}", "${color3}${hr}${color}"]

# --- serial boards: /dev/serial/by-id gives human names ---
byid = Path("/dev/serial/by-id")
ports = sorted(byid.iterdir()) if byid.is_dir() else []
if ports:
    for p in ports[:4]:
        dev = os.path.realpath(p)
        name = p.name.replace("usb-", "").split("-if")[0][:26]
        name = name.replace("$", "$$")
        rows.append(f"${{color4}}󱐋${{color}} ${{color2}}"
                    f"{Path(dev).name}${{color}}${{alignr}}"
                    f"${{color3}}{name}${{color}}")
else:
    rows.append("${color3}󰌘 no serial boards plugged in${color}")

# --- docker containers ---
try:
    out = subprocess.run(["docker", "ps", "--format",
                          "{{.Names}}\t{{.Status}}"],
                         capture_output=True, text=True, timeout=4)
    lines = [l for l in out.stdout.splitlines() if l.strip()] \
        if out.returncode == 0 else None
except Exception:
    lines = None
if lines is None:
    rows.append("${color3}󰡨 docker unavailable${color}")
elif lines:
    for l in lines[:4]:
        name, _, status = l.partition("\t")
        up = status.lower().startswith("up")
        c = "color4" if up else "color5"
        rows.append(f"${{{c}}}󰡨${{color}} ${{color2}}{name[:18]}"
                    f"${{color}}${{alignr}}${{color3}}"
                    f"{status.replace('$', '$$')[:16]}${{color}}")
else:
    rows.append("${color3}󰡨 no containers running${color}")

# --- CAN interfaces (SocketCAN) ---
can = [i.name for i in Path("/sys/class/net").iterdir()
       if i.name.startswith(("can", "vcan"))] \
    if Path("/sys/class/net").is_dir() else []
if can:
    rows.append(f"${{color4}}󰇺 CAN${{color}}${{alignr}}"
                f"${{color2}}{', '.join(can[:3])}${{color}}")

text = "\n".join(rows)
CACHE.write_text(text)
print(text)
