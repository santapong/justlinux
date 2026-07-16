#!/usr/bin/env python3
"""Fake Hyprland IPC socket for benchmarks.

Serves $XDG_RUNTIME_DIR/hypr/$SIG/.socket.sock with canned JSON:
20 windows on workspace 4 (configurable), monitors, cursorpos. Appends
every request to $SOCKLOG (one per line) so runs can count IPC traffic.

Usage: fake_hypr.py <runtime_dir> <signature> <n_windows> [socklog]
"""
import json
import os
import socket
import sys
import threading

runtime, sig, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
socklog = sys.argv[4] if len(sys.argv) > 4 else None

sockdir = os.path.join(runtime, "hypr", sig)
os.makedirs(sockdir, exist_ok=True)
path = os.path.join(sockdir, ".socket.sock")
try:
    os.unlink(path)
except FileNotFoundError:
    pass

clients = [
    {
        "address": f"0x{i:04x}",
        "at": [(i % 5) * 320, (i // 5) * 180],
        "workspace": {"id": 4, "name": "4"},
        "title": f"window {i}",
        "class": "kitty",
    }
    for i in range(n)
]
RESP = {
    "j/activeworkspace": json.dumps({"id": 4, "name": "4", "monitor": "DP-1"}),
    "j/activewindow": json.dumps(clients[0]) if clients else "{}",
    "j/clients": json.dumps(clients),
    "j/monitors": json.dumps(
        [
            {"name": "DP-1", "x": 0, "y": 0, "focused": True},
            {"name": "HDMI-A-1", "x": 1600, "y": 0, "focused": False},
        ]
    ),
    "cursorpos": "512, 300",
}

srv = socket.socket(socket.AF_UNIX)
srv.bind(path)
srv.listen(64)
print("READY", flush=True)

lock = threading.Lock()


def handle(conn):
    try:
        req = conn.recv(4096).decode()
        if socklog:
            with lock, open(socklog, "a") as f:
                f.write(req + "\n")
        conn.sendall(RESP.get(req, "ok").encode())
    except OSError:
        pass
    finally:
        conn.close()


while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
