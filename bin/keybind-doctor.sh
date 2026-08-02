#!/usr/bin/env bash
# keybind-doctor.sh — run this WHEN keybinds are dead (by MOUSE: the bar's
# menu button → this entry — the whole point is it needs no keyboard).
# Captures every known bind-killer's state, appends to a log, and says the
# verdict in a notification.
LOG="$HOME/.local/state/keybind-doctor.log"
mkdir -p "$(dirname "$LOG")"

{
    echo "===== $(date '+%F %T') ====="

    sub=$(hyprctl submap 2>/dev/null | head -1)
    echo "submap: ${sub:-?}"

    echo "--- keyboard-taking processes ---"
    # bracket pattern: a bare 'hypr-arrange' matches the SHELL running this
    # script whenever the invocation quotes the name (QA checklist trap)
    pgrep -af '[h]ypr-arrange' || echo "hypr-arrange: not running"

    echo "--- fcitx5 ---"
    pgrep -x fcitx5 >/dev/null && echo "fcitx5: running" || echo "fcitx5: DOWN"
    # which input context holds focus, if any — a grab needs a focused IC
    timeout 3 busctl --user call org.fcitx.Fcitx5 /controller \
        org.fcitx.Fcitx.Controller1 DebugInfo 2>/dev/null \
        | sed 's/\\n/\n/g' | grep 'focus:1' || echo "no focused IC"

    echo "--- active window ---"
    hyprctl activewindow -j 2>/dev/null \
        | python3 -c "import json,sys; d=json.load(sys.stdin); \
print(d.get('class'), '|', (d.get('title') or '')[:60])" 2>/dev/null

    echo "--- overlay/top layers (anything unexpected holding the screen?) ---"
    hyprctl layers -j 2>/dev/null | python3 -c "
import json,sys
for mon,v in json.load(sys.stdin).items():
    for lvl in ('2','3'):                    # top, overlay
        for l in v.get('levels',{}).get(lvl,[]):
            print(' ', mon, lvl, l.get('namespace'))" 2>/dev/null

    echo "--- did Hyprland just reload? (config mtimes) ---"
    stat -c '%y %n' ~/.config/hypr/hyprland.conf 2>/dev/null
} >> "$LOG" 2>&1

# the one-line verdict, best effort
verdict="state captured"
pgrep -af '[h]ypr-arrange' >/dev/null 2>&1 && verdict="hypr-arrange is holding the keyboard — close its overlay (Esc/click) or: pkill -f hypr-arrange"
[ "$(hyprctl submap 2>/dev/null | head -1)" != "default" ] && \
    [ -n "$(hyprctl submap 2>/dev/null | head -1)" ] && \
    verdict="stuck in submap '$(hyprctl submap | head -1)' — hyprctl dispatch submap reset"

notify-send "Keybind doctor" "$verdict — details in ~/.local/state/keybind-doctor.log"

# offer the big hammer for the fcitx5 hypothesis: restarting it releases
# any IM keyboard grab without touching the session
if pgrep -x fcitx5 >/dev/null; then
    pkill -x fcitx5 && sleep 0.5 && setsid fcitx5 -d >/dev/null 2>&1
    notify-send "Keybind doctor" "fcitx5 restarted (releases a stuck IM grab). If binds come back NOW, the input method was the killer — tell Claude."
fi
