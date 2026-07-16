#!/usr/bin/env bash
# Install these dots: backs up anything it would overwrite, then copies.
set -euo pipefail
cd "$(dirname "$0")"
STAMP=$(date +%Y%m%d-%H%M%S)

for c in config/*/; do
    name=$(basename "$c")
    target="$HOME/.config/$name"
    if [ -e "$target" ]; then
        echo "backup: $target -> $target.bak-$STAMP"
        mv "$target" "$target.bak-$STAMP"
    fi
    mkdir -p "$HOME/.config"
    cp -r "$c" "$target"
    echo "installed: ~/.config/$name"
done

mkdir -p "$HOME/.local/bin"
for f in bin/*; do
    cp "$f" "$HOME/.local/bin/"
    chmod +x "$HOME/.local/bin/$(basename "$f")"
    echo "installed: ~/.local/bin/$(basename "$f")"
done

# terminal colors follow the wallpaper
LINE='[ -f ~/.cache/wallust/sequences ] && command cat ~/.cache/wallust/sequences'
grep -qF "wallust/sequences" "$HOME/.zshrc" 2>/dev/null || echo "$LINE" >> "$HOME/.zshrc"

echo
echo "Done. Log into Hyprland (SDDM) and press ALT+D for the tools hub."
