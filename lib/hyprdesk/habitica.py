"""Habitica API v3 client — just the parts a desk board needs.

Credentials come from hyprdesk.secrets ("habitica": user_id, api_token),
never from widgets.conf, because that file is in a public repo.

Habitica's model is four task types — Habits, Dailies, To-Dos, Rewards —
which are NOT kanban columns, so each gets its own tab and only To-Dos are
laid out as one.

"Doing" is the honest problem here. Habitica has no such state. Checklist
progress was tried as a stand-in and was wrong for real use: almost no
to-do has a checklist, so Doing and Done sat empty forever. So Doing is a
TAG — the one place Habitica lets you say something it did not think of —
named `doing` and shared with the phone app and the website.

    To do   neither of the below
    Doing   carries the `doing` tag
    Done    task marked complete

The tag is created the first time something is moved into Doing, never on
a read: opening a board must not write to your account.

Dailies and Habits keep their own tabs. "Done" for a Daily means "done
today", and a Habit is not something you finish at all — it is a + / −
you press. Neither claim survives being squeezed into a To-Do column.
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


DOING_TAG = "doing"


def tags(fresh=False):
    if fresh:
        _CACHE.pop("tags", None)
    return _cached("tags", lambda: (_call("/tags").get("data") or []))


def doing_tag_id(fresh=False):
    """The id of the `doing` tag, or "" if it does not exist yet. Never
    creates it — a read must not write to somebody's account."""
    for t in tags(fresh=fresh):
        if (t.get("name") or "").strip().lower() == DOING_TAG:
            return t.get("id") or ""
    return ""


def ensure_doing_tag():
    """The id, creating the tag if this is the first time anything moved
    into Doing."""
    tid = doing_tag_id()
    if tid:
        return tid
    d = (_call("/tags", method="POST", body={"name": DOING_TAG})
         .get("data") or {})
    _CACHE.pop("tags", None)
    tid = d.get("id") or ""
    if not tid:
        raise HabiticaError("Habitica would not create the `doing` tag")
    return tid


def add_tag(task_id, tag_id):
    out = _call(f"/tasks/{task_id}/tags/{tag_id}", method="POST")
    _CACHE.clear()
    return out


def del_tag(task_id, tag_id):
    out = _call(f"/tasks/{task_id}/tags/{tag_id}", method="DELETE")
    _CACHE.clear()
    return out


def set_column(task, col):
    """Move a To-Do between board columns, saying it in Habitica's own
    terms. Returns a short line describing what was actually done, because
    "moved to Done" and "ticked off, +gold" are not the same news."""
    tid = task.get("id")
    was_done = bool(task.get("completed")) or task.get("col") == "done"
    said = []
    if col == "done":
        if not was_done:
            score(tid, "up")
            said.append("ticked off in Habitica")
        if task.get("doing"):
            del_tag(tid, doing_tag_id())
            said.append("cleared its `doing` tag")
    else:
        if was_done:
            # scoring a completed To-Do down is Habitica's own undo — it
            # takes back the reward too, which is what "not done" means
            score(tid, "down")
            said.append("un-ticked it")
        if col == "doing" and not task.get("doing"):
            add_tag(tid, ensure_doing_tag())
            said.append("tagged it `doing`")
        elif col == "todo" and task.get("doing"):
            del_tag(tid, doing_tag_id())
            said.append("cleared its `doing` tag")
    _CACHE.clear()
    return " · ".join(said) or "already there"


def _col(t, doing_id=""):
    """Which column a To-Do belongs in, from Habitica's own fields."""
    if t.get("completed"):
        return "done"
    if doing_id and doing_id in (t.get("tags") or []):
        return "doing"
    return "todo"


def board(fresh=False):
    """{'todo': [...], 'doing': [...], 'done': [...], 'dailies': [...]}.

    Each card: id, text, notes, checklist progress, due date, priority.
    Raises HabiticaError — the caller decides how to say it.
    """
    # `type=todos` returns only the UNFINISHED ones — a ticked to-do
    # vanishes from it entirely. That, not just the old checklist rule, is
    # why Done was always empty: the cards were never fetched.
    todos = tasks("todos", fresh=fresh) + tasks("completedTodos", fresh=fresh)
    # NOT fresh: Habitica allows ~30 requests a minute and a board refresh
    # already spends five. Tags almost never change, and the two calls that
    # can change them (ensure_doing_tag, add/del_tag) clear the cache.
    doing_id = doing_tag_id()
    cols = {"todo": [], "doing": [], "done": [], "dailies": [], "habits": []}
    for t in todos:
        items = t.get("checklist") or []
        col = _col(t, doing_id)
        cols[col].append({
            "id": t.get("id") or t.get("_id"),
            "text": (t.get("text") or "").strip(),
            "notes": (t.get("notes") or "").strip(),
            "done_n": sum(1 for i in items if i.get("completed")),
            "total_n": len(items),
            "due": (t.get("date") or "")[:10],
            "pri": t.get("priority", 1),
            "kind": "todo",
            "col": col,
            "completed": bool(t.get("completed")),
            "done_at": (t.get("dateCompleted") or "")[:10],
            "doing": bool(doing_id and doing_id in (t.get("tags") or [])),
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
            "streak": t.get("streak") or 0,
        })
    for t in tasks("habits", fresh=fresh):
        # a Habit is a + / − you press, not something you finish; its
        # counters reset on its own schedule, which is why they are shown
        cols["habits"].append({
            "id": t.get("id") or t.get("_id"),
            "text": (t.get("text") or "").strip(),
            "notes": (t.get("notes") or "").strip(),
            "up": bool(t.get("up", True)),
            "down": bool(t.get("down", True)),
            "up_n": t.get("counterUp") or 0,
            "down_n": t.get("counterDown") or 0,
            "pri": t.get("priority", 1),
            "kind": "habit",
        })
    # highest difficulty first, then nearest due date — the order you'd
    # actually pick work in
    for k in ("todo", "doing"):
        cols[k].sort(key=lambda c: (-float(c["pri"] or 1), c["due"] or "9999"))
    # Done is a history, so it reads newest-first. Habitica hands back
    # roughly the last 30; the board says so rather than implying that is
    # everything you have ever finished.
    cols["done"].sort(key=lambda c: c.get("done_at") or "", reverse=True)
    return cols
