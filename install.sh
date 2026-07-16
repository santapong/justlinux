#!/usr/bin/env bash
# Install these dots: backs up anything it would overwrite, then copies.
# The desktop tools are one Rust binary (rust/) installed as symlinks that
# keep the original script names, so keybinds & waybar hooks stay valid.
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

# --- build the tools binary (cargo is already required, for wallust) ---
command -v cargo >/dev/null || {
    echo "cargo not found — install rustup (https://rustup.rs), it is also needed for wallust"
    exit 1
}
export PATH="$HOME/.cargo/bin:$PATH"
echo "building justlinux (rust/)…"
(cd rust && cargo build --release)

mkdir -p "$HOME/.local/bin"
install -m755 rust/target/release/justlinux "$HOME/.local/bin/justlinux"
echo "installed: ~/.local/bin/justlinux"

# applet symlinks — SAME names as the old scripts (hyprland.conf keybinds,
# waybar exec hooks and `pgrep -f waybar-autohide.sh` depend on them)
for name in hypr-tools.sh hypr-settings hypr-launcher wallpaper.sh \
            screenshot.sh bar-toggle.sh waybar-autohide.sh \
            av-status.sh fw-status.sh; do
    ln -sf "$HOME/.local/bin/justlinux" "$HOME/.local/bin/$name"
    echo "installed: ~/.local/bin/$name -> justlinux"
done

# terminal colors follow the wallpaper
LINE='[ -f ~/.cache/wallust/sequences ] && command cat ~/.cache/wallust/sequences'
grep -qF "wallust/sequences" "$HOME/.zshrc" 2>/dev/null || echo "$LINE" >> "$HOME/.zshrc"

echo
echo "Done. Log into Hyprland (SDDM) and press ALT+D for the tools hub."
