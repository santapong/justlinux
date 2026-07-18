#!/bin/sh
# Manage the desktop widgets: conky (clock / stats / calendar / claude)
# and the hypr-pet wandering cat. Honors ~/.config/conky/widgets.conf.
#   desktop-widgets.sh [start|stop|restart]
WIDGET_DIR="$HOME/.config/conky/widgets"
CONF="$HOME/.config/conky/widgets.conf"
PET="$HOME/.local/bin/hypr-pet"
DOCK="$HOME/.local/bin/hypr-appdock"

setting() {   # setting <key> <default>
    v=$(grep -s "^$1=" "$CONF" | tail -1 | cut -d= -f2)
    echo "${v:-$2}"
}

case "${1:-start}" in
stop)
    pkill -f "conky -c $WIDGET_DIR" 2>/dev/null
    pkill -xf "python3 $PET" 2>/dev/null
    pkill -xf "python3 $DOCK" 2>/dev/null
    # wait for real exit — a half-dead conky makes restart's pgrep guard
    # skip widgets as "already running"
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        pgrep -f "conky -c $WIDGET_DIR" >/dev/null 2>&1 ||
            pgrep -xf "python3 $PET" >/dev/null 2>&1 || break
        sleep 0.1
    done
    ;;
restart)
    "$0" stop; "$0" start
    ;;
start)
    for cfg in "$WIDGET_DIR"/*.conf; do
        name=$(basename "$cfg" .conf)
        [ "$(setting "$name" on)" = "on" ] || continue
        pgrep -f "conky -c $cfg" >/dev/null ||
            conky -c "$cfg" >/dev/null 2>&1 &
    done
    if [ "$(setting apps on)" = "on" ]; then
        pgrep -xf "python3 $DOCK" >/dev/null ||
            "$DOCK" >/dev/null 2>&1 &
    fi
    if [ "$(setting pet on)" = "on" ]; then
        if ! pgrep -xf "python3 $PET" >/dev/null; then
            layer=$(setting pet_layer bottom)
            [ "$layer" = "bottom" ] && layer=""
            HYPRPET_LAYER="$layer" "$PET" >/dev/null 2>&1 &
        fi
    fi
    ;;
esac
