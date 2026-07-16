# UI research — Hyprland showcases worth stealing from

Curated for the `feature/better-ui` work. Part 1–3 list the showcases and
what each does well; Part 4 turns that into a concrete, ranked backlog
mapped to this repo's files. Links verified via web search July 2026.

---

## Part 1 — Full desktop showcases (the famous rices)

### [end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) — "illogical impulse"
Widely considered the most visually striking Hyprland setup. Dropped
waybar entirely for **Quickshell (QML/Qt6)**: bar, sidebars, and widgets
are one coherent shell with mobile-OS-fluid animations. Two complete panel
families ("ii" and the Windows-like "waffle"). **Material You theming**:
tonal palettes generated from the wallpaper recolor everything. Extras:
workspace drag-and-drop, screen translation, "anti-flashbang".
Docs: [ii.clsty.link](https://ii.clsty.link/en/).
**Steal:** the *coherence* — one design language across bar/panels/OSD;
tonal palette (not raw hue extraction) from the wallpaper; overview widget.

### [mylinuxforwork/dotfiles](https://github.com/mylinuxforwork/dotfiles) — ML4W
The "ready-to-work" polished suite. Its killer feature matches this
repo's philosophy: a **Settings App** exposing waybar time/date format,
module toggles, workspace count, blur/animation variations, wallpaper
effects, idle times, keybindings — all click-and-pick. Ships a
**waybar theme collection** with a starter theme + `config-custom`/
`style-custom.css` override convention, and a "Global Glass" look.
Docs: [ml4w.com](https://www.ml4w.com/), [waybar customization](https://mylinuxforwork.github.io/dotfiles/customization/waybar).
**Steal:** waybar *theme switching* as a first-class settings page; the
custom-override file convention so user tweaks survive updates.

### [JaKooLit/Hyprland-Dots](https://github.com/JaKooLit/Hyprland-Dots)
The community all-in-one. Uses **wallust** exactly like this repo, but
pushes it further: a whole [theme-switching layer](https://deepwiki.com/JaKooLit/Hyprland-Dots/6.6-theme-switching-utilities)
with dark/light toggle, **live-preview rofi theme selector**
(SUPER+CTRL+R), and waybar **styles AND layouts swappable at runtime**
(symlink `selected.css` → restart bar — same trick our `restart-bar`
already enables). Gallery: [wiki](https://github.com/JaKooLit/Hyprland-Dots/wiki/Gallery).
**Steal:** runtime waybar layout/style swap; rofi theme live preview;
dark/light mode toggle driven through wallust.

### [prasanthrangan/hyprdots](https://github.com/prasanthrangan/hyprdots) → [HyDE-Project](https://github.com/HyDE-Project)
The theming *framework* approach: themes are modular, shareable packages
(hyde-themes repo, `themepatcher` installer) covering GTK/icons/cursor plus
gaps/border/rounding/blur presets per theme. [Theming wiki](https://github.com/prasanthrangan/hyprdots/wiki/Theming).
**Steal:** the idea that a "theme" bundles *appearance numbers* (gaps,
rounding, blur) with colors — our settings panel already live-edits those
numbers, so theme presets are a natural extension.

### [caelestia-dots/shell](https://github.com/caelestia-dots/shell)
"A fluid, morphing shell" (Quickshell). Top-center **system panel** with
CPU/GPU/RAM meters, media controller and weather; bottom-right **quick
toggles** (Wi-Fi/BT/light-dark); workspace switcher with window pinning;
launcher doubles as wallpaper picker driven by IPC.
**Steal:** quick-toggles cluster concept for our Tools hub tiles; meters
in the settings panel (ratatui has gauges/sparklines natively).

### [AvengeMedia/DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell)
Quickshell+Go shell that **replaces waybar, swaylock, swayidle, mako,
fuzzel and polkit at once** — Material 3 panels, automatic theming, a
plugin/theme market ([danklinux.com/plugins](https://danklinux.com/plugins)).
Write-up: [sudoscience.blog](https://sudoscience.blog/2026/01/24/dank-material-shell-makes-hyprland-feel-complete/).
**Steal:** notification popups, OSD and lock screen sharing one palette —
our wallust templates could grow hyprlock + OSD targets.

### [noctalia-dev/noctalia](https://github.com/noctalia-dev/noctalia)
"Sleek and minimal" full shell: bars, panels, launcher, notifications,
dock, lock screen, idle, OSDs, wallpapers, multi-monitor surfaces —
restrained, dark, rounded aesthetic rather than maximalist.
**Steal:** the minimal look is the closest to this repo's glass-TUI vibe;
good reference for spacing/radius restraint.

---

## Part 2 — Component galleries

### Waybar
- [sejjy/mechabar](https://github.com/sejjy/mechabar) — mecha/cyberpunk modular bar, Catppuccin variants, kitty-based menus. Nice pill-cluster layout.
- [Waybar wiki examples](https://github.com/Alexays/Waybar/wiki/Examples) — the canonical style gallery.
- [waybar-themes topic](https://github.com/topics/waybar-themes) — browsable collections (incl. Omarchy-inspired sets).

### Lock screen (hyprlock)
- [JayeshVegda/awesome-hyprlock](https://github.com/JayeshVegda/awesome-hyprlock) — curated showcase of lockscreen setups with screenshots.
- [MrVivekRajan/Hyprlock-Styles](https://github.com/MrVivekRajan/Hyprlock-Styles) — big ready-to-use style collection.
- [mahaveergurjar/Hyprlock-Dots](https://github.com/mahaveergurjar/Hyprlock-Dots) — layouts with music/weather/battery widgets via scripts.
- [catppuccin/hyprlock](https://github.com/catppuccin/hyprlock), [Tamarindtype/googlish-hyprlock-theme](https://github.com/Tamarindtype/googlish-hyprlock-theme) (MD3-style), [Thunder-Blaze/BlazinLock](https://github.com/Thunder-Blaze/BlazinLock) (auto-theming, works with hyprdots/end-4 installs).

### Compositor eye-candy (plugins, via `hyprpm`)
- [hyprwm/hyprland-plugins](https://github.com/hyprwm/hyprland-plugins) — official set:
  **hyprexpo** (exposé grid of live workspace previews with smooth
  enter/exit animations — [overview](https://deepwiki.com/hyprwm/hyprland-plugins/6.1-hyprexpo)),
  **borders-plus-plus** (1–2 extra borders for a layered ring look).
- [KZDKM/Hyprspace](https://github.com/KZDKM/Hyprspace) — macOS/KDE-style workspace overview: minimap, drag windows between workspaces, animation overrides.
- Plugin directory: [hypr.land/plugins](https://hypr.land/plugins/).

### OSD / notifications / widgets (from [awesome-hyprland](https://github.com/hyprland-community/awesome-hyprland))
- **SwayOSD** — GNOME-like volume/brightness OSD (GTK); **Avizo** — macOS-like; **wob** — minimal bar OSD.
- Notification daemons: swaync (already used here), mako, dunst, fnott.
- Widget frameworks if ever needed beyond TUI: eww (Rust+GTK), Quickshell (QML), ironbar.
- Full utilities index: [Hyprland wiki — Useful Utilities](https://wiki.hypr.land/Useful-Utilities/), [Status bars](https://wiki.hypr.land/Useful-Utilities/Status-Bars/).

---

## Part 3 — TUI polish (our panels are ratatui, not QML)

The panels here are terminal UIs in glass kitty floats — the relevant
showcase is the Rust TUI world, not GTK/QML:

- [ratatui App Showcase](https://ratatui.rs/showcase/apps/) and
  [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) — hundreds
  of polished TUIs; look at **Television** (fuzzy finder) for
  launcher-grade list ergonomics and preview panes.
- [ratatui examples](https://github.com/ratatui/ratatui/tree/main/examples)
  — official demos of gauges, sparklines, charts, tabs, scrollbars,
  animation ticks — all widgets we already ship but barely use.

---

## Part 4 — Ranked backlog for THIS repo

Ordered by visual impact ÷ effort. File references are where the change lands.

1. **Waybar style switcher** (ML4W/JaKooLit pattern) — ship 3–4 CSS
   variants (`config/waybar/styles/*.css`: current, pill/floating islands,
   glass full-width, minimal text-only), all consuming the existing wallust
   `colors.css`. Add a "Bar style" picker card to the settings Bar page
   (`rust/src/applets/settings.rs`) that symlinks the choice and calls the
   existing `restart-bar`. *Effort: S. Impact: the whole desktop reads differently.*

2. **Workspace overview** — enable **hyprexpo** (official plugin) with a
   grid gesture/keybind (`config/hypr/hyprland.conf`), or Hyprspace for
   drag-between-workspaces. Pairs perfectly with the existing stash flow
   (ALT+A). Add a Tools-hub tile (`rust/src/applets/launcher.rs`
   `tool_entries`). *Effort: S (config-only). Impact: high.*

3. **Volume/brightness OSD** — add SwayOSD (or wob for zero-GTK
   minimalism), bind XF86 keys in `hyprland.conf`, template its colors from
   wallust (`config/wallust/templates/`). *Effort: S.*

4. **hyprlock glow-up** — current `hyprlock.conf` is functional; borrow a
   layout from awesome-hyprlock / Hyprlock-Styles (clock + avatar + blurred
   current wallpaper + input ring) and template its palette via wallust so
   the lock screen recolors with the desktop. *Effort: S–M.*

5. **Launcher visual pass** (`rust/src/applets/launcher.rs`) —
   Television-style: selected-tile accent glow (bold border + subtle bg
   tint exists; add per-category accent hue), hover highlight on mouse
   motion (crossterm reports Moved events), fuzzy matching (currently
   prefix/word/substring tiers), a right-side preview pane in wallpaper
   mode (kitty graphics already in place — show the selection at 2× size),
   and animated section expand (2–3 frame ease on the tick timer).
   *Effort: M. Impact: it's the most-touched UI (ALT+R/W/D).*

6. **Settings panel: theme presets + meters**
   (`rust/src/applets/settings.rs`) — a "Presets" card (HyDE idea): save/
   load named bundles of gaps/border/rounding/blur/animations (JSON in
   `~/.config/hypr/presets/`), applied live via the existing keyword path.
   Add a small CPU/RAM sparkline card (caelestia idea) — ratatui
   `Sparkline` + `/proc/stat`, zero new deps. *Effort: M.*

7. **Animation profiles** — ship 2–3 named `animations {}` blocks as
   include files (`config/hypr/animations/*.conf`, end-4-style bezier
   curves: snappy / smooth / off) selected from the settings Appearance
   page. *Effort: S–M.*

8. **borders-plus-plus accent ring** — double border with
   wallust accent + surface colors for the layered look. *Effort: S.*

9. **Notification style pass** — swaync is already wallust-templated;
   tighten the CSS toward the noctalia look (larger radius, thin accent
   left-bar per urgency, progress bars for volume-change notifications).
   *Effort: S.*

10. **Material-You tones** (end-4/DMS idea, stretch) — wallust gives raw
    palette extraction; generating a *tonal* scale (surface tints at
    several luminance steps) from the accent would make cards/borders/
    hover states feel designed rather than sampled. Could be a small pure
    function in `rust/src/colors.rs` (HSL lightness ladder) feeding the
    TUI styles — no new dependency. *Effort: M. Impact: subtle but everywhere.*

## Sources

Search-verified July 2026: [end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) · [illogical-impulse docs](https://ii.clsty.link/en/) · [ML4W dotfiles](https://github.com/mylinuxforwork/dotfiles) · [ML4W waybar docs](https://mylinuxforwork.github.io/dotfiles/customization/waybar) · [JaKooLit/Hyprland-Dots](https://github.com/JaKooLit/Hyprland-Dots) · [JaKooLit theme utilities](https://deepwiki.com/JaKooLit/Hyprland-Dots/6.6-theme-switching-utilities) · [prasanthrangan/hyprdots](https://github.com/prasanthrangan/hyprdots) · [HyDE-Project](https://github.com/HyDE-Project) · [caelestia-dots/shell](https://github.com/caelestia-dots/shell) · [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) · [noctalia](https://github.com/noctalia-dev/noctalia) · [mechabar](https://github.com/sejjy/mechabar) · [Waybar examples](https://github.com/Alexays/Waybar/wiki/Examples) · [awesome-hyprlock](https://github.com/JayeshVegda/awesome-hyprlock) · [Hyprlock-Styles](https://github.com/MrVivekRajan/Hyprlock-Styles) · [Hyprlock-Dots](https://github.com/mahaveergurjar/Hyprlock-Dots) · [catppuccin/hyprlock](https://github.com/catppuccin/hyprlock) · [BlazinLock](https://github.com/Thunder-Blaze/BlazinLock) · [hyprland-plugins](https://github.com/hyprwm/hyprland-plugins) · [hyprexpo overview](https://deepwiki.com/hyprwm/hyprland-plugins/6.1-hyprexpo) · [Hyprspace](https://github.com/KZDKM/Hyprspace) · [hypr.land/plugins](https://hypr.land/plugins/) · [awesome-hyprland](https://github.com/hyprland-community/awesome-hyprland) · [Hyprland wiki utilities](https://wiki.hypr.land/Useful-Utilities/) · [ratatui showcase](https://ratatui.rs/showcase/apps/) · [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) · [ratatui examples](https://github.com/ratatui/ratatui/tree/main/examples)
