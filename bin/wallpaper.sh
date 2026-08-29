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
# the wallpaper's own hues, lightness-corrected so a module tint can follow
# the wallpaper without ever going invisible
"$HOME/.local/bin/gen-readable-colors.sh" --quiet || true
# and verify it: a slot used as text that still fails is a bug, not a taste
"$HOME/.local/bin/check-contrast.sh" --quiet --notify || true
# Push the new palette into the LIVE config by keyword, never `hyprctl
# reload`. A reload re-applies the explicit `monitor =` rules in
# hyprland.conf, and re-applying an explicit mode makes Hyprland disable
# and re-enable every output — a three-monitor DRM re-probe + modeset that
# freezes the whole desktop for seconds on every recolor. `keyword` touches
# only the setting named, so nothing is re-modeset.
apply_colors() {
    local conf="$HOME/.config/hypr/colors.conf" k v
    declare -A C=()
    while IFS= read -r line; do
        case "$line" in
            \$*=*) k="${line%%=*}"; v="${line#*=}"
                   C["$(echo "${k#\$}" | xargs)"]="$(echo "$v" | xargs)" ;;
        esac
    done < "$conf"
    # a slot missing from the template must not blank a border
    for k in wallbg wallfg wallaccent wallaccent2 wallmuted; do
        [ -n "${C[$k]:-}" ] || return 1
    done
    local grad="${C[wallaccent]} ${C[wallaccent2]} 45deg"
    hyprctl --batch "\
        keyword general:col.active_border $grad ;\
        keyword general:col.inactive_border ${C[wallmuted]} ;\
        keyword group:col.border_active $grad ;\
        keyword group:col.border_inactive ${C[wallmuted]} ;\
        keyword group:groupbar:col.active ${C[wallaccent]} ;\
        keyword group:groupbar:col.inactive ${C[wallbg]} ;\
        keyword group:groupbar:text_color ${C[wallfg]} ;\
        keyword plugin:hyprbars:bar_color ${C[wallbg]} ;\
        keyword plugin:hyprbars:col.text ${C[wallfg]}" >/dev/null
}
# Never fall back to `hyprctl reload`: even a rare malformed palette must not
# re-apply the explicit monitor modes and freeze all three outputs. Keep the
# previous border colours and report the recoverable theming failure instead.
if ! apply_colors; then
    notify-send -u normal "Wallpaper applied" \
        "Palette was incomplete; window-border colours were kept." 2>/dev/null || true
fi
pkill -SIGUSR2 waybar 2>/dev/null || true       # waybar restyles in place
swaync-client -rs 2>/dev/null || true           # swaync reloads css
# widget cards + docks retheme LIVE (no restart). Only a component that
# is RUNNING but fails its ctl forces the fallback restart — a
# deliberately disabled dock/host must not condemn every recolor to a
# full fleet bounce
live_ok=1
if pgrep -xf "python3 $HOME/.local/bin/hypr-cardhost" >/dev/null 2>&1 ||
   pgrep -xf "$HOME/.local/bin/hypr-cardhost" >/dev/null 2>&1; then
    ~/.local/bin/hypr-cardhost --ctl reload-theme >/dev/null 2>&1 || live_ok=0
fi
if pgrep -xf "python3 $HOME/.local/bin/hypr-appdock" >/dev/null 2>&1 ||
   pgrep -xf "$HOME/.local/bin/hypr-appdock" >/dev/null 2>&1; then
    ~/.local/bin/hypr-appdock --ctl reload-theme >/dev/null 2>&1 || live_ok=0
fi
pkill -USR2 -xf "python3 $HOME/.local/bin/hypr-claude-office" 2>/dev/null || true
pkill -USR2 -xf "$HOME/.local/bin/hypr-office2d" 2>/dev/null || true
pkill -USR2 -xf "$HOME/.local/bin/hypr-pet" 2>/dev/null || true
# viz bakes its palette too, and `start` below deliberately leaves a running
# one alone — without this it keeps the previous theme's colours forever
pkill -USR2 -xf "python3 $HOME/.local/bin/hypr-viz" 2>/dev/null || true
pkill -USR2 -xf "$HOME/.local/bin/hypr-viz" 2>/dev/null || true
# every kitty window re-inks from the fresh colors-kitty.conf (wallust)
pkill -USR1 -x kitty 2>/dev/null || true
# a LIVE studio re-dresses its tmux bar in the new palette
if pgrep -xf "$HOME/.local/bin/draveniq --sidebar" >/dev/null 2>&1; then
    ~/.local/bin/draveniq --style >/dev/null 2>&1 || true
fi
if [ $live_ok = 1 ]; then
    # the pet bakes its palette at spawn — bounce only the pet
    # rust pet handles USR2 above; only a python pet still needs the bounce
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
