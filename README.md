# Hyprland Dots — Kali Linux

My Hyprland 0.55 desktop on Kali Linux (Intel UHD 630, 3× 1600×900 monitors),
with a set of **custom-built TUI panels** instead of the usual rofi-for-everything:
every menu is a click-and-pick interface running in a glass kitty float,
auto-themed from the wallpaper by wallust.

## The custom apps (bin/)

| App | Bind | What it is |
|---|---|---|
| `hypr-settings` | `ALT+X` | Settings panel — sidebar + cards. Live gaps/border/rounding steppers with gauges, blur/shadow/animation switches, Save writes back to `hyprland.conf`, bar controls, wallpaper, firewall/ClamAV status, power page with confirm dialogs |
| `hypr-launcher apps` | `ALT+R` | App launcher — **icon grid** (real icons via kitty graphics protocol), collapsible categories, ★ favorites (right-click to star), recent apps, 10-per-row |
| `hypr-launcher windows` | `ALT+W` / `ALT+H` | Window overview — every workspace's windows in one filterable list; hidden/stashed windows (`special:*`) are labeled and sorted first, Enter restores them to the active workspace |
| `hypr-launcher wallpaper` | via Tools | Wallpaper picker — monitor tiles show each screen's current wallpaper, thumbnail grid, **live preview on the real desktop** while you browse, Esc restores |
| `hypr-launcher menu` | `ALT+D` | Tools hub — every desktop tool as a tile with live status ("ufw is ON") and keybind hints |
| `hypr-tools.sh` | — | The glue: dispatcher for all of the above + reminders (systemd timers), workspace stash, per-window hide/unhide, bar reorder, keybind cheatsheet |
| `wallpaper.sh` | `ALT+SHIFT+W` | Random/pick wallpaper + wallust recolor of the whole desktop (hypr borders, waybar, rofi, swaync — live) |
| smart top bar | `ALT+B` pins | waybar auto-hide, owned by `hypr-appdock` (`bar_smart=on`): hover the top screen edge to fade the bar in (250ms), leave to fade out; `ALT+B` pins it open / releases it; toggle from the Tools hub. (`waybar-autohide.sh` is the retired standalone predecessor) |
| `screenshot.sh` | `ALT+SHIFT+S` | Region/screen/all screenshots → file + clipboard + notification |
| `drop-claude` | `ALT+SHIFT+U` | Dropdown **Claude Code** terminal (guake-style, keeps its session) — the AI sibling of `ALT+U` drop-term |
| `hypr-claude-studio` | `ALT+CTRL+U` | **Claude Studio** — a VS-Code-style workspace: expandable session tree (🟢 running · 󰑮 background · projects → conversations) in tab 0, every opened session is its own **tab** named from Claude's own title for the conversation, with a **✕ to close it**. Closing a tab ends the terminal, not the conversation — it stays resumable from the tree. `n` = new session, `x` = stop a background one |
| `hypr-launcher claude` | Tools hub | Quick session picker — the flat filterable list version of the same data |
| `hypr-claude-office` | `ALT+CTRL+O` | **The 2D Claude office** — one pixel-art desk per live Claude session, each labelled with what that session *is*. States come from Claude's own job state, not a CPU guess; a desk that needs you is tinted and the header counts them. Clicking a desk opens that session, clicking anywhere else opens the Studio. Full detail in [`docs/claude-office.md`](docs/claude-office.md) |
| `claude-select.sh` | `ALT+SHIFT+N` | Act on the selected text with Claude: explain / fix / rewrite / summarize / translate (Thai ⇄ English) / custom — result in a glass float, `c` copies |
| `claude-vision.sh` | `ALT+SHIFT+I` | Select a region → Claude **looks** at it (diagnoses errors, explains diagrams/UI) — the reasoning sibling of `ALT+I` OCR |
| `focus-mode.sh` | `ALT+SHIFT+F` | Deep-work session: notifications muted, countdown card on the desktop, auto-ends via systemd timer and reports what queued up |
| `hypr-viz` | `ALT+SHIFT+Y` | Ambient audio visualizer — glass spectrum bars on the desktop (PipeWire sink monitor, pure-Python Goertzel, zero deps), wallust-colored |
| `wallpaper-ambient.sh` | — | Sky-reactive wallpaper: hourly gradient matched to time-of-day + live weather, cascaded through wallust so the whole desktop follows the sky (`on`/`off` toggles the timer) |

> **Scope**: this repo is the *desktop* — dotfiles, widgets, panels. Robot work
> lives in `~/ros2_ws` and the RoboLLM repo, not here.

## Architecture

C4-model diagrams (Context → Containers → Components → Dynamic) live in
[`docs/architecture.md`](docs/architecture.md) — start there to see how the
card host, docks, smart bar, TUI panels and the single `widgets.conf`
source of truth fit together.

[`docs/claude-office.md`](docs/claude-office.md) — how the Claude office
decides what each desk shows: where its facts come from, how a session is
bound to a conversation, how "needs you" is derived, and every `office_*` key.

## Requirements

```sh
# Debian/Kali packages
sudo apt install hyprland hyprpaper hyprlock hypridle waybar rofi swaync \
                 kitty thunar grim slurp wl-clipboard cliphist brightnessctl \
                 imagemagick lxpolkit network-manager-gnome sddm

# wallust (wallpaper -> colors) via cargo
cargo install wallust --locked

# TUI framework for the custom panels
pip install --user --break-system-packages textual textual-image
```

Fonts: JetBrainsMono Nerd Font. Icon theme: Flat-Remix-Blue-Dark.

## Install

```sh
./install.sh    # backs up existing configs, copies everything into place
```

Then log into Hyprland (SDDM session). Notes:
- `config/hypr/colors.conf`, `config/waybar/colors.css` etc. are **generated by
  wallust** — they'll be overwritten on the first wallpaper change.
- LightDM cannot start Hyprland (compositor deadlock) — use SDDM.
- SDDM sets no locale for Wayland sessions; `hyprland.conf` exports
  `LANG=en_US.UTF-8` for that reason.

## Keybinds ($mod = ALT)

| Keys | Action |
|---|---|
| `ALT+Q` / `ALT+C` | terminal / close window |
| `ALT+R` / `ALT+W` / `ALT+E` | apps / windows / files |
| `ALT+D` / `ALT+X` | tools hub / settings panel |
| `ALT+ESC` | power page (lock/logout/reboot/shutdown) |
| `ALT+A` / `ALT+SHIFT+A` / `ALT+CTRL+A` | stash workspace / hide window / unhide |
| `ALT+S` / `ALT+CTRL+S` | scratchpad toggle / send to scratchpad |
| `ALT+B` | pin the smart bar open / release it |
| `ALT+H` | window overview (all workspaces + restore hidden) |
| double-click a titlebar | maximize toggle (hyprbars; keeps waybar + gaps) |
| `ALT+T` / `ALT+K` | reminder / keybind cheatsheet |
| `ALT+SHIFT+U` / `ALT+SHIFT+N` / `ALT+SHIFT+I` | Claude: dropdown / selection actions / region vision |
| `ALT+CTRL+U` / `ALT+CTRL+O` | Claude Studio (tabbed workspace) / 2D Claude office |
| `ALT+SHIFT+F` / `ALT+SHIFT+Y` | focus session / audio visualizer |
| `ALT+SHIFT+S` / `PRINT` | region screenshot |
| `ALT+1-0`, `ALT+SHIFT+1-0` | workspace switch / move |
| `ALT+G`, `ALT+SHIFT+TAB` | window group (tabs) / cycle tabs |

Everything was built and verified with automated tests (Textual pilot harness,
`HYPRSETTINGS_DRYRUN=1`).

## Desktop widget cards (hyprcard)

All 14 desktop widgets (clock, stats, calendar, netgraph, weather, github,
trading, nowplaying, security, devgit, robotics, claude usage, hidden-window
stash, focus countdown) are
glass cards rendered by **one process** — `bin/hypr-cardhost` — from
declarative TOML templates in `config/hyprcard/templates/`. The old
per-widget conky fleet is retired.

| Piece | Role |
|---|---|
| `bin/hypr-cardhost` | the card host: async data sources, grid placement, error/stale cards, live retheme (`--ctl ping\|reload\|reload-theme`) |
| `bin/hypr-arrange` | **ALT+SHIFT+E** — grid edit mode with **two snap modes**, `g` switches: *GRID* lands every surface on a cell, *FREE* gives pixel placement with magnetic alignment guides. Drag anything — cards, the office, the visualizer, the docks — Enter saves, Esc cancels, `--undo` reverts the last save |
| `bin/hypr-widgetpicker` | template gallery with live previews, params, per-monitor add + manage (also: Settings → Widgets → *＋ Add widget…*) |
| `bin/hypr-appdock` | one auto-hiding dock per monitor (touch the bottom screen edge to reveal), per-monitor pins — plus the **smart top bar**: `bar_smart=on` makes waybar itself auto-hide with top-edge dwell reveal, `ALT+B` pin, and a fade animation |
| `lib/hyprdesk/` | shared modules: `rows.py` renderers, `cardspec.py` templates, `grid.py` cells, `confwrite.py` atomic config writer, `pixicons.py` icon bitmaps, `monitors.py`, `layer.py`, `theme.py` |
| `lib/hyprdesk/pixicons.py` | **pixel icons** — generated bitmaps from [pixelarticons](https://github.com/halfmage/pixelarticons) (MIT). Rasterised offline so every icon lands on whole pixels; colour comes from the caller, so wallust still drives them. A row gets one with `icon = "cpu"`. Regenerate with `gen_pixicons.py` |
| `bin/widget-*.sh` | data fetchers — `--fields` emits `key=value` for the host; no flag emits legacy conky markup (rollback) |

**Settings** live in `config/conky/widgets.conf` (single source of truth —
the path is legacy, the file is NOT conky-specific): `inst_<id>=<template>`
registers an instance, `<id>=on|off` toggles it, `<id>_mon/_col/_row`
place it on the grid, `<id>_p_<param>=…` parameterizes it (e.g.
`trading_p_coins=bitcoin,ethereum`). New widget = drop a `.toml` template
+ add an `inst_` line (or use the picker).

**Rollback**: the conky twin configs (`config/conky/widgets/*.conf`,
`card.lua`) ship until final acceptance; `desktop-widgets.sh` runs any
widget whose stem has **no** `inst_` twin in widgets.conf, so reverting a
single widget = delete its `inst_<name>` line and restart. Full revert =
`git checkout` the pre-hyprcard commit of `bin/` + `config/` (widgets.conf
keeps user placement either way).
