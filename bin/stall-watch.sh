#!/usr/bin/env bash
# ============================================================
#  stall-watch.sh — catch desktop freezes in the act
#
#  Every 100ms it times an IPC round-trip to Hyprland. hyprctl is answered
#  on the compositor's MAIN THREAD, so a slow reply is not a slow hyprctl:
#  it is the main loop being blocked, which is exactly what a freeze is.
#
#  A freeze is intermittent and nobody is watching a terminal when it hits,
#  so the point of this script is the SNAPSHOT: when a stall is detected it
#  records who was on CPU, what the compositor logged, and how loaded the
#  box was, at the moment it happened. Reading that after the fact beats
#  guessing.
#
#    stall-watch.sh                 watch until Ctrl-C
#    stall-watch.sh 300             watch for 300 seconds
#    stall-watch.sh --report        summarise what has been caught so far
#
#  Log: ~/.cache/hypr-stalls.log
# ============================================================
set -uo pipefail

LOG="$HOME/.cache/hypr-stalls.log"
THRESHOLD_MS=${STALL_MS:-150}     # below this is scheduling noise, not a freeze

if [ "${1:-}" = "--report" ]; then
    [ -s "$LOG" ] || { echo "No stalls recorded yet: $LOG"; exit 0; }
    echo "=== stalls recorded ==="
    grep -c "^STALL" "$LOG"
    echo "=== worst ==="
    grep "^STALL" "$LOG" | sort -t' ' -k3 -rn | head -5
    echo "=== most frequent process at fault ==="
    grep "^  " "$LOG" | awk '{print $2}' | sort | uniq -c | sort -rn | head -8
    exit 0
fi

DEADLINE=""
[ -n "${1:-}" ] && DEADLINE=$(( $(date +%s) + $1 ))

HL="$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/hyprland.log"

echo "watching (threshold ${THRESHOLD_MS}ms) → $LOG"
printf '\n===== session started %s =====\n' "$(date '+%F %T')" >> "$LOG"

while :; do
    [ -n "$DEADLINE" ] && [ "$(date +%s)" -ge "$DEADLINE" ] && break

    s=$(date +%s%N)
    hyprctl -j version >/dev/null 2>&1
    e=$(date +%s%N)
    ms=$(( (e - s) / 1000000 ))

    if [ "$ms" -ge "$THRESHOLD_MS" ]; then
        {
            echo "STALL $(date '+%F %T') ${ms}ms load=$(cut -d' ' -f1-3 /proc/loadavg)"
            # who held the CPU. ps sorts by lifetime average, which is useless
            # here, so read the two-sample delta top gives instead
            top -b -n2 -d0.2 2>/dev/null | awk '/^ *PID/{p++} p==2 && $12 != "COMMAND" && $12 != "" {print "  "$12" "$9"%cpu "$10"%mem"}' | head -6
            # the compositor's own account of the same moment
            if [ -r "$HL" ]; then
                echo "  --- hyprland.log tail ---"
                grep -v "Cursor buffer imported" "$HL" | tail -3 | sed 's/^/  /'
            fi
        } >> "$LOG"
        echo "caught ${ms}ms stall at $(date '+%T')"
    fi
    sleep 0.1
done
echo "done → $LOG  (summarise with: stall-watch.sh --report)"
