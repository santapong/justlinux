#!/bin/sh
# Manage the desktop widgets: conky (clock / stats / calendar / claude)
# and the hypr-pet wandering cat. Honors ~/.config/conky/widgets.conf.
#   desktop-widgets.sh [start|stop|restart|restart-conky|restart-office|restart-viz]
WIDGET_DIR="$HOME/.config/conky/widgets"
CONF="$HOME/.config/conky/widgets.conf"
PET="$HOME/.local/bin/hypr-pet"
OFFICE="$HOME/.local/bin/hypr-claude-office"
OFFICE2D="$HOME/.local/bin/hypr-office2d"          # rust floor-plan office
VIZ="$HOME/.local/bin/hypr-viz"
DOCK="$HOME/.local/bin/hypr-appdock"
SERWATCH="$HOME/.local/bin/serial-watch"
CARDHOST="$HOME/.local/bin/hypr-cardhost"

setting() {   # setting <key> <default>
    v=$(grep -s "^$1=" "$CONF" | tail -1 | cut -d= -f2)
    echo "${v:-$2}"
}

# serialize all verbs: concurrent restarts (login + wallpaper + settings)
# could otherwise spawn two card hosts / two dock managers.
# never re-invoke "$0" from a verb — flock without -n blocks, so the child
# would wait on its parent forever. stop/start are functions, called inline.
# every daemon below is spawned with 9>&-: an inherited fd keeps the lock
# held for the daemon's whole life and bricks every later invocation.
exec 9>"${XDG_RUNTIME_DIR:-/tmp}/desktop-widgets.lock"
# -w, NOT an unbounded wait. Serialisation here is best-effort: the cost of
# two overlapping starts is a duplicate daemon, but the cost of waiting
# forever is that the whole fleet stops responding. A daemon that leaks fd 9
# (one spawned without 9>&- by an older build, or by hand) holds this lock
# for its entire life — that had hypr-appdock and serial-watch from a
# previous boot wedging every `start`, which silently killed the pet during
# a wallpaper change because wallpaper.sh runs under `set -e`.
flock -w 10 9 2>/dev/null || echo "desktop-widgets: lock busy, proceeding" >&2

viz_up() { pgrep -xf "python3 $VIZ" >/dev/null 2>&1 || pgrep -xf "$VIZ" >/dev/null 2>&1; }

do_stop() {
    pkill -f "conky -c $WIDGET_DIR" 2>/dev/null
    pkill -xf "python3 $PET" 2>/dev/null   # script form
    pkill -xf "$PET" 2>/dev/null           # rust binary form
    pkill -xf "python3 $OFFICE" 2>/dev/null
    pkill -xf "$OFFICE2D" 2>/dev/null
    pkill -xf "python3 $VIZ" 2>/dev/null
    pkill -xf "$VIZ" 2>/dev/null
    pkill -xf "python3 $DOCK" 2>/dev/null
    pkill -xf "$DOCK" 2>/dev/null
    pkill -xf "python3 $SERWATCH" 2>/dev/null
    pkill -xf "python3 $CARDHOST" 2>/dev/null
    pkill -xf "$CARDHOST" 2>/dev/null
    # wait for real exit — a half-dead conky makes restart's pgrep guard
    # skip widgets as "already running", and a dying cardhost/dock still
    # owns its ctl socket (the new instance would probe it and disable ctl)
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        pgrep -f "conky -c $WIDGET_DIR" >/dev/null 2>&1 ||
            pgrep -xf "python3 $PET" >/dev/null 2>&1 ||
            pgrep -xf "$PET" >/dev/null 2>&1 ||
            pgrep -xf "python3 $DOCK" >/dev/null 2>&1 ||
            pgrep -xf "$DOCK" >/dev/null 2>&1 ||
            pgrep -xf "python3 $CARDHOST" >/dev/null 2>&1 ||
            pgrep -xf "$CARDHOST" >/dev/null 2>&1 ||
            pgrep -xf "python3 $OFFICE" >/dev/null 2>&1 ||
            pgrep -xf "python3 $SERWATCH" >/dev/null 2>&1 ||
            pgrep -xf "python3 $VIZ" >/dev/null 2>&1 ||
            pgrep -xf "$VIZ" >/dev/null 2>&1 || break
        sleep 0.1
    done
}

do_start() {   # $want_viz=1 forces viz back up even when the conf says off
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
            conky -c "$cfg" >/dev/null 2>&1 9>&- &
    done
    if grep -qs "^inst_" "$CONF"; then
        pgrep -xf "python3 $CARDHOST" >/dev/null ||
            "$CARDHOST" >/dev/null 2>&1 9>&- &
    fi
    # dock manager runs if the global apps toggle is on OR any monitor has
    # its own dock enabled (dock_<MON>=on) even with apps=off
    if [ "$(setting apps on)" = "on" ] ||
       grep -qsE '^dock_[A-Za-z0-9_]+=on' "$CONF"; then
        if ! pgrep -xf "python3 $DOCK" >/dev/null && ! pgrep -xf "$DOCK" >/dev/null; then
            "$DOCK" >/dev/null 2>&1 9>&- &
        fi
    fi
    pgrep -xf "python3 $SERWATCH" >/dev/null ||
        "$SERWATCH" >/dev/null 2>&1 9>&- &
    if [ "$(setting pet on)" = "on" ]; then
        # the pet may be the python script (cmdline "python3 <path>") or
        # the rust binary (cmdline "<path>") — guard against both forms
        if ! pgrep -xf "python3 $PET" >/dev/null && ! pgrep -xf "$PET" >/dev/null; then
            layer=$(setting pet_layer bottom)
            [ "$layer" = "bottom" ] && layer=""
            HYPRPET_LAYER="$layer" "$PET" >/dev/null 2>&1 9>&- &
        fi
    fi
    if [ "$(setting claude_office on)" = "on" ]; then
        # office_layout picks WHICH office: grid = the python original,
        # floor = the rust 2D one (walking agents, meeting room). Same
        # toggle, same restart verbs — the layout key is the only switch.
        if [ "$(setting office_layout grid)" = "floor" ] && [ -x "$OFFICE2D" ]; then
            pgrep -xf "$OFFICE2D" >/dev/null ||
                "$OFFICE2D" >/dev/null 2>&1 9>&- &
        else
            pgrep -xf "python3 $OFFICE" >/dev/null ||
                "$OFFICE" >/dev/null 2>&1 9>&- &
        fi
    fi
    # viz never autostarts at login: only the conf toggle or a bounce that
    # found it already running (ALT+SHIFT+Y leaves no trace in the conf)
    if [ "$(setting viz off)" = "on" ] || [ "$want_viz" = 1 ]; then
        if ! pgrep -xf "python3 $VIZ" >/dev/null && ! pgrep -xf "$VIZ" >/dev/null; then
            "$VIZ" >/dev/null 2>&1 9>&- &
        fi
    fi
}

case "${1:-start}" in
stop)
    do_stop
    ;;
restart)
    # a hotkey-started viz is invisible to the conf, so remember what was
    # actually up: otherwise every theme / wallpaper / settings bounce
    # kills the visualizer for good
    viz_up && want_viz=1
    do_stop
    do_start
    ;;
restart-conky)
    # bounce ONLY the conky twins (arrange saved a conky move) — the
    # cardhost, dock and pet keep running untouched
    pkill -f "conky -c $WIDGET_DIR" 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pgrep -f "conky -c $WIDGET_DIR" >/dev/null 2>&1 || break
        sleep 0.1
    done
    do_start
    ;;
restart-office)
    # bounce ONLY the office widget (whichever layout is running) —
    # conky/cardhost/dock/pet keep running untouched
    pkill -xf "python3 $OFFICE" 2>/dev/null
    pkill -xf "$OFFICE2D" 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pgrep -xf "python3 $OFFICE" >/dev/null 2>&1 ||
            pgrep -xf "$OFFICE2D" >/dev/null 2>&1 || break
        sleep 0.1
    done
    do_start
    ;;
toggle-office)
    # ALT+CTRL+O lands here so the hotkey follows office_layout too.
    # The office binaries carry their own toggle semantics (run again
    # kills), so this just picks the right one and exec's it.
    # 9>&- on the execs: the office inherits our fds, and an inherited
    # fd 9 holds the widget lock for the office's WHOLE LIFE — the exact
    # wedge the header comment documents.
    if [ "$(setting office_layout grid)" = "floor" ] && [ -x "$OFFICE2D" ]; then
        exec "$OFFICE2D" 9>&-
    fi
    exec "$OFFICE" 9>&-
    ;;
restart-viz)
    # bounce ONLY hypr-viz — conky/cardhost/dock/pet keep running untouched.
    # NO want_viz capture here: this is the verb Hypr Settings calls right
    # AFTER writing viz=off, so remembering "it was up" would restart it and
    # make the OFF switch a no-op. The conf is authoritative for this verb.
    pkill -xf "python3 $VIZ" 2>/dev/null
    pkill -xf "$VIZ" 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pgrep -xf "python3 $VIZ" >/dev/null 2>&1 || break
        sleep 0.1
    done
    do_start
    ;;
start)
    do_start
    ;;
esac
