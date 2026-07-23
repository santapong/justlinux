#!/usr/bin/env bash
# claude-select.sh (ALT+SHIFT+N) — act on the current text selection with
# Claude. Launcher mode grabs the selection FIRST (the float would clear
# focus), then opens a glass kitty float running --ui.
RUN="${XDG_RUNTIME_DIR:-/tmp}"
SEL="$RUN/claude-sel.txt"
OUT="$RUN/claude-sel-out.txt"

if [ "$1" != "--ui" ]; then
    # primary selection first (what's highlighted), clipboard as fallback
    text=$(wl-paste -p 2>/dev/null) || text=""
    [ -z "$text" ] && text=$(wl-paste 2>/dev/null) || true
    if [ -z "$text" ]; then
        notify-send "Claude" "Nothing selected (and clipboard is empty)"
        exit 0
    fi
    printf '%s' "$text" > "$SEL"
    exec kitty --class hyprclaude -e "$0" --ui
fi

# ---------- UI mode (inside the float) ----------
text=$(cat "$SEL")
preview=$(printf '%s' "$text" | head -c 200)
printf '\033[1m  Claude — act on selection\033[0m\n\n'
printf '\033[2m%s%s\033[0m\n\n' "$preview" "$([ ${#text} -gt 200 ] && echo …)"
printf '  1  Explain this\n'
printf '  2  Fix / improve it\n'
printf '  3  Rewrite concisely\n'
printf '  4  Summarize\n'
printf '  5  Translate → Thai\n'
printf '  6  Translate → English\n'
printf '  7  Custom prompt…\n\n'
read -rn1 -p '  choice: ' ch; echo; echo
case "$ch" in
    1) prompt="Explain this clearly and briefly:" ;;
    2) prompt="Fix or improve this. Show the corrected version first, then a one-line note on what changed:" ;;
    3) prompt="Rewrite this concisely, keeping the meaning:" ;;
    4) prompt="Summarize this in a few bullets:" ;;
    5) prompt="Translate this to Thai (natural, not literal):" ;;
    6) prompt="Translate this to English (natural, not literal):" ;;
    7) read -rp '  prompt: ' prompt ;;
    *) exit 0 ;;
esac

printf '\033[2m  thinking…\033[0m\n\n'
claude -p "$prompt" < "$SEL" | tee "$OUT"
printf '\n\033[2m  [c] copy result · any other key closes\033[0m\n'
read -rn1 k
[ "$k" = "c" ] && wl-copy < "$OUT" && notify-send "Claude" "Result copied"
exit 0
