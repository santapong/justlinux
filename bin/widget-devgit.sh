#!/usr/bin/env python3
"""Dev repos widget: branch, dirty files, ahead/behind for each repo in
widgets.conf `dev_repos` (comma-separated paths, ~ ok). Cache 60 s."""
import os, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".local/lib"))
from hyprdesk import conf_get

FIELDS = "--fields" in sys.argv
if "--repos" in sys.argv:
    _i = sys.argv.index("--repos")
    _repos_arg = sys.argv[_i + 1] if len(sys.argv) > _i + 1 else ""
else:
    _repos_arg = conf_get("dev_repos", "~/hyprland-dots")

import hashlib
_tag = hashlib.md5(_repos_arg.encode()).hexdigest()[:8]
RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
CACHE = RUN / (f"widget-devgit-{_tag}.fields" if FIELDS
               else "widget-devgit.cache")
# fields TTL < host interval (60) — equal TTL halves the refresh rate
_TTL = 50 if FIELDS else 60
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < _TTL:
    print(CACHE.read_text(), end=""); raise SystemExit

repos = [Path(p.strip()).expanduser() for p in _repos_arg.split(",")
         if p.strip()]


def git(repo, *args):
    try:
        r = subprocess.run(["git", "-C", str(repo), *args],
                           capture_output=True, text=True, timeout=4)
        return r.stdout.strip() if r.returncode == 0 else None
    except Exception:
        return None


MAX_REPOS = 7        # same reason as the markets card: a glance, not a table


def ago(epoch):
    """Relative age of the last commit — the signal this card was missing.
    A repo you touched an hour ago and one you abandoned in March looked
    identical before, both just '✓ clean'."""
    try:
        d = int(time.time() - int(epoch))
    except (TypeError, ValueError):
        return ""
    if d < 3600:
        return f"{max(1, d // 60)}m"
    if d < 86400:
        return f"{d // 3600}h"
    if d < 86400 * 30:
        return f"{d // 86400}d"
    return f"{d // (86400 * 30)}mo"


if FIELDS:
    lines, n = [], 0
    dropped = max(0, len(repos) - MAX_REPOS)
    for repo in repos[:MAX_REPOS]:
        if not repo.exists():
            # SAY it. A path typo used to vanish silently, which looks
            # exactly like a repo that is fine.
            lines += [f"repo.{n}.name={repo.name[:14] or '?'}",
                      f"repo.{n}.branch=—",
                      f"repo.{n}.state=no such path", f"repo.{n}.slot=bad"]
            n += 1
            continue
        if not (repo / ".git").exists():
            lines += [f"repo.{n}.name={repo.name[:14]}",
                      f"repo.{n}.branch=—",
                      f"repo.{n}.state=not a git repo", f"repo.{n}.slot=sub"]
            n += 1
            continue
        branch = git(repo, "rev-parse", "--abbrev-ref", "HEAD") or "?"
        dirty = len((git(repo, "status", "--porcelain") or "").splitlines())
        ab = git(repo, "rev-list", "--left-right", "--count",
                 "@{upstream}...HEAD")
        behind, ahead = (ab.split() if ab else ("0", "0"))
        marks = []
        if dirty:
            marks.append(f"±{dirty}")
        if ahead != "0":
            marks.append(f"↑{ahead}")
        if behind != "0":
            marks.append(f"↓{behind}")
        # good/bad are STATUS ONLY; a dirty-but-synced repo is prominent
        # (fg), not colored — accent2 is title ink, never a status
        slot = "bad" if behind != "0" else ("fg" if marks else "good")
        last = ago(git(repo, "log", "-1", "--format=%ct"))
        state = " ".join(marks) or "clean"
        if last:
            state = f"{state} · {last}"
        lines += [f"repo.{n}.name={repo.name[:14]}",
                  f"repo.{n}.branch={branch[:12]}",
                  f"repo.{n}.state={state}",
                  f"repo.{n}.slot={slot}"]
        n += 1
    if dropped:
        lines += [f"repo.{n}.name=+{dropped} more",
                  f"repo.{n}.branch=—",
                  f"repo.{n}.state=over the {MAX_REPOS} shown",
                  f"repo.{n}.slot=sub"]
        n += 1
    if not n:
        lines = ["repo.0.name=add repos", "repo.0.branch=dev_repos=",
                 "repo.0.state=—", "repo.0.slot=sub"]
    text = "\n".join(lines)
    CACHE.write_text(text)
    print(text)
    raise SystemExit

rows = ["${color1}  DEV REPOS${color}", "${color3}${hr}${color}"]
shown = 0
for repo in repos:
    if not (repo / ".git").exists():
        continue
    branch = (git(repo, "rev-parse", "--abbrev-ref", "HEAD") or "?").replace("$", "$$")
    dirty = len((git(repo, "status", "--porcelain") or "").splitlines())
    ab = git(repo, "rev-list", "--left-right", "--count", "@{upstream}...HEAD")
    behind, ahead = (ab.split() if ab else ("0", "0"))
    marks = []
    if dirty:
        marks.append(f"${{color1}}±{dirty}${{color}}")
    if ahead != "0":
        marks.append(f"${{color4}}↑{ahead}${{color}}")
    if behind != "0":
        marks.append(f"${{color5}}↓{behind}${{color}}")
    state = " ".join(marks) if marks else "${color4}✓ clean${color}"
    name = repo.name[:14].replace("$", "$$")
    rows.append(f"${{color2}}{name}${{color}} ${{color3}} {branch[:12]}"
                f"${{color}}${{alignr}}{state}")
    shown += 1
if not shown:
    rows.append("${color3}add repos: dev_repos= in widgets.conf${color}")
text = "\n".join(rows)
CACHE.write_text(text)
print(text)
