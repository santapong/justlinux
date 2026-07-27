"""Habitica API v3 client — just the parts a desk board needs.

Credentials come from hyprdesk.secrets ("habitica": user_id, api_token),
never from widgets.conf, because that file is in a public repo.

Habitica's model is four task types — Habits, Dailies, To-Dos, Rewards —
which are NOT kanban columns. Forcing all four into three columns would
misrepresent the data, so board() maps only To-Dos onto columns, using the
signal Habitica actually gives us:

    To do   no checklist item ticked yet
    Doing   some ticked, not all      (a real "started" signal)
    Done    task marked complete

Dailies stay a separate strip: they recur, so "Done" for a Daily means
"done today", which is a different claim from a To-Do being finished.
"""
import json
import time
import urllib.error
import urllib.request

from . import secrets

BASE = "https://habitica.com/api/v3"
TIMEOUT = 10
_CACHE = {}
_CACHE_TTL = 60


class HabiticaError(Exception):
    """Anything that stops us answering — no credentials, refused, offline.
    Carries a message a human can act on, not a stack trace."""


def _creds():
    c = secrets.get("habitica") or {}
    uid, tok = str(c.get("user_id", "")).strip(), str(c.get("api_token", "")).strip()
    if not uid or not tok:
        raise HabiticaError("not set up — add your Habitica keys in Settings")
    return uid, tok


def _call(path, method="GET", body=None, creds=None):
    uid, tok = creds or _creds()
    req = urllib.request.Request(
        f"{BASE}{path}", method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={
            "x-api-user": uid,
            "x-api-key": tok,
            # Habitica asks third-party clients to identify themselves as
            # <UserID>-<AppName>; unidentified clients can get rate-limited
            "x-client": f"{uid}-hyprdesk",
            "Content-Type": "application/json",
        })
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        if e.code == 401:
            raise HabiticaError("Habitica refused those keys — check them "
                                "in Settings") from e
        if e.code == 429:
            raise HabiticaError("rate-limited by Habitica; try shortly") from e
        raise HabiticaError(f"Habitica returned HTTP {e.code}") from e
    except urllib.error.URLError as e:
        raise HabiticaError(f"cannot reach Habitica ({e.reason})") from e
    except (TimeoutError, OSError) as e:
        raise HabiticaError(f"cannot reach Habitica ({e})") from e
    except ValueError as e:
        raise HabiticaError("Habitica sent something unreadable") from e


def check(user_id, api_token):
    """Validate a credential pair BEFORE storing it, so a typo fails at the
    point of entry instead of silently on the board days later.
    Returns the display name."""
    uid, tok = str(user_id).strip(), str(api_token).strip()
    if not uid or not tok:
        raise HabiticaError("both the User ID and the API token are needed")
    d = _call("/user?userFields=profile,stats", creds=(uid, tok))
    prof = ((d.get("data") or {}).get("profile") or {})
    return prof.get("name") or "your account"


def _cached(key, fn):
    hit = _CACHE.get(key)
    if hit and time.time() - hit[0] < _CACHE_TTL:
        return hit[1]
    val = fn()
    _CACHE[key] = (time.time(), val)
    return val


def tasks(kind="todos", fresh=False):
    """kind: todos | dailys | habits | rewards. Habitica spells it 'dailys'."""
    if fresh:
        _CACHE.pop(f"tasks:{kind}", None)
    return _cached(f"tasks:{kind}",
                   lambda: (_call(f"/tasks/user?type={kind}").get("data") or []))


def stats(fresh=False):
    """hp / mp / exp / level / gold — the bit that makes it a game."""
    if fresh:
        _CACHE.pop("stats", None)

    def go():
        d = (_call("/user?userFields=stats,profile").get("data") or {})
        s = d.get("stats") or {}
        return {
            "level": s.get("lvl", 0),
            "hp": round(s.get("hp", 0)),
            "maxhp": round(s.get("maxHealth", 50)),
            "exp": round(s.get("exp", 0)),
            "next": round(s.get("toNextLevel", 0)),
            "gold": round(s.get("gp", 0)),
            "name": ((d.get("profile") or {}).get("name") or ""),
        }
    return _cached("stats", go)


def score(task_id, direction="up"):
    """Tick a task off (or undo it). Clears the cache so the board redraws
    from the truth rather than from what we hoped happened."""
    out = _call(f"/tasks/{task_id}/score/{direction}", method="POST")
    _CACHE.clear()
    return out


def _col(t):
    """Which column a To-Do belongs in, from Habitica's own fields."""
    if t.get("completed"):
        return "done"
    items = t.get("checklist") or []
    if items and any(i.get("completed") for i in items):
        return "doing"
    return "todo"


def board(fresh=False):
    """{'todo': [...], 'doing': [...], 'done': [...], 'dailies': [...]}.

    Each card: id, text, notes, checklist progress, due date, priority.
    Raises HabiticaError — the caller decides how to say it.
    """
    todos = tasks("todos", fresh=fresh)
    cols = {"todo": [], "doing": [], "done": [], "dailies": []}
    for t in todos:
        items = t.get("checklist") or []
        cols[_col(t)].append({
            "id": t.get("id") or t.get("_id"),
            "text": (t.get("text") or "").strip(),
            "notes": (t.get("notes") or "").strip(),
            "done_n": sum(1 for i in items if i.get("completed")),
            "total_n": len(items),
            "due": (t.get("date") or "")[:10],
            "pri": t.get("priority", 1),
            "kind": "todo",
        })
    for t in tasks("dailys", fresh=fresh):
        if not t.get("isDue", True):
            continue                      # not scheduled for today
        items = t.get("checklist") or []
        cols["dailies"].append({
            "id": t.get("id") or t.get("_id"),
            "text": (t.get("text") or "").strip(),
            "notes": (t.get("notes") or "").strip(),
            "done_n": sum(1 for i in items if i.get("completed")),
            "total_n": len(items),
            "due": "",
            "pri": t.get("priority", 1),
            "kind": "daily",
            "completed": bool(t.get("completed")),
        })
    # highest difficulty first, then nearest due date — the order you'd
    # actually pick work in
    for k in ("todo", "doing", "done"):
        cols[k].sort(key=lambda c: (-float(c["pri"] or 1), c["due"] or "9999"))
    return cols
