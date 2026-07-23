#!/bin/sh
# Manage the desktop widgets: conky (clock / stats / calendar / claude)
# and the hypr-pet wandering cat. Honors ~/.config/conky/widgets.conf.
#   desktop-widgets.sh [start|stop|restart]
WIDGET_DIR="$HOME/.config/conky/widgets"
CONF="$HOME/.config/conky/widgets.conf"
PET="$HOME/.local/bin/hypr-pet"
OFFICE="$HOME/.local/bin/hypr-claude-office"
DOCK="$HOME/.local/bin/hypr-appdock"
SERWATCH="$HOME/.local/bin/serial-watch"
CARDHOST="$HOME/.local/bin/hypr-cardhost"

setting() {   # setting <key> <default>
    v=$(grep -s "^$1=" "$CONF" | tail -1 | cut -d= -f2)
    echo "${v:-$2}"
}

# serialize all verbs: concurrent restarts (login + wallpaper + settings)
# could otherwise spawn two card hosts / two dock managers
exec 9>"${XDG_RUNTIME_DIR:-/tmp}/desktop-widgets.lock"
flock 9 2>/dev/null || true

case "${1:-start}" in
stop)
    pkill -f "conky -c $WIDGET_DIR" 2>/dev/null
    pkill -xf "python3 $PET" 2>/dev/null
    pkill -xf "python3 $OFFICE" 2>/dev/null
    pkill -xf "python3 $DOCK" 2>/dev/null
    pkill -xf "python3 $SERWATCH" 2>/dev/null
    pkill -xf "python3 $CARDHOST" 2>/dev/null
    # wait for real exit — a half-dead conky makes restart's pgrep guard
    # skip widgets as "already running", and a dying cardhost/dock still
    # owns its ctl socket (the new instance would probe it and disable ctl)
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        pgrep -f "conky -c $WIDGET_DIR" >/dev/null 2>&1 ||
            pgrep -xf "python3 $PET" >/dev/null 2>&1 ||
            pgrep -xf "python3 $DOCK" >/dev/null 2>&1 ||
            pgrep -xf "python3 $CARDHOST" >/dev/null 2>&1 || break
        sleep 0.1
    done
    ;;
restart)
    "$0" stop; "$0" start
    ;;
restart-conky)
    # bounce ONLY the conky twins (arrange saved a conky move) — the
    # cardhost, dock and pet keep running untouched
    pkill -f "conky -c $WIDGET_DIR" 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pgrep -f "conky -c $WIDGET_DIR" >/dev/null 2>&1 || break
        sleep 0.1
    done
    "$0" start
    ;;
start)
    for cfg in "$WIDGET_DIR"/*.conf; do
        [ -e "$cfg" ] || continue          # empty/absent dir: glob is literal
        name=$(basename "$cfg" .conf)
        [ "$(setting "$name" on)" = "on" ] || continue
        # a widget that has migrated to the card host stays conky-OFF: an
        # `inst_` twin OR a matching hyprcard template both mean "this is a
        # card now" — so removing the inst_ line can't resurrect the twin
        grep -qs "^inst_$name=" "$CONF" && continue
        [ -e "$HOME/.config/hyprcard/templates/$name.toml" ] && continue
        pgrep -f "conky -c $cfg" >/dev/null ||
            conky -c "$cfg" >/dev/null 2>&1 &
    done
    if grep -qs "^inst_" "$CONF"; then
        pgrep -xf "python3 $CARDHOST" >/dev/null ||
            "$CARDHOST" >/dev/null 2>&1 &
    fi
    # dock manager runs if the global apps toggle is on OR any monitor has
    # its own dock enabled (dock_<MON>=on) even with apps=off
    if [ "$(setting apps on)" = "on" ] ||
       grep -qsE '^dock_[A-Za-z0-9_]+=on' "$CONF"; then
        pgrep -xf "python3 $DOCK" >/dev/null ||
            "$DOCK" >/dev/null 2>&1 &
    fi
    pgrep -xf "python3 $SERWATCH" >/dev/null ||
        "$SERWATCH" >/dev/null 2>&1 &
    if [ "$(setting pet on)" = "on" ]; then
        if ! pgrep -xf "python3 $PET" >/dev/null; then
            layer=$(setting pet_layer bottom)
            [ "$layer" = "bottom" ] && layer=""
            HYPRPET_LAYER="$layer" "$PET" >/dev/null 2>&1 &
        fi
    fi
    if [ "$(setting claude_office on)" = "on" ]; then
        pgrep -xf "python3 $OFFICE" >/dev/null ||
            "$OFFICE" >/dev/null 2>&1 &
    fi
    ;;
esac
