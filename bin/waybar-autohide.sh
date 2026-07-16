#!/usr/bin/env python3
# ============================================================
#  waybar-autohide.sh — hide waybar; reveal when the mouse
#  touches the top edge of the screen, hide again on leave.
#  (.sh name kept so `pgrep -f waybar-autohide.sh` still works)
#
#  Zero-fork rewrite: the old bash loop spawned a `hyprctl`
#  process ~7x/second; this reads the cursor straight from
#  Hyprland's IPC socket — near-zero CPU.
#
#  Manual toggle while running: send SIGUSR1 to THIS process
#  (bar-toggle.sh does that — bound to ALT+B):
#    - if hidden  → show and PIN (auto-hide paused)
#    - if pinned  → hide and resume auto-hide
#  Stop the daemon: pkill -f waybar-autohide  (bar stays visible)
# ============================================================
import os
import signal
import socket
import subprocess
import sys
import time

INTERVAL = 0.2
BAR_HEIGHT = 34          # a little more than the bar's 30px
SOCK = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000"),
                    "hypr", os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", ""),
                    ".socket.sock")

state = {"visible": True, "pinned": False, "waybar": None}


def waybar_pid():
    pid = state["waybar"]
    if pid is not None:
        try:
            os.kill(pid, 0)
            return pid
        except (ProcessLookupError, PermissionError):
            state["waybar"] = None
    try:
        out = subprocess.run(["pgrep", "-x", "waybar"],
                             capture_output=True, text=True).stdout.split()
        state["waybar"] = int(out[0]) if out else None
    except Exception:
        state["waybar"] = None
    return state["waybar"]


def toggle_bar():
    pid = waybar_pid()
    if pid is not None:
        try:
            os.kill(pid, signal.SIGUSR1)
        except ProcessLookupError:
            state["waybar"] = None


def show():
    if not state["visible"]:
        toggle_bar()
        state["visible"] = True


def hide():
    if state["visible"]:
        toggle_bar()
        state["visible"] = False


def cursor_y():
    """Cursor y position via Hyprland's socket — no subprocess."""
    try:
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(1)
            s.connect(SOCK)
            s.sendall(b"cursorpos")
            data = s.recv(64).decode()
        return int(data.split(",")[1])
    except Exception:
        return None


def on_usr1(_sig, _frm):
    # ALT+B: hidden -> show and pin; visible -> hide and resume auto-hide
    if state["visible"]:
        hide()
        state["pinned"] = False
    else:
        show()
        state["pinned"] = True


def on_term(_sig, _frm):
    show()               # when the daemon is stopped, leave the bar visible
    sys.exit(0)


signal.signal(signal.SIGUSR1, on_usr1)
signal.signal(signal.SIGTERM, on_term)
signal.signal(signal.SIGINT, on_term)

hide()
while True:
    if not state["pinned"]:
        y = cursor_y()
        if y is not None:
            if not state["visible"] and y <= 1:
                show()
            elif state["visible"] and y > BAR_HEIGHT:
                hide()
    time.sleep(INTERVAL)
