#!/usr/bin/env bash
# focus-mode.sh — deep-work session controller (ALT+SHIFT+F toggles).
#   start [mins] [label]   begin a session: DND on, timer armed, card live
#   end                    finish: DND off, report what queued up
#   toggle                 start 25min / end, for the keybind
# State drives the focus widget card (widget-focus.sh reads it).
STATE_DIR="$HOME/.local/state/focus-mode"
STATE="$STATE_DIR/state"
UNIT="focus-mode-end"

start() {
    mins="${1:-25}"; label="${2:-deep work}"
    mkdir -p "$STATE_DIR"
    now=$(date +%s)
    printf 'end=%s\nstart=%s\nlabel=%s\n' "$((now + mins * 60))" "$now" "$label" > "$STATE"
    systemctl --user stop "$UNIT.timer" "$UNIT.service" 2>/dev/null
    systemd-run --user --collect --unit="$UNIT" --on-active="${mins}m" \
        "$HOME/.local/bin/focus-mode.sh" end >/dev/null 2>&1
    notify-send "Focus" "${mins} min — ${label}. Notifications muted."
    sleep 1                       # let the notification land before DND
    swaync-client -dn >/dev/null 2>&1
}

end() {
    rm -f "$STATE"
    swaync-client -df >/dev/null 2>&1
    waiting=$(swaync-client -c 2>/dev/null || echo "")
    msg="Session done."
    [ -n "$waiting" ] && [ "$waiting" != "0" ] && msg="Session done — $waiting notification(s) waiting."
    notify-send "Focus" "$msg"
    systemctl --user stop "$UNIT.timer" 2>/dev/null
}

case "${1:-toggle}" in
    start) start "$2" "$3" ;;
    end) end ;;
    toggle) if [ -f "$STATE" ]; then end; else start 25; fi ;;
    *) echo "usage: focus-mode.sh [start [mins] [label] | end | toggle]" >&2; exit 1 ;;
esac
