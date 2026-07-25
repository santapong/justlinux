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
    # LOG BEFORE DELETING. This used to open with `rm -f "$STATE"`, which
    # threw away the start time, the label and therefore the duration —
    # every focus session ever run left no trace at all. Read it first,
    # append a row, then clear.
    if [ -f "$STATE" ]; then
        s_start=$(sed -n 's/^start=//p' "$STATE" | head -1)
        s_label=$(sed -n 's/^label=//p' "$STATE" | head -1)
        s_end=$(sed -n 's/^end=//p' "$STATE" | head -1)
        now=$(date +%s)
        case "$s_start" in
            ''|*[!0-9]*) : ;;      # unreadable state: skip the row, still clean up
            *)
                # actual elapsed, not the planned length — an early ALT+SHIFT+F
                # ends the session and that is the honest number to record
                elapsed=$(( now - s_start ))
                planned=0
                case "$s_end" in ''|*[!0-9]*) : ;; *) planned=$(( s_end - s_start ));; esac
                [ "$elapsed" -lt 0 ] && elapsed=0
                printf '%s\t%s\t%s\t%s\n' "$s_start" "$elapsed" "$planned" \
                    "${s_label:-deep work}" >> "$STATE_DIR/history.tsv"
                # keep it bounded — this file is append-only forever otherwise
                if [ "$(wc -l < "$STATE_DIR/history.tsv" 2>/dev/null || echo 0)" -gt 2000 ]; then
                    tail -1000 "$STATE_DIR/history.tsv" > "$STATE_DIR/history.tsv.tmp" &&
                        mv "$STATE_DIR/history.tsv.tmp" "$STATE_DIR/history.tsv"
                fi
                ;;
        esac
    fi
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
