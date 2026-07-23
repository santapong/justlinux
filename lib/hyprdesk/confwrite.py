"""Atomic, lock-guarded writer for widgets.conf — the fleet's ONLY writer.

Readers (hyprdesk.theme.conf(), card.lua conf(), hypr-settings) parse flat
``key=value`` lines with keys ``\\w+`` and values ``\\S+``; this module
enforces that grammar on write and makes every write atomic
(flock + temp + os.replace), so concurrent writers — the Settings TUI,
the arrange overlay, the card host — can no longer lose each other's keys.

The previous file content is kept one-deep in widgets.conf.undo; callers
that want a true "session" snapshot (e.g. arrange-commit undo) should
batch all their changes into a single conf_set() call.
"""
import fcntl
import os
import re

from .theme import WIDGETS_CONF

LOCK = WIDGETS_CONF.parent / (WIDGETS_CONF.name + ".lock")
TMP = WIDGETS_CONF.parent / (WIDGETS_CONF.name + ".tmp")
UNDO = WIDGETS_CONF.parent / (WIDGETS_CONF.name + ".undo")

KEY_RE = re.compile(r"^\w+$")
VAL_RE = re.compile(r"^\S+$")
LINE_RE = re.compile(r"^(\w+)\s*=")


def sanitize_key_fragment(text):
    """Monitor connector (or similar) → legal key fragment: 'DP-1' → 'DP_1'."""
    return re.sub(r"[^\w]", "_", str(text))


def conf_set(changes, delete=()):
    """Apply ``{key: value}`` updates and ``delete`` removals in ONE atomic write.

    Values are stringified. Keys must match ``\\w+`` and values ``\\S+``
    (three independent parsers rely on that grammar) — violations raise
    ValueError instead of silently writing keys nothing can read back.
    Line order and comments are preserved; new keys append at the end.
    """
    changes = {str(k): str(v) for k, v in dict(changes).items()}
    delete = {str(k) for k in delete}
    for k in list(changes) + list(delete):
        if not KEY_RE.match(k):
            raise ValueError(f"illegal widgets.conf key: {k!r} (must be \\w+)")
    for k, v in changes.items():
        if not VAL_RE.match(v):
            raise ValueError(
                f"illegal widgets.conf value for {k}: {v!r} (no whitespace)")

    WIDGETS_CONF.parent.mkdir(parents=True, exist_ok=True)
    with open(LOCK, "w") as lockf:
        fcntl.flock(lockf, fcntl.LOCK_EX)
        try:
            original = WIDGETS_CONF.read_text()
        except OSError:
            original = ""
        out, pending, written = [], dict(changes), set()
        for line in original.splitlines():
            m = LINE_RE.match(line)
            key = m.group(1) if m else None
            if key in delete:
                continue
            if key in written:
                continue          # readers are last-wins: drop stale duplicates
            if key in pending:
                out.append(f"{key}={pending.pop(key)}")
                written.add(key)
            else:
                out.append(line)
        out.extend(f"{k}={v}" for k, v in pending.items())
        text = "\n".join(out) + "\n"
        if text != original:
            if original:
                UNDO.write_text(original)      # one-deep undo
            TMP.write_text(text)
            os.replace(TMP, WIDGETS_CONF)
        # lock released when lockf closes


def conf_delete(keys):
    """Remove keys from widgets.conf (atomic, lock-guarded)."""
    conf_set({}, delete=keys)


def conf_undo():
    """Restore the previous widgets.conf (one-deep). Returns True if restored."""
    WIDGETS_CONF.parent.mkdir(parents=True, exist_ok=True)   # dir may be gone
    with open(LOCK, "w") as lockf:
        fcntl.flock(lockf, fcntl.LOCK_EX)
        try:
            prev = UNDO.read_text()
        except OSError:
            return False
        try:
            UNDO.write_text(WIDGETS_CONF.read_text())   # undo the undo
        except OSError:
            pass
        TMP.write_text(prev)
        os.replace(TMP, WIDGETS_CONF)
    return True


def snapshot(tag):
    """Save a NAMED snapshot of widgets.conf (e.g. before an arrange
    commit) — immune to being clobbered by unrelated conf_set writers,
    unlike the shared one-deep UNDO."""
    WIDGETS_CONF.parent.mkdir(parents=True, exist_ok=True)
    snap = WIDGETS_CONF.parent / (WIDGETS_CONF.name + f".snap-{tag}")
    with open(LOCK, "w") as lockf:
        fcntl.flock(lockf, fcntl.LOCK_EX)
        try:
            snap.write_text(WIDGETS_CONF.read_text())
        except OSError:
            return False
    return True


def restore(tag):
    """Restore a named snapshot. Returns True if it existed and applied."""
    snap = WIDGETS_CONF.parent / (WIDGETS_CONF.name + f".snap-{tag}")
    with open(LOCK, "w") as lockf:
        fcntl.flock(lockf, fcntl.LOCK_EX)
        try:
            text = snap.read_text()
        except OSError:
            return False
        TMP.write_text(text)
        os.replace(TMP, WIDGETS_CONF)
    return True
