#!/usr/bin/env bash
# Install these dots: backs up anything it would overwrite, then copies.
set -euo pipefail
cd "$(dirname "$0")"
STAMP=$(date +%Y%m%d-%H%M%S)

# Move aside whatever we are about to replace. Every install target below is
# replaced WHOLESALE, so anything not in this repo would otherwise vanish.
keep() {
    if [ -e "$1" ]; then
        echo "backup: $1 -> $1.bak-$STAMP"
        mv "$1" "$1.bak-$STAMP"
    fi
}

mkdir -p "$HOME/.config"
for c in config/*/; do
    [ -d "$c" ] || continue
    name=$(basename "$c")
    target="$HOME/.config/$name"
    keep "$target"
    cp -r "$c" "$target"
    echo "installed: ~/.config/$name"
done

mkdir -p "$HOME/.local/bin"
for f in bin/*; do
    # Regular files only. Running any of these scripts leaves a bin/__pycache__
    # behind, and `cp` without -r exits 1 on a directory — which under set -e
    # aborted the entire install right here. Configs landed, roughly half the
    # scripts landed, and everything below this loop silently never ran.
    [ -f "$f" ] || continue
    cp "$f" "$HOME/.local/bin/"
    chmod +x "$HOME/.local/bin/$(basename "$f")"
    echo "installed: ~/.local/bin/$(basename "$f")"
done

# The python package every bin/ script imports. Each one starts with
# sys.path.insert(~/.local/lib) and then `from hyprdesk import ...`, so
# without this a fresh machine gets all the scripts and every single one of
# them dies on import — the tools hub, the widgets, the studio, all of it.
#
# Installed per PACKAGE, never as the whole lib/ dir: ~/.local/lib also holds
# python3.13/site-packages, which is not ours to move aside.
mkdir -p "$HOME/.local/lib"
for p in lib/*/; do
    [ -d "$p" ] || continue
    name=$(basename "$p")
    target="$HOME/.local/lib/$name"
    keep "$target"
    cp -r "$p" "$target"
    rm -rf "$target/__pycache__"   # bytecode from THIS working copy
    echo "installed: ~/.local/lib/$name"
done

# The design system Claude Code reads before touching any of these surfaces.
# Per SKILL, never the whole skills dir — other skills live there too and are
# not ours to back up.
if [ -d claude/skills ]; then
    mkdir -p "$HOME/.claude/skills"
    for s in claude/skills/*/; do
        [ -d "$s" ] || continue
        name=$(basename "$s")
        target="$HOME/.claude/skills/$name"
        keep "$target"
        cp -r "$s" "$target"
        echo "installed: ~/.claude/skills/$name"
    done
fi

# terminal colors follow the wallpaper
LINE='[ -f ~/.cache/wallust/sequences ] && command cat ~/.cache/wallust/sequences'
grep -qF "wallust/sequences" "$HOME/.zshrc" 2>/dev/null || echo "$LINE" >> "$HOME/.zshrc"

echo
echo "Done. Log into Hyprland (SDDM) and press ALT+D for the tools hub."
