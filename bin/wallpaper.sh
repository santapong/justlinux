#!/usr/bin/env bash
# ============================================================
#  wallpaper.sh — set wallpaper + recolor the whole desktop
#  Usage:
#    wallpaper.sh                     random image, all monitors
#    wallpaper.sh <image>             image on all monitors
#    wallpaper.sh <image> <monitor>   image on one monitor (e.g. DP-1)
#  Recolors: waybar / rofi / swaync / wlogout / hyprland borders
# ============================================================
set -euo pipefail

WALLDIR="$HOME/Pictures/wallpaper"
PAPER_CONF="$HOME/.config/hypr/hyprpaper.conf"
export PATH="$HOME/.cargo/bin:$PATH"

WALL="${1:-}"
MON="${2:-all}"
if [ -z "$WALL" ]; then
    WALL=$(find "$WALLDIR" -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' \) | shuf -n1)
fi
[ -e "$WALL" ] || { echo "No such image or folder: $WALL"; exit 1; }
WALL="$(realpath "$WALL")"
SLIDESHOW=0
[ -d "$WALL" ] && SLIDESHOW=1
echo "Wallpaper: $WALL  (target: $MON$([ $SLIDESHOW = 1 ] && echo ', slideshow'))"

# --- read existing assignments from hyprpaper.conf ---
declare -A ASSIGN=()
FALLBACK=""
cur_mon=""
while IFS= read -r line; do
    case "$line" in
        *monitor*=*) cur_mon="$(echo "${line#*=}" | xargs)" ;;
        *path*=*)    p="$(echo "${line#*=}" | xargs)"
                     if [ -z "$cur_mon" ]; then FALLBACK="$p"; else ASSIGN[$cur_mon]="$p"; fi
                     cur_mon="" ;;
    esac
done < <(grep -E '^\s*(monitor|path)\s*=' "$PAPER_CONF" 2>/dev/null || true)

# --- update assignments ---
if [ "$MON" = "all" ]; then
    FALLBACK="$WALL"
    ASSIGN=()          # clear per-monitor overrides so "all" really means all
else
    ASSIGN[$MON]="$WALL"
    [ -z "$FALLBACK" ] && FALLBACK="$WALL"
fi

# --- persist (hyprpaper 0.8+ block format) ---
write_block() {  # $1=monitor $2=path
    printf 'wallpaper {\n    monitor  = %s\n    path     = %s\n    fit_mode = cover\n' "$1" "$2"
    if [ -d "$2" ]; then
        printf '    timeout   = 300\n    order     = random\n    recursive = true\n'
    fi
    printf '}\n'
}
{
    echo "# Managed by wallpaper.sh — run 'wallpaper.sh <image|folder> [monitor]' to change."
    write_block "" "$FALLBACK"
    for m in "${!ASSIGN[@]}"; do
        write_block "$m" "${ASSIGN[$m]}"
    done
    echo "splash = false"
    echo "ipc = true"
} > "$PAPER_CONF"

# --- apply live ---
if [ $SLIDESHOW = 1 ]; then
    # directories need a config reload — restart hyprpaper
    pkill hyprpaper 2>/dev/null || true; sleep 0.5
    hyprctl dispatch exec hyprpaper >/dev/null
    # recolor from a random image inside the folder
    RECOLOR=$(find "$WALL" -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' \) | shuf -n1)
else
    if [ "$MON" = "all" ]; then
        hyprctl hyprpaper wallpaper ", $WALL" >/dev/null
        for m in $(hyprctl monitors | awk '/^Monitor/{print $2}'); do
            hyprctl hyprpaper wallpaper "$m, $WALL" >/dev/null
        done
    else
        hyprctl hyprpaper wallpaper "$MON, $WALL" >/dev/null
    fi
    RECOLOR="$WALL"
fi

# --- recolor the desktop ---
wallust run "$RECOLOR"
# a new wallpaper can drop an unreadable colour into any ansi slot —
# shout instead of shipping an invisible warning light
"$HOME/.local/bin/check-contrast.sh" --quiet --notify || true
hyprctl reload >/dev/null                       # window borders
pkill -SIGUSR2 waybar 2>/dev/null || true       # waybar restyles in place
swaync-client -rs 2>/dev/null || true           # swaync reloads css
# widget cards + docks retheme LIVE (no restart). Only a component that
# is RUNNING but fails its ctl forces the fallback restart — a
# deliberately disabled dock/host must not condemn every recolor to a
# full fleet bounce
live_ok=1
if pgrep -xf "python3 $HOME/.local/bin/hypr-cardhost" >/dev/null 2>&1; then
    ~/.local/bin/hypr-cardhost --ctl reload-theme >/dev/null 2>&1 || live_ok=0
fi
if pgrep -xf "python3 $HOME/.local/bin/hypr-appdock" >/dev/null 2>&1; then
    ~/.local/bin/hypr-appdock --ctl reload-theme >/dev/null 2>&1 || live_ok=0
fi
pkill -USR2 -xf "python3 $HOME/.local/bin/hypr-claude-office" 2>/dev/null || true
# viz bakes its palette too, and `start` below deliberately leaves a running
# one alone — without this it keeps the previous theme's colours forever
pkill -USR2 -xf "python3 $HOME/.local/bin/hypr-viz" 2>/dev/null || true
if [ $live_ok = 1 ]; then
    # the pet bakes its palette at spawn — bounce only the pet
    pkill -xf "python3 $HOME/.local/bin/hypr-pet" 2>/dev/null || true
    for _ in $(seq 30); do
        pgrep -xf "python3 $HOME/.local/bin/hypr-pet" >/dev/null 2>&1 || break
        sleep 0.1
    done
    ~/.local/bin/desktop-widgets.sh start       # respawns pet, skips the rest
else
    ~/.local/bin/desktop-widgets.sh restart
fi

echo "Desktop recolored ✔"
