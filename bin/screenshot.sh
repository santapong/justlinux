#!/usr/bin/env bash
# screenshot.sh region|screen|all — save to ~/Pictures/Screenshots, copy to clipboard, notify
set -euo pipefail
DIR="$HOME/Pictures/Screenshots"
mkdir -p "$DIR"
f="$DIR/$(date +%F_%H-%M-%S).png"

case "${1:-region}" in
    region)
        g=$(slurp) || exit 0          # Esc cancels quietly
        grim -g "$g" "$f"
        what="region" ;;
    screen)
        mon=$(hyprctl activeworkspace -j | python3 -c "import json,sys; print(json.load(sys.stdin)['monitor'])")
        grim -o "$mon" "$f"
        what="this screen" ;;
    all)
        grim "$f"
        what="all screens" ;;
esac

wl-copy < "$f"
notify-send -i "$f" "Screenshot ($what)" "Copied to clipboard + saved:\n${f/#$HOME/~}"
