# Testing on the real machine

Everything on this branch is testable on your Hyprland desktop — nothing
needs the CI container. Follow top to bottom; each step says what you
should SEE, so a miss is immediately obvious.

## 0. Install from this branch

```sh
cd ~/justlinux            # your clone
git fetch origin
git checkout feature/better-ui
./install.sh              # backs up ~/.config, builds rust/, installs symlinks
```

Then reload Hyprland (`ALT+X` → won't exist yet on first install — run
`hyprctl reload` in a terminal, or log out/in once). If waybar was running
it keeps the old process: `~/.local/bin/hypr-tools.sh restart-bar`.

> Rollback at any time: the originals are in `legacy/bin/`
> (`cp legacy/bin/* ~/.local/bin/`), and install.sh backed up your old
> `~/.config/*` as `*.bak-<timestamp>`.

## 1. Migration sanity (5 minutes, from the optimize branch work)

| Press | Expect |
|---|---|
| `ALT+X` | Settings panel opens **fast** (no python pause) in a glass float |
| `ALT+R` | App launcher with icon grid (icons need kitty + `magick` installed) |
| `ALT+W` / `ALT+D` | Window switcher / tools hub |
| `ALT+A` twice | Windows stash away, then come back in the same layout order |
| `ALT+SHIFT+A`, then `ALT+CTRL+A` | Focused window hides; picker brings it back to its workspace |
| `ALT+SHIFT+S` | Region screenshot → notification with image preview |
| `ALT+SHIFT+W` | Random wallpaper + whole desktop recolors |
| `ALT+B` | Bar hides/shows (pins if auto-hide daemon is on) |
| waybar shield/bug icons | still show firewall/ClamAV state |

## 2. New UI features on this branch

### Bar style switcher
`ALT+X` → **Bar** → "Bar style" card → click **Islands**, then **Glass**,
then **Minimal**, then **Default**.
- Each click restarts waybar (≈0.5 s flicker) with a visibly different look.
- The active style shows a ● and stays after `ALT+SHIFT+W` (wallpaper
  change recolors it, but the layout stays).
- Files involved if something looks wrong: `~/.config/waybar/style.css`
  (first line says which style is active) and `~/.config/waybar/styles/`.

### Animation profiles
`ALT+X` → **Theme** → "Animations" → click **Snappy**: open/close a few
windows — short, no overshoot. Click **Smooth** — floatier, windows pop in.
**Off** — instant. **Default** — the original feel.
- Survives reboot (it edits `~/.config/hypr/animations/current.conf`,
  which hyprland.conf sources).

### Appearance presets
`ALT+X` → **Appearance**: crank gaps/rounding to something extreme →
**Theme** → Slot 1 **Save**. Change appearance again → Slot 1 **Apply** —
the extreme look returns instantly (live). The slot line shows a summary
like `gaps 20/30 · border 3 · round 16 · blur✓…`.
- "Apply" is live-only: use Appearance → **Save (survives reboot)** to persist.

### System page
`ALT+X` → **System** (or key `6`): CPU sparkline moves (open a video to
spike it), Memory shows `used / total GB`.

### Launcher polish (`ALT+R`)
- Move the mouse over tiles **without clicking**: a subtle hover tint
  follows the pointer.
- Category headers and tile glyphs wear different palette hues
  (Favorites keep the main accent).
- Type `ffx` (or any skip-letter fragment): fuzzy matches still find
  Firefox-style names, listed after exact matches.
- Tools hub (`ALT+D`) and wallpaper picker get the same hover/accents.

### Wallpaper preview pane
`ALT+X` → Wallpaper → "Pick image / folder…" (or Tools → Pick wallpaper):
in the folder view, a **Preview** pane sits on the right showing the
selected image bigger, plus name/size — while the real desktop previews it
live, as before. Esc still restores the original wallpaper.
- The pane only appears when the float is ≥ 64 columns wide.

### Notifications
Send a test: `notify-send "hello" "world"` and
`notify-send -u critical "urgent" "look at me"` — rounded cards with an
accent bar on the left (red for critical). `ALT+N` opens the center;
hovering a notification tints it.

### Workspace overview (optional, needs plugin)
```sh
hyprpm update
hyprpm add https://github.com/hyprwm/hyprland-plugins
hyprpm enable hyprexpo
```
Uncomment `# source = ~/.config/hypr/hyprexpo.conf` in
`~/.config/hypr/hyprland.conf`, `hyprctl reload`, then `ALT+TAB` → grid of
live workspaces; click to jump. (Skip freely — everything else works
without it.)

## 3. Automated tests & benchmarks on your machine

```sh
cd ~/justlinux/rust && cargo test        # 80 unit+integration tests
cd ~/justlinux && ./bench/run.sh         # re-runs the old-vs-new benchmarks
```

`bench/run.sh` needs `python3` (and `pip install textual` for the legacy
TUI baseline). Numbers on real hardware are the ones that count — the
committed results came from a shared container (see
docs/MIGRATION.md → bias check).

## 4. If something breaks

- Panel won't open: run `~/.local/bin/hypr-settings` in a terminal — real
  error prints instead of a dead float.
- Bar style looks broken after a wallust recolor: `hypr-tools.sh
  restart-bar` (wallust rewrote colors.css; styles import it live).
- Anything else: `cp legacy/bin/* ~/.local/bin/` restores the pre-Rust
  tools while you file/describe the issue.
