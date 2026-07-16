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
hyprctl reload >/dev/null                       # window borders
pkill -SIGUSR2 waybar 2>/dev/null || true       # waybar restyles in place
swaync-client -rs 2>/dev/null || true           # swaync reloads css

echo "Desktop recolored ✔"
