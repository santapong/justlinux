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
from datetime import datetime
from pathlib import Path

CLAUDE_PROJECTS = Path.home() / ".claude/projects"
CLAUDE_JOBS = Path.home() / ".claude/jobs"
CLAUDE_ROSTER = Path.home() / ".claude/daemon/roster.json"

_SESSION_META_CACHE = {}       # (path, st_mtime) -> (cwd, preview)
_SESSION_META_CAP = 200        # bounded: transcripts are append-only so a
                                # cached parse of an unchanged file never
                                # goes stale — this just caps memory


def session_meta(path, max_lines=40):
    """(cwd, preview) pulled from a transcript's first lines. Cached on
    (path, mtime) — U13 (Claude Studio sidebar) calls this per-frame."""
    try:
        mtime = os.stat(path).st_mtime
    except OSError:
        return "", ""
    key = (str(path), mtime, max_lines)
    hit = _SESSION_META_CACHE.get(key)
    if hit is not None:
        return hit

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

    if len(_SESSION_META_CACHE) >= _SESSION_META_CAP:
        _SESSION_META_CACHE.pop(next(iter(_SESSION_META_CACHE)))
    _SESSION_META_CACHE[key] = (cwd, preview)
    return cwd, preview


_TITLE_CACHE = {}              # (path, st_mtime) -> str


def session_title(path, tail_bytes=65536):
    """Claude's own name for a conversation — the newest `aiTitle` in the
    transcript ("Debug Claude Science launch issue"). Titles are rewritten
    as a session evolves, so the LAST one wins; the tail is scanned first
    because that is where a fresh one lands. "" when absent."""
    try:
        st = os.stat(path)
    except OSError:
        return ""
    key = (str(path), st.st_mtime)
    hit = _TITLE_CACHE.get(key)
    if hit is not None:
        return hit

    title = ""
    try:
        with open(path, "rb") as f:
            if st.st_size > tail_bytes:
                f.seek(-tail_bytes, os.SEEK_END)
                f.readline()          # drop the partial first line
            chunk = f.read().decode(errors="replace").splitlines()
        for line in reversed(chunk):          # newest first
            if '"aiTitle"' not in line:
                continue
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if isinstance(d, dict) and isinstance(d.get("aiTitle"), str):
                title = d["aiTitle"].strip()
                break
        if not title and st.st_size > tail_bytes:
            # long session whose only title was written near the start
            with open(path, errors="replace") as f:
                for i, line in enumerate(f):
                    if i > 400:
                        break
                    if '"aiTitle"' not in line:
                        continue
                    try:
                        d = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(d, dict) and isinstance(d.get("aiTitle"),
                                                          str):
                        title = d["aiTitle"].strip()
    except OSError:
        return ""

    if len(_TITLE_CACHE) >= _SESSION_META_CAP:
        _TITLE_CACHE.pop(next(iter(_TITLE_CACHE)))
    _TITLE_CACHE[key] = title
    return title


_JOB_STATE_CACHE = {}          # (path, st_mtime) -> dict


def job_state(sid):
    """state/detail/tempo/inFlight/name for a job, read from
    ~/.claude/jobs/<sid[:8]>/state.json. Undocumented internal — shape
    has already drifted across cliVersions on this box (inFlight/name/
    tokens are missing on older jobs), so every field access is
    best-effort and any parse failure just degrades to {}."""
    path = CLAUDE_JOBS / str(sid)[:8] / "state.json"
    try:
        mtime = path.stat().st_mtime
    except OSError:
        return {}
    key = (str(path), mtime)
    hit = _JOB_STATE_CACHE.get(key)
    if hit is not None:
        return hit

    try:
        d = json.loads(path.read_text())
        if not isinstance(d, dict):
            return {}
        out = {
            "state": d.get("state", ""),
            "detail": d.get("detail", ""),
            "tempo": d.get("tempo", ""),
            "inFlight": d.get("inFlight") or {},
            "name": d.get("name", ""),
            "mtime": mtime,
        }
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return {}

    if len(_JOB_STATE_CACHE) >= _SESSION_META_CAP:
        _JOB_STATE_CACHE.pop(next(iter(_JOB_STATE_CACHE)))
    _JOB_STATE_CACHE[key] = out
    return out


def daemon_roster():
    """[(pid, sid, cwd, transcript, name)] for daemon-hosted workers,
    read from ~/.claude/daemon/roster.json (mode 0600 — unreadable is
    a normal outcome, not an error). Undocumented internal: degrade to
    [] on any shape surprise."""
    out = []
    try:
        d = json.loads(CLAUDE_ROSTER.read_text())
        if not isinstance(d, dict):
            return []
        for short, w in (d.get("workers") or {}).items():
            if not isinstance(w, dict):
                continue
            dispatch = w.get("dispatch") or {}
            launch = dispatch.get("launch") or {}
            seed = dispatch.get("seed") or {}
            out.append((
                w.get("pid"),
                w.get("sessionId", short),
                w.get("cwd", ""),
                launch.get("transcriptPath", ""),
                seed.get("name", ""),
            ))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return []
    return out


def search_transcripts(query, limit=40, ctx=90):
    """Sessions whose CONVERSATION contains `query`, newest first.

    The pickers only ever reached recent_transcripts(25), so anything older
    than about a day was unreachable — you re-ask a question you already
    answered because the answer is not findable. This greps the whole
    corpus instead (~280 MB, ~820 files); ripgrep does that in well under a
    second, which is why the search can re-run on every keystroke.

    Returns [{sid, cwd, title, snippet, mtime, path}]. Falls back to a
    python scan when rg is absent — slower, same answers.
    """
    q = (query or "").strip()
    if len(q) < 2:
        return []
    hits = {}                      # path -> first matching line

    def note(path, line):
        if path not in hits and len(hits) < limit * 3:
            hits[path] = line

    try:
        r = subprocess.run(
            ["rg", "--no-messages", "--no-heading", "--with-filename",
             "--max-count", "1", "--fixed-strings", "--ignore-case",
             "--glob", "*.jsonl", "--", q, str(CLAUDE_PROJECTS)],
            capture_output=True, text=True, timeout=8)
        for out in r.stdout.splitlines():
            path, _, line = out.partition(":")
            if path.endswith(".jsonl"):
                note(path, line)
    except (OSError, subprocess.SubprocessError):
        low = q.lower()
        for _mt, f in recent_transcripts(limit=400):
            try:
                with open(f, errors="replace") as fh:
                    for line in fh:
                        if low in line.lower():
                            note(str(f), line)
                            break
            except OSError:
                continue

    rows = []
    for path, line in hits.items():
        p = Path(path)
        if "subagents" in p.parts:          # agent logs are not conversations
            continue
        try:
            mtime = p.stat().st_mtime
        except OSError:
            continue
        cwd, preview = session_meta(p)
        rows.append({
            "sid": p.stem,
            "cwd": cwd or str(Path.home()),
            "title": session_title(p) or preview or p.stem[:8],
            "snippet": _snippet(line, q, ctx),
            "mtime": mtime,
            "path": str(p),
        })
    rows.sort(key=lambda r: r["mtime"], reverse=True)
    return rows[:limit]


def _snippet(raw, q, ctx=90):
    """The matching text with its surroundings, as plain a string as we can
    make it — a transcript line is JSON, so the raw line is mostly noise."""
    text = raw
    try:
        d = json.loads(raw)
        msg = (d.get("message") or {}).get("content")
        if isinstance(msg, list):
            parts = []
            for p in msg:
                if not isinstance(p, dict):
                    continue
                if p.get("type") == "text":
                    parts.append(p.get("text", ""))
                elif p.get("type") == "tool_use":
                    parts.append(str(p.get("input", ""))[:200])
            text = " ".join(parts) or raw
        elif isinstance(msg, str):
            text = msg
    except ValueError:
        pass
    text = " ".join(str(text).split())
    i = text.lower().find(q.lower())
    if i < 0:
        return text[:ctx]
    a = max(0, i - ctx // 3)
    out = text[a:a + ctx]
    return ("…" if a else "") + out + ("…" if a + ctx < len(text) else "")


CLAUDE_INFRA = ("daemon", "bg-pty-host")


def _argv(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().decode(errors="replace").split("\0")
    except OSError:
        return []


def _pids_named(name):
    """PIDs whose comm is exactly `name`, read straight from /proc.

    This replaced `pgrep -x`, which cost a fork+exec every call — ~18ms,
    and the office calls it twice a second, which made it the single
    largest CPU consumer in the whole widget fleet. Reading /proc costs
    no process at all.
    """
    out = []
    try:
        for entry in os.scandir("/proc"):
            if not entry.name.isdigit():
                continue
            try:
                with open(f"/proc/{entry.name}/comm") as f:
                    if f.read().rstrip("\n") == name:
                        out.append(entry.name)
            except OSError:
                continue          # died between scandir and open
    except OSError:
        pass
    return out


def claude_procs():
    """[(pid, cwd, interactive, tty)] for live claude CLI sessions.
    Claude Code's own plumbing — the supervisor (`claude daemon run`)
    and its bg-pty-host / --bg-spare workers — is skipped: the daemon
    is always alive, so listing it painted a phantom everlasting
    background job in every picker."""
    out = []
    pids = _pids_named("claude")
    for p in pids:
        argv = _argv(p)
        if any(a in CLAUDE_INFRA for a in argv[1:3]) or "--bg-spare" in argv:
            continue
        try:
            cwd = os.readlink(f"/proc/{p}/cwd")
            tty = os.readlink(f"/proc/{p}/fd/0")
            out.append((int(p), cwd, tty.startswith("/dev/pts"), tty))
        except OSError:
            continue
    return out


def daemon_hosted():
    """[(pid, sid, cwd)] sessions the Claude Code daemon keeps alive in
    bg-pty-host workers. They survive their terminal being closed —
    close a studio tab or a kitty window and the session lives on until
    finished or killed, which reads as a task running non-stop."""
    out = []
    # cmdline scan rather than `pgrep -f`: same reason as _pids_named — no
    # fork, and we already have to read every cmdline below anyway
    pids = []
    try:
        for entry in os.scandir("/proc"):
            if entry.name.isdigit() and "bg-pty-host" in " ".join(_argv(entry.name)):
                pids.append(entry.name)
    except OSError:
        pass
    for p in pids:
        argv = _argv(p)
        if "--session-id" not in argv:
            continue                    # pre-warmed spare, not a session
        i = argv.index("--session-id")
        if i + 1 >= len(argv):
            continue
        try:
            out.append((int(p), argv[i + 1], os.readlink(f"/proc/{p}/cwd")))
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


def _sid_key(text):
    """Normalize a session id (or a path fragment ending in one) to the
    last 32 hex chars — uuid-sans-dashes, layout-independent."""
    hexed = "".join(ch for ch in text.lower() if ch in "0123456789abcdef")
    return hexed[-32:]


def active_subagents(max_age=20):
    """{sid_key(parent_session): active_agent_count}. Agent transcripts
    appear in TWO layouts: nested (<proj>/<sid>/subagents/[workflows/
    <wf>/]agent-*.jsonl — current) and flat encoded top-level dirs whose
    name embeds '-subagents-' (older runs). An agent file modified in
    the last max_age seconds counts as actively working."""
    now = time.time()
    out = {}

    def bump(sid, f):
        try:
            if now - f.stat().st_mtime < max_age:
                k = _sid_key(sid)
                out[k] = out.get(k, 0) + 1
        except OSError:
            pass

    try:
        for proj in CLAUDE_PROJECTS.iterdir():
            if not proj.is_dir():
                continue
            if "-subagents-" in proj.name:          # flat encoded layout
                sid = proj.name.split("-subagents-")[0]
                for f in proj.glob("agent-*.jsonl"):
                    bump(sid, f)
                continue
            for sub in proj.glob("*/subagents"):    # nested layout
                sid = sub.parent.name
                for f in sub.glob("agent-*.jsonl"):
                    bump(sid, f)
                for f in sub.glob("*/*/agent-*.jsonl"):
                    bump(sid, f)
    except OSError:
        pass
    return out


# ---------- binding a process to its conversation ----------
# Worked out in the office, which needs a desk to name the conversation
# its process is ACTUALLY on; shared now because every session list has
# the same problem. argv is the one hard fact — the rest is careful
# guessing, and waiting beats guessing wrong.


def _boot_epoch():
    try:
        with open("/proc/stat") as f:
            for line in f:
                if line.startswith("btime"):
                    return int(line.split()[1])
    except (OSError, ValueError, IndexError):
        pass
    return 0


BOOT = _boot_epoch()
try:
    HZ = os.sysconf("SC_CLK_TCK") or 100
except (ValueError, OSError):
    HZ = 100


def _proc_start(pid):
    """Wall-clock start of a pid, or None. Pairing a session to its
    transcript by iteration order handed desks each other's
    conversations — a plain `claude` writes its first line seconds after
    it starts, so time is the honest signal."""
    if not BOOT:
        return None
    try:
        with open(f"/proc/{pid}/stat") as f:
            data = f.read()
        # comm can contain spaces/parens: everything after the LAST ')'
        fields = data[data.rindex(")") + 2:].split()
        return BOOT + int(fields[19]) / HZ
    except (OSError, ValueError, IndexError):
        return None


def _is_sid(s):
    return len(s) >= 32 and all(c in "0123456789abcdef-" for c in s.lower())


def _argv_sid(pid):
    """The session a process was TOLD to run, from its own argv — the one
    hard fact about which conversation a desk belongs to. A fork resumes
    into a NEW id, so its argv names nothing we can bind to."""
    argv = _argv(pid)
    if "--fork-session" in argv:
        return ""
    for i, a in enumerate(argv):
        if a in ("--resume", "-r", "--session-id"):
            nxt = argv[i + 1] if i + 1 < len(argv) else ""
            if _is_sid(nxt):
                return nxt
    return ""


_TX_START = {}                 # path -> epoch of its first timestamped line


def _tx_start(path):
    """When a transcript's conversation began. The first lines are
    untimestamped bookkeeping, so scan a few; degrade to None."""
    key = str(path)
    if key in _TX_START:
        return _TX_START[key]
    out = None
    try:
        with open(path, errors="replace") as f:
            for i, line in enumerate(f):
                if i > 12:
                    break
                try:
                    ts = json.loads(line).get("timestamp")
                except (ValueError, AttributeError):
                    continue
                if not ts:
                    continue
                try:
                    out = datetime.fromisoformat(
                        str(ts).replace("Z", "+00:00")).timestamp()
                except (TypeError, ValueError):
                    out = None
                break
    except OSError:
        pass
    if len(_TX_START) > 200:
        _TX_START.clear()
    _TX_START[key] = out
    return out


def _tx_for_sid(sid, txs):
    """Path of a known session id — from the listing we already have,
    else straight off disk (an old resumed session is not "recent")."""
    for _mt, f in txs:
        if f.stem == sid:
            return str(f)
    try:
        for f in CLAUDE_PROJECTS.glob(f"*/{sid}.jsonl"):
            return str(f)
    except OSError:
        pass
    return ""


def transcript_for(pid, cwd, taken, txs):
    """Which conversation this process is really on. argv wins when it
    names the session (`claude --resume <sid>`) — no guessing beats a
    fact. Otherwise pair unclaimed cwd-matching transcripts by TIME:
    handing them out newest-first while pids arrive oldest-first bound
    desks to each other's sessions. Callers add the winner to `taken`."""
    sid = _argv_sid(pid)
    if sid:
        # argv is fact — if its file has not appeared yet, wait for it
        # rather than pair it wrong
        return _tx_for_sid(sid, txs)
    started = _proc_start(pid)
    best, best_gap = "", None
    for mt, f in txs:
        if str(f) in taken:
            continue
        meta_cwd, _ = session_meta(f, max_lines=15)
        if meta_cwd != cwd:
            continue
        if started is None:      # no clock: newest-first, as before
            return str(f)
        if mt < started - 5:
            continue             # untouched since the process began
        gap = abs((_tx_start(f) or mt) - started)
        if best_gap is None or gap < best_gap:
            best, best_gap = str(f), gap
    # an unbounded best wins by default, so a brand-new `claude` with no
    # transcript yet would adopt a SIBLING session's file in the same cwd.
    # Waiting beats adopting — an unbound row retries on the next poll.
    return best if best_gap is not None and best_gap <= 120 else ""


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
    txs = recent_transcripts()
    # a live session is named by what it IS, not where it runs: every
    # session in $HOME used to list as "~". The directory moves to the
    # detail line, so neither fact is lost.
    taken = set()
    for pid, cwd, interactive, tty in claude_procs():
        tx = transcript_for(pid, cwd, taken, txs)
        if tx:
            taken.add(tx)
        title = session_title(tx) if tx else ""
        sid = Path(tx).stem if tx else ""
        where = nice(cwd)
        if interactive:
            rows.append({"label": title or where, "icon": "󰚩", "kind": "run",
                         "addr": window_of_pid(pid, clients), "cwd": cwd,
                         "pid": pid, "tty": tty, "sid": sid, "dir": where,
                         "detail": f"🟢 running in {where} — Enter focuses"
                                   " its terminal"})
        else:
            rows.append({"label": title or where, "icon": "󰚩", "kind": "bg",
                         "cwd": cwd, "pid": pid, "sid": sid, "dir": where,
                         "detail": f"󰑮 background job in {where} —"
                                   " view on claude.ai"})
    for pid, sid, cwd in daemon_hosted():
        where = nice(cwd)
        tx = _tx_for_sid(sid, txs)
        if tx:
            taken.add(tx)
        rows.append({"label": (session_title(tx) if tx else "") or where,
                     "icon": "󰚩", "kind": "bg",
                     "cwd": cwd, "pid": pid, "sid": sid, "dir": where,
                     "detail": f"󰑮 daemon-hosted in {where} —"
                               " outlives its terminal"})
    for mtime, f in txs:
        cwd, preview = session_meta(f)
        if not preview:
            continue                     # empty/aborted session: skip
        # past rows keep the DIRECTORY as their label — the studio tree
        # groups projects by it. The title rides alongside for flat lists
        # like the launcher, where 20 rows reading "~" help nobody.
        rows.append({"label": nice(cwd), "icon": "󰚩", "kind": "past",
                     "sid": f.stem, "cwd": cwd or home, "dir": nice(cwd),
                     "title": session_title(f),
                     "detail": f"{ago(mtime)} — {preview}"})
    return rows
