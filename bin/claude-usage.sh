#!/usr/bin/env python3
"""Claude plan usage as conky markup — the same numbers claude.ai shows:
current-session (5 h) %, weekly all-models %, weekly Fable %, with reset
times. Fetched from the OAuth usage endpoint using Claude Code's own
credentials; falls back to the last good result if offline.
Cached 5 min — the API is polled at most 12×/hour."""
import json, os, time, urllib.request
from datetime import datetime, timezone
from pathlib import Path

RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
CACHE = RUN / "claude-usage.cache"
STALE = RUN / "claude-usage.last"          # survives API failures
TTL = 300
BAR = 18

if CACHE.exists() and time.time() - CACHE.stat().st_mtime < TTL:
    print(CACHE.read_text(), end="")
    raise SystemExit


def bar(pct):
    f = max(0, min(BAR, round(pct / 100 * BAR)))
    color = "color2" if pct < 80 else "color5"
    return f"${{{color}}}{'█' * f}${{color}}${{color3}}{'░' * (BAR - f)}${{color}}"


def when(iso, kind):
    try:
        t = datetime.fromisoformat(iso).astimezone()
    except (ValueError, TypeError):
        return ""
    if kind == "session":
        left = (t - datetime.now(timezone.utc).astimezone()).total_seconds()
        if left <= 0:
            return "resets soon"
        return f"↺ {int(left // 3600)}h{int(left % 3600 // 60):02d}"
    return f"↺ {t.strftime('%a %H:%M')}"


def fetch():
    creds = json.loads((Path.home() / ".claude/.credentials.json").read_text())
    tok = creds["claudeAiOauth"]["accessToken"]
    req = urllib.request.Request(
        "https://api.anthropic.com/api/oauth/usage",
        headers={"Authorization": f"Bearer {tok}",
                 "anthropic-beta": "oauth-2025-04-20"})
    with urllib.request.urlopen(req, timeout=4) as r:
        return json.loads(r.read())


try:
    data = fetch()
    rows = []
    for lim in data.get("limits", []):
        pct = lim.get("percent") or 0
        reset = when(lim.get("resets_at"), lim.get("kind"))
        if lim.get("kind") == "session":
            label = "Session (5 h)"
        elif lim.get("kind") == "weekly_all":
            label = "Weekly · all"
        elif lim.get("kind") == "weekly_scoped":
            model = ((lim.get("scope") or {}).get("model") or {})
            label = f"Weekly · {(model.get('display_name') or 'model')[:14]}"
        else:
            continue
        rows.append(f"${{color3}}{label}${{color}}${{alignr}}"
                    f"${{color3}}{reset}${{color}}")
        rows.append(f"{bar(pct)}${{alignr}}{pct:.0f}% used")
    text = "\n".join(
        [f"${{color1}}󱚝  CLAUDE PLAN USAGE${{color}}",
         "${color3}${hr}${color}"] + rows)
    STALE.write_text(text)
except Exception:
    if STALE.exists():
        text = STALE.read_text() + "\n${color3}(offline — last known)${color}"
    else:
        text = ("${color1}󱚝  CLAUDE PLAN USAGE${color}\n"
                "${color3}usage API unreachable${color}")

CACHE.write_text(text)
print(text)
