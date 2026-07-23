#!/usr/bin/env bash
# wallpaper-ambient.sh — time-and-weather-reactive wallpaper.
#   (no arg)   generate + apply a gradient matched to the hour + weather,
#              then let wallpaper.sh cascade it through wallust so the
#              WHOLE desktop re-themes with the sky
#   on         enable the hourly systemd user timer
#   off        disable the timer (wallpaper stays until you change it)
# Zero-API: reads the weather widget's existing wttr.in cache; pure
# ImageMagick gradients, no image generation service.
RUN="${XDG_RUNTIME_DIR:-/tmp}"
OUT_DIR="$HOME/.local/share/wallpapers"
OUT="$OUT_DIR/ambient.png"
UNIT="wallpaper-ambient"

case "${1:-}" in
    on)
        systemd-run --user --collect --unit="$UNIT" \
            --on-calendar='hourly' "$HOME/.local/bin/wallpaper-ambient.sh" \
            >/dev/null 2>&1
        "$HOME/.local/bin/wallpaper-ambient.sh"   # apply immediately too
        notify-send "Ambient wallpaper" "Following the sky — hourly."
        exit 0 ;;
    off)
        systemctl --user stop "$UNIT.timer" 2>/dev/null
        notify-send "Ambient wallpaper" "Timer off."
        exit 0 ;;
esac

hour=$(date +%H)
desc=$(grep -m1 '^desc=' "$RUN/widget-weather.fields" 2>/dev/null | cut -d= -f2)
desc=${desc,,}

# time-of-day base gradient (top → bottom)
if   [ "$hour" -ge 5 ]  && [ "$hour" -lt 8 ];  then top="#2b3a67"; bot="#eda85c"  # dawn
elif [ "$hour" -ge 8 ]  && [ "$hour" -lt 17 ]; then top="#3a6ea5"; bot="#a8c6df"  # day
elif [ "$hour" -ge 17 ] && [ "$hour" -lt 20 ]; then top="#3d2b56"; bot="#e2703a"  # dusk
else                                                top="#0b1026"; bot="#1c2541"  # night
fi

# weather mood overrides
case "$desc" in
    *rain*|*drizzle*|*thunder*) top="#2e3440"; bot="#4c566a" ;;   # rain: slate
    *mist*|*fog*|*overcast*)    top="#4a5568"; bot="#718096" ;;   # grey veil
    *snow*)                     top="#9fb3c8"; bot="#e2e8f0" ;;
esac

mkdir -p "$OUT_DIR"
magick -size 1600x900 gradient:"$top"-"$bot" "$OUT" || exit 1
exec "$HOME/.local/bin/wallpaper.sh" "$OUT"
