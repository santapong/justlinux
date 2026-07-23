#!/usr/bin/env bash
# claude-vision.sh (ALT+SHIFT+I) — select a screen region and have Claude
# LOOK at it (diagnose an error, explain a diagram/UI), not just OCR it.
# Sibling of ALT+I ocr-region: same gesture, reasoning instead of text.
RUN="${XDG_RUNTIME_DIR:-/tmp}"
IMG="$RUN/claude-vision.png"
OUT="$RUN/claude-vision-out.txt"

if [ "$1" != "--ui" ]; then
    geo=$(slurp) || exit 0          # user cancelled
    grim -g "$geo" "$IMG" || { notify-send "Claude vision" "capture failed"; exit 1; }
    exec kitty --class hyprclaude -e "$0" --ui
fi

# ---------- UI mode (inside the float) ----------
printf '\033[1m  Claude — looking at your selection…\033[0m\n\n'
claude -p "Look at the screenshot at $IMG and explain what it shows.
If it is an error message, stack trace or broken UI: diagnose the likely
cause and suggest a fix. If it is a diagram or chart: explain it briefly.
Answer compactly." --allowedTools "Read" | tee "$OUT"
printf '\n\033[2m  [c] copy answer · any other key closes\033[0m\n'
read -rn1 k
[ "$k" = "c" ] && wl-copy < "$OUT" && notify-send "Claude vision" "Answer copied"
exit 0
