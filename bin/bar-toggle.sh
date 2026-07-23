#!/usr/bin/env bash
# Toggle the top bar (ALT+B).
# When the smart bar owns waybar (bar_smart=on), ALT+B PINS it open /
# releases it — a raw SIGUSR1 here would desync the smart-bar poller,
# which re-hides an unpinned bar within half a second.
# Fallbacks: the legacy waybar-autohide daemon, then a raw toggle.
if grep -qx 'bar_smart=on' "$HOME/.config/conky/widgets.conf" 2>/dev/null \
        && out=$("$HOME/.local/bin/hypr-appdock" --ctl bar-pin 2>/dev/null); then
    case "$out" in
        *pinned*)   notify-send "Waybar" "Bar pinned open — ALT+B to release" ;;
        *released*) notify-send "Waybar" "Bar released — auto-hide resumes" ;;
    esac
elif pgrep -f waybar-autohide.sh >/dev/null; then
    pkill -USR1 -f waybar-autohide.sh
else
    pkill -SIGUSR1 waybar
fi
