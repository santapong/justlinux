#!/usr/bin/env python3
"""Dev repos widget: branch, dirty files, ahead/behind for each repo in
widgets.conf `dev_repos` (comma-separated paths, ~ ok). Cache 60 s."""
import os, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".local/lib"))
from hyprdesk import conf_get

CACHE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "widget-devgit.cache"
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < 60:
    print(CACHE.read_text(), end=""); raise SystemExit

repos = [Path(p.strip()).expanduser() for p in
         conf_get("dev_repos", "~/hyprland-dots").split(",") if p.strip()]


def git(repo, *args):
    try:
        r = subprocess.run(["git", "-C", str(repo), *args],
                           capture_output=True, text=True, timeout=4)
        return r.stdout.strip() if r.returncode == 0 else None
    except Exception:
        return None


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
