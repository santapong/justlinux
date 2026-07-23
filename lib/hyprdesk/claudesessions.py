"""Claude Code session discovery — shared by the launcher picker, the
Claude Studio sidebar and the claude-office scene.

Sessions come from two worlds:
  · live `claude` processes (a /dev/pts stdin = interactive terminal
    session, anything else = background daemon/job)
  · resumable transcripts under ~/.claude/projects/<encoded-cwd>/*.jsonl —
    cwd and preview are read from the jsonl lines themselves, because the
    encoded directory name is ambiguous ('.'/'/' both become '-')
"""
import json
import os
import subprocess
import time
from pathlib import Path

CLAUDE_PROJECTS = Path.home() / ".claude/projects"


def session_meta(path, max_lines=40):
    """(cwd, preview) pulled from a transcript's first lines."""
    cwd, preview = "", ""
    try:
        with open(path, errors="replace") as f:
            for i, line in enumerate(f):
                if i > max_lines or (cwd and preview):
                    break
                try:
                    d = json.loads(line)
                except ValueError:
                    continue
                if not cwd and isinstance(d.get("cwd"), str):
                    cwd = d["cwd"]
                if not preview and d.get("type") == "user":
                    m = (d.get("message") or {}).get("content", "")
                    if isinstance(m, list):
                        m = " ".join(p.get("text", "") for p in m
                                     if isinstance(p, dict))
                    m = " ".join(str(m).split())
                    if m and not m.startswith("<"):
                        preview = m[:48]
    except OSError:
        pass
    return cwd, preview


def claude_procs():
    """[(pid, cwd, interactive)] for live claude CLI processes."""
    out = []
    try:
        pids = subprocess.run(["pgrep", "-x", "claude"], capture_output=True,
                              text=True).stdout.split()
    except Exception:
        pids = []
    for p in pids:
        try:
            cwd = os.readlink(f"/proc/{p}/cwd")
            tty = os.readlink(f"/proc/{p}/fd/0")
            out.append((int(p), cwd, tty.startswith("/dev/pts")))
        except OSError:
            continue
    return out


def proc_cpu_ticks(pid):
    """utime+stime of a pid, or None if it vanished — poll the delta to
    tell a *working* claude from one waiting for input."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            parts = f.read().split()
        return int(parts[13]) + int(parts[14])
    except (OSError, ValueError, IndexError):
        return None


def window_of_pid(pid, clients):
    """Climb the parent chain until a Hyprland client pid matches (the
    terminal window hosting this process)."""
    by_pid = {c.get("pid"): c for c in clients if isinstance(c, dict)}
    for _ in range(8):
        if pid in by_pid:
            return by_pid[pid].get("address", "")
        try:
            with open(f"/proc/{pid}/stat") as f:
                pid = int(f.read().split()[3])
        except (OSError, ValueError, IndexError):
            return ""
        if pid <= 1:
            return ""
    return ""


def ago(ts):
    d = max(0, int(time.time() - ts))
    if d < 3600:
        return f"{d // 60}m ago"
    if d < 86400:
        return f"{d // 3600}h ago"
    return f"{d // 86400}d ago"


def recent_transcripts(limit=25):
    """[(mtime, path)] newest first; workflow/subagent transcripts are
    not resumable conversations and are skipped."""
    files = []
    try:
        for proj in CLAUDE_PROJECTS.iterdir():
            if not proj.is_dir() or "-subagents-" in proj.name:
                continue
            for f in proj.glob("*.jsonl"):
                try:
                    files.append((f.stat().st_mtime, f))
                except OSError:
                    pass
    except OSError:
        pass
    return sorted(files, key=lambda t: t[0], reverse=True)[:limit]


def session_rows(clients=None):
    """Flat entry list for pickers: running first, then bg jobs, then
    resumable transcripts. Same dict shape the launcher's Item expects."""
    if clients is None:
        try:
            clients = json.loads(subprocess.run(
                ["hyprctl", "clients", "-j"],
                capture_output=True, text=True).stdout)
        except Exception:
            clients = []
    home = str(Path.home())

    def nice(p):
        return (p or "?").replace(home, "~") or "~"

    rows = []
    for pid, cwd, interactive in claude_procs():
        if interactive:
            rows.append({"label": nice(cwd), "icon": "󰚩", "kind": "run",
                         "addr": window_of_pid(pid, clients), "cwd": cwd,
                         "detail": "🟢 running — Enter focuses its terminal"})
        else:
            rows.append({"label": nice(cwd), "icon": "󰚩", "kind": "bg",
                         "cwd": cwd,
                         "detail": "󰑮 background job — view on claude.ai"})
    for mtime, f in recent_transcripts():
        cwd, preview = session_meta(f)
        if not preview:
            continue                     # empty/aborted session: skip
        rows.append({"label": nice(cwd), "icon": "󰚩", "kind": "past",
                     "sid": f.stem, "cwd": cwd or home,
                     "detail": f"{ago(mtime)} — {preview}"})
    return rows
