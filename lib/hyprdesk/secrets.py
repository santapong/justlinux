"""Credential store for the fleet — deliberately NOT widgets.conf.

widgets.conf is committed to a public dotfiles repo, so anything secret has
to live somewhere else entirely. This is that somewhere: a single JSON file
under ~/.config/hyprdesk/, created 0600, never referenced by the repo.

    from hyprdesk.secrets import get, put, forget
    tok = get("habitica", "api_token")
    put("habitica", {"user_id": "...", "api_token": "..."})

Values are stored in plain text. That is an honest limit, not an oversight:
without a keyring daemon or a passphrase prompt there is nothing to encrypt
*with* — a key sitting next to the ciphertext protects nobody. File mode is
the actual boundary, so the file is created 0600 and re-checked on every
read; a world-readable secrets file is refused rather than used.
"""
import json
import os
import stat
from pathlib import Path

PATH = Path.home() / ".config/hyprdesk/secrets.json"


def _load():
    try:
        st = PATH.stat()
    except OSError:
        return {}
    if st.st_mode & (stat.S_IRWXG | stat.S_IRWXO):
        # someone widened it since we wrote it — narrow it back rather than
        # reading a secret the whole machine can see
        try:
            PATH.chmod(0o600)
        except OSError:
            return {}
    try:
        d = json.loads(PATH.read_text())
        return d if isinstance(d, dict) else {}
    except (OSError, ValueError):
        return {}


def get(service, key=None, default=None):
    """One value, or the whole dict for a service when key is None."""
    svc = _load().get(service) or {}
    if not isinstance(svc, dict):
        return default
    return svc if key is None else svc.get(key, default)


def put(service, values):
    """Merge `values` into a service. Written 0600 via a temp file so a
    crash mid-write cannot leave a truncated — or worse, world-readable —
    secrets file behind."""
    data = _load()
    svc = data.get(service)
    data[service] = {**(svc if isinstance(svc, dict) else {}), **values}
    PATH.parent.mkdir(parents=True, exist_ok=True)
    tmp = PATH.with_suffix(".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, indent=2)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    os.replace(tmp, PATH)
    try:
        PATH.chmod(0o600)
    except OSError:
        pass
    return True


def forget(service, key=None):
    """Drop one key, or the whole service."""
    data = _load()
    if service not in data:
        return False
    if key is None:
        data.pop(service, None)
    elif isinstance(data[service], dict):
        data[service].pop(key, None)
    PATH.parent.mkdir(parents=True, exist_ok=True)
    tmp = PATH.with_suffix(".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(data, f, indent=2)
    os.replace(tmp, PATH)
    return True


def configured(service, *required):
    """True when every required key is present and non-empty — what a card
    checks before trying to draw remote data."""
    svc = get(service) or {}
    return all(str(svc.get(k, "")).strip() for k in required)
