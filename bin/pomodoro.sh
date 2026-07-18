#!/bin/sh
# Pomodoro timer.  pomodoro.sh start [minutes] | stop | status
# The desktop widget shows `status`; a swaync notification fires when done.
STATE="${XDG_RUNTIME_DIR:-/tmp}/pomodoro.state"

case "${1:-status}" in
start)
    mins="${2:-25}"
    case "$mins" in ''|*[!0-9]*)
        echo "usage: pomodoro.sh start [minutes]"; exit 1 ;;
    esac
    end=$(( $(date +%s) + mins * 60 ))
    echo "$end $mins" > "$STATE"
    # the notifier re-checks it still OWNS the timer — a newer `start`
    # replaces the state file and orphans this job silently
    ( sleep $((mins * 60))
      [ -f "$STATE" ] || exit 0
      read -r cur _ < "$STATE" 2>/dev/null || exit 0
      [ "$cur" = "$end" ] || exit 0
      notify-send -u critical "󰄉 Pomodoro" "$mins minutes are up — take a break!"
      rm -f "$STATE" ) >/dev/null 2>&1 &
    echo "started: $mins min"
    ;;
stop)
    rm -f "$STATE"; echo "stopped"
    ;;
status)
    if [ -f "$STATE" ]; then
        read -r end mins < "$STATE" 2>/dev/null
        case "$end" in ''|*[!0-9]*)   # corrupt state file
            rm -f "$STATE"
            printf '${color3}󰄉 no timer — pomodoro.sh start [min]${color}\n'
            exit 0 ;;
        esac
        left=$(( end - $(date +%s) ))
        if [ "$left" -le 0 ]; then
            printf '${color4}󰄉 done — take a break!${color}\n'
        else
            total=$(( ${mins:-25} * 60 )); filled=$(( (total - left) * 16 / total ))
            bar=""; i=0
            while [ $i -lt 16 ]; do
                [ $i -lt $filled ] && bar="${bar}█" || bar="${bar}░"; i=$((i+1))
            done
            printf '${color1}󰄉 focus${color}${alignr}${color2}%02d:%02d${color}\n${color2}%s${color}\n' \
                   $((left / 60)) $((left % 60)) "$bar"
        fi
    else
        printf '${color3}󰄉 no timer — pomodoro.sh start [min]${color}\n'
    fi
    ;;
esac
