---
name: hyprland-ui-ux
description: UX/UI design rules for GUI work on this Hyprland desktop — layer-shell surface patterns, the widget fleet's ink/color system, direct-manipulation and interaction heuristics, and the QA checklist. Use when building or changing desktop widgets, layer surfaces, the pet, the dock, Hypr Settings/Launcher pages, or any on-screen GUI element on this machine.
---

# Hyprland UI/UX — this machine's design system

Everything on-screen must feel like ONE desktop: glass surfaces, wallust-driven
color, JetBrainsMono NF, no window chrome where a layer surface will do.

## Architecture (never bypass)

- Settings: ONLY `~/.config/conky/widgets.conf`, read via `hyprdesk.conf()`
  (python) / `card.lua conf()` (conky). One file, one truth.
- Colors: ONLY `hyprdesk.colors()` / `card.colors()` — theme-aware
  (wallust ⭢ follows wallpaper; else `~/.config/conky/themes/<t>.lua`).
- Layer surfaces: ONLY `hyprdesk.layer.LayerWindow` (ctypes gtk-layer-shell;
  no GIR typelib exists here). Conky widgets position via `card.place`.
- New conky widget = drop a `.conf` in `~/.config/conky/widgets/` → it
  self-registers in Hypr Settings (toggle + layout row). Keep it that way.

## Ink & color (validated — don't re-derive by eye)

| Slot | Role | Source |
|---|---|---|
| color1 | titles, section icons | accent2 |
| color2 | key values (primary ink) | fg |
| color3 | secondary labels, separators | `sub` = mix(fg,bg,0.62) — ~7:1 |
| color4 | status GOOD only | #8EC07C (theme-overridable) |
| color5 | status BAD only | #E06C75 |

- The wallust `muted` slot (~1.5:1) is a SURFACE color. Never text.
- Status colors never decorate non-status content.
- Validate new palettes with the dataviz skill's `validate_palette.js`
  (dark surface = theme bg). Text needs ≥4.5:1; 3:1 absolute floor.
- Card glass: bg tint argb 205 (~80%) + Hyprland `layerrule blur` +
  `ignore_alpha 0.3` on the surface namespace. GTK surfaces get rounded
  corners via CSS border-radius; conky cannot round (no layer rounding
  in Hyprland 0.55, no drawable for lua-cairo on wayland).

## Layer-shell UX patterns

- **Display-only surface** (widgets): empty input region
  (`input_shape_combine_region(cairo.Region())`) — it must NEVER eat clicks.
- **Interactive desktop element** (pet, dock): input region EXACTLY over the
  interactive pixels, updated when they move. Everything else stays
  click-through. Bottom layer = only reachable when the desktop shows;
  provide a windowed/settings fallback for every action (the dock's picker
  is also a Settings button because of this).
- **Modal editor/overlay**: `layer="overlay"` (renders above fullscreen),
  keyboard mode on-demand for Esc/Enter, dim the backdrop, always both
  cancel (Esc) and save (Enter/button) paths.
- Never steal keyboard focus from the compositor for passive surfaces.
- Windowed companions (pickers, editors) get an app_id via
  `GLib.set_prgname` + a hyprland windowrule float.

## Interaction heuristics

- Direct manipulation first: drag beats steppers, click beats menu, but keep
  the indirect path too (steppers/settings) for reachability under windows.
- Click targets ≥ 40×40 px on desktop surfaces (Fitts: bottom/corner edges
  are cheap, use them).
- Disambiguate click vs drag with a 6px / 200ms threshold; double-click via
  GDK `_2BUTTON_PRESS`.
- Every state change the user triggers must give visible feedback within
  150ms (animation frame, notify, or color change).
- Anything that can fail (network, D-Bus, APIs) must render a graceful
  fallback card — a blank widget is a bug. Mark stale cached data `(stale)`.

## Gotchas that already bit us (QA checklist)

- Lua 5.4 `string.format("%02X", float)` hard-errors — `math.floor(x+0.5)`.
- conky `${execpi}` re-parses `$` — escape `$`→`$$` in ALL external strings
  (API text, track titles, prices); use `${execi}` for plain text files.
- A childless app-paintable Gtk.Window never emits draw → use DrawingArea.
- GTK allocation doesn't follow layer-shell stretch anchors → size windows
  from Gdk monitor geometry explicitly.
- GdkPixbuf loaders crash on odd files → wrap every load (`safe_pixbuf`).
- MPRIS names may be `org.mpris.MediaPlayer2.app.instanceN` → first segment.
- `Gio.bus_get_sync` outside try = blank widget on headless/bus failure.
- Unix-socket event streams need a residual line buffer + EOF handling
  (empty recv → remove watch, fall back to polling).
- Shared state files: write temp + `os.replace`; corrupt-read fallback must
  NOT overwrite the file.
- pkill/pgrep -f from automation self-matches the invoking shell — use
  `pkill -xf "python3 /full/path"` or bracket patterns.
- Conky/GTK layer surfaces attach to the FOCUSED monitor at spawn.
- Bottom-anchored cards need y ≥ 68 to clear the 56px pet strip.

## Verification

- Pilot-test TUI pages headlessly (`HYPRSETTINGS_DRYRUN=1`, `run_test()`,
  `save_screenshot()` → magick → PNG).
- Render sprites/frames offscreen to PNG (cairo ImageSurface) and LOOK.
- `hyprctl layers` geometry proves placement/overlap; screenshots via grim.
- For fleet-wide changes: multi-agent audit (one agent per widget, schema'd
  findings, layout-overlap checker) — it catches what a single pass misses.
