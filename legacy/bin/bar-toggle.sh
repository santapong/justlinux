#!/usr/bin/env bash
# Toggle the top bar (ALT+B).
# If the auto-hide daemon is running, let IT do the toggle so its
# internal state stays in sync (it will pin the bar open / resume hiding).
if pgrep -f waybar-autohide.sh >/dev/null; then
    pkill -USR1 -f waybar-autohide.sh
else
    pkill -SIGUSR1 waybar
fi
