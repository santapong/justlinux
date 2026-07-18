#!/usr/bin/env python3
"""Robotics / embedded bench widget.
Serial boards with FREE/holder badges, docker health-first radiator, and
SocketCAN controller states. No extra packages — sysfs + CLI only."""
import json, os, subprocess, time
from pathlib import Path

CACHE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "widget-robotics.cache"
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < 10:
    print(CACHE.read_text(), end=""); raise SystemExit


def esc(s):
    return str(s).replace("$", "$$")


def port_holder(dev):
    """Name of the same-user process holding a tty, or None (== FREE)."""
    me = os.getuid()
    try:
        target = os.stat(dev).st_rdev
    except OSError:
        return None
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if proc.stat().st_uid != me:
                continue
            for fd in (proc / "fd").iterdir():
                try:
                    if fd.stat().st_rdev == target:
                        return (proc / "comm").read_text().strip()[:12]
                except OSError:
                    continue
        except OSError:
            continue
    return None


rows = ["${color1}󰚩  ROBOTICS BENCH${color}", "${color3}${hr}${color}"]

# --- serial boards ---
byid = Path("/dev/serial/by-id")
ports = sorted(byid.iterdir()) if byid.is_dir() else []
if ports:
    for p in ports[:4]:
        dev = os.path.realpath(p)
        pretty = esc(p.name.replace("usb-", "").split("-if")[0][:22])
        holder = port_holder(dev)
        badge = (f"${{color5}}◉ {esc(holder)}${{color}}" if holder
                 else "${color4}○ free${color}")
        rows.append(f"${{color2}}{Path(dev).name}${{color}} "
                    f"${{color3}}{pretty}${{color}}${{alignr}}{badge}")
else:
    rows.append("${color3}󰌘 no serial boards plugged in${color}")

# --- docker: exceptions first, healthy collapsed ---
try:
    r = subprocess.run(["docker", "ps", "-a", "--format",
                        "{{.Names}}\t{{.State}}\t{{.Status}}"],
                       capture_output=True, text=True, timeout=4)
    lines = [l.split("\t") for l in r.stdout.splitlines() if l.strip()] \
        if r.returncode == 0 else None
except Exception:
    lines = None
if lines is None:
    rows.append("${color3}󰡨 docker daemon stopped${color}")
else:
    bad = [l for l in lines if "unhealthy" in l[2].lower()
           or l[1].lower() == "restarting"]
    up = [l for l in lines if l[1].lower() == "running" and l not in bad]
    for name, _state, status in bad[:3]:
        rows.append(f"${{color5}}󰡨 {esc(name)[:16]}${{color}}${{alignr}}"
                    f"${{color5}}{esc(status)[:14]}${{color}}")
    if up or not bad:
        exited = len(lines) - len(up) - len(bad)
        summary = f"{len(up)} up" + (f" · {exited} stopped" if exited else "")
        tone = "color4" if up else "color3"
        rows.append(f"${{color3}}󰡨 docker${{color}}${{alignr}}"
                    f"${{{tone}}}{summary or 'idle'}${{color}}")

# --- CAN controller states ---
try:
    r = subprocess.run(["ip", "-json", "-details", "link", "show", "type",
                        "can"], capture_output=True, text=True, timeout=3)
    cans = json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else []
    # some iproute2 builds ignore the type filter — keep only real CAN links
    cans = [c for c in cans
            if ((c.get("linkinfo") or {}).get("info_kind") == "can"
                or str(c.get("ifname", "")).startswith(("can", "vcan")))]
except Exception:
    cans = []
for c in cans[:3]:
    info = (c.get("linkinfo") or {}).get("info_data") or {}
    state = info.get("state", c.get("operstate", "?"))
    bitrate = (info.get("bittiming") or {}).get("bitrate")
    tone = {"ERROR-ACTIVE": "color4", "ERROR-PASSIVE": "color1",
            "BUS-OFF": "color5"}.get(state, "color3")
    extra = f" {bitrate // 1000}k" if bitrate else ""
    rows.append(f"${{color3}}󰇺 {esc(c.get('ifname', 'can?'))}{extra}"
                f"${{color}}${{alignr}}${{{tone}}}{esc(state)}${{color}}")

text = "\n".join(rows)
CACHE.write_text(text)
print(text)
