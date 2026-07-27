#!/usr/bin/env python3
"""Robotics / embedded bench widget.
Serial boards with FREE/holder badges, docker health-first radiator, and
SocketCAN controller states. No extra packages — sysfs + CLI only.

Default: conky markup (legacy twin). --fields: generic line rows
(ln.N.key / ln.N.val / ln.N.slot) for hypr-cardhost. The bench state is
computed ONCE as tuples so the two output modes can never drift."""
import json, os, subprocess, sys, time
from pathlib import Path

FIELDS = "--fields" in sys.argv
RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
CACHE = RUN / ("widget-robotics.fields" if FIELDS
               else "widget-robotics.cache")
# fields TTL < host interval (10) — equal TTL halves the refresh rate
_TTL = 8 if FIELDS else 10
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < _TTL:
    print(CACHE.read_text(), end=""); raise SystemExit


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


lines = []          # (key, val, slot[, markup_key]) — the optional 4th
                    # element preserves the twin's exact per-segment inks
                    # in markup mode (rollback parity); fields mode
                    # ignores it

# --- serial boards ---
byid = Path("/dev/serial/by-id")
ports = sorted(byid.iterdir()) if byid.is_dir() else []
if ports:
    for p in ports[:4]:
        dev = os.path.realpath(p)
        pretty = p.name.replace("usb-", "").split("-if")[0][:22]
        holder = port_holder(dev)
        mk = (f"${{color2}}{Path(dev).name}${{color}} "
              f"${{color3}}{pretty}${{color}}")     # twin: fg dev + sub desc
        if holder:
            lines.append((f"{Path(dev).name} · {pretty}",
                          f"◉ {holder}", "bad", mk))
        else:
            lines.append((f"{Path(dev).name} · {pretty}", "○ free", "good",
                          mk))
else:
    lines.append(("󰌘 no serial boards plugged in", "", "sub"))

# --- docker: exceptions first, healthy collapsed ---
try:
    r = subprocess.run(["docker", "ps", "-a", "--format",
                        "{{.Names}}\t{{.State}}\t{{.Status}}"],
                       capture_output=True, text=True, timeout=4)
    dockers = [l.split("\t") for l in r.stdout.splitlines() if l.strip()] \
        if r.returncode == 0 else None
except Exception:
    dockers = None
if dockers is None:
    lines.append(("󰡨 docker daemon stopped", "", "sub"))
else:
    bad = [l for l in dockers if "unhealthy" in l[2].lower()
           or l[1].lower() == "restarting"]
    up = [l for l in dockers if l[1].lower() == "running" and l not in bad]
    for name, _state, status in bad[:3]:
        lines.append((f"󰡨 {name[:16]}", status[:14], "bad",
                      f"${{color5}}󰡨 {name[:16]}${{color}}"))  # twin: bad key
    # NAME what is running. "0 up · 12 stopped" is a true sentence that
    # tells you nothing — on this bench ROS lives in containers, so which
    # ones are up IS the bench state.
    for name, _state, status in up[:3]:
        lines.append((f"󰡨 {name[:16]}", status[:14] or "up", "good"))
    if not up and not bad:
        stopped = len(dockers) - len(up) - len(bad)
        lines.append(("󰡨 docker", f"idle · {stopped} stopped"
                      if stopped else "idle", "sub"))
    elif len(up) > 3:
        lines.append(("󰡨 docker", f"+{len(up) - 3} more up", "sub"))

# --- ROS 2 bench: the images the ros2-* wrappers launch ---
# There is no /opt/ros on this host; the bench is containerised, so the
# honest question is "which bench images exist and is one live", not
# "is ROS installed".
try:
    r = subprocess.run(["docker", "images", "--format",
                        "{{.Repository}}:{{.Tag}}"],
                       capture_output=True, text=True, timeout=4)
    imgs = [i for i in r.stdout.split() if i.startswith("ros2-")] \
        if r.returncode == 0 else []
except Exception:
    imgs = []
if imgs:
    live = [l[0] for l in (up if dockers else [])]
    lines.append(("󰚩 ros2 bench",
                  f"{len(imgs)} images" + (" · running" if live else " · idle"),
                  "good" if live else "sub"))

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
    slot = {"ERROR-ACTIVE": "good", "ERROR-PASSIVE": "accent2",
            "BUS-OFF": "bad"}.get(state, "sub")
    extra = f" {bitrate // 1000}k" if bitrate else ""
    lines.append((f"󰇺 {c.get('ifname', 'can?')}{extra}", str(state), slot))

# --- emit ---
if FIELDS:
    out = []
    for i, ln in enumerate(lines):
        key, val, slot = ln[0], ln[1], ln[2]
        out += [f"ln.{i}.key={key}", f"ln.{i}.val={val}",
                f"ln.{i}.slot={slot}"]
    text = "\n".join(out)
else:
    COLOR = {"good": "color4", "bad": "color5", "sub": "color3",
             "accent2": "color1"}

    def esc(s):
        return str(s).replace("$", "$$")

    rows = ["${color1}󰚩  ROBOTICS BENCH${color}", "${color3}${hr}${color}"]
    for ln in lines:
        key, val, slot = ln[0], ln[1], ln[2]
        mk = ln[3] if len(ln) > 3 else None
        c = COLOR[slot]
        left = mk if mk else f"${{color3}}{esc(key)}${{color}}"
        if val:
            rows.append(f"{left}${{alignr}}${{{c}}}{esc(val)}${{color}}")
        else:
            rows.append(f"${{{c}}}{esc(key)}${{color}}" if not mk else left)
    text = "\n".join(rows)

CACHE.write_text(text)
print(text)
