#!/usr/bin/env bash
# waybar status for "a Claude session needs you" (JSON)
# reads ~/.cache/hyprdesk/office.json written by the office poll loop —
# empty text unless something is actually needy, so the badge stays gone
# on every monitor until there's a real reason to look.
CACHE="$HOME/.cache/hyprdesk/office.json"
STALE=120   # seconds; older than this the office poll is presumed dead

empty() { echo '{"text":"","tooltip":""}'; exit 0; }

[ -r "$CACHE" ] || empty

now=$(date +%s)
mtime=$(stat -c %Y "$CACHE" 2>/dev/null) || empty
[ $((now - mtime)) -le $STALE ] || empty

needy=$(jq -r '.needy // 0' "$CACHE" 2>/dev/null) || empty
case "$needy" in
    ''|*[!0-9]*) empty ;;
esac
[ "$needy" -gt 0 ] || empty

tooltip=$(jq -r '.tooltip // empty' "$CACHE" 2>/dev/null)
# the office is already running whenever this badge is visible (it is the
# only writer of the cache), so the click RAISES it — don't promise "open"
[ -n "$tooltip" ] || tooltip="Claude: ${needy} session(s) need you"

jq -n --arg text "󰚩 ${needy}" --arg tooltip "$tooltip" \
    '{"text":$text,"tooltip":$tooltip,"class":"needy"}'
