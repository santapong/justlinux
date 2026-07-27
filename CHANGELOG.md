# Changelog

All notable changes to this desktop. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is [SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Splits in Claude Studio.** Any tab now holds several panes — `C-b |`
  side by side, `C-b -` stacked, or the `[|]` `[-]` buttons at the right of
  the tab bar — so a conversation can sit next to its build log or a shell
  in the same repo. `C-b` arrows move between panes, `C-b z` zooms one.
  Splits inherit the current pane's directory. The pane title line only
  appears once a window actually has two panes, so a single-pane tab loses
  no rows. Clicking a split button while the session tree is selected opens
  a terminal *tab* instead — the sidebar is a Textual app that owns its
  whole window and must not be cut in half.
- **`t` in the session tree** — a plain terminal tab in the highlighted
  project, for the git/build/log half of the work.
- **MCP servers in Settings → Integrations.** Health of every server
  `claude` can see, colour-coded from its own check, plus add and remove for
  the local ones. Servers managed by your account at claude.ai are shown
  and marked, never offered for deletion — this panel does not own them.
  An `https://` target is added as HTTP transport, anything else as a stdio
  command. The command field has a paste button, because `Ctrl+V` belongs
  to the terminal emulator and never reaches a TUI.
- `docs/claude-studio.md`.

### Fixed

- **Live sessions listed as `~`.** The session tree and the launcher named a
  running conversation by its directory, so everything started in `$HOME`
  read as `~` and told you nothing. They now use Claude's own title, with
  the directory kept on the detail line. The rule for deciding which
  conversation a process is actually on — argv first, then time-paired
  transcripts — was worked out in the office and now lives in
  `claudesessions.py` where every session list can reach it, rather than
  being duplicated.

## [1.0.0] — 2026-07-27

First tagged release. The desktop has been in daily use throughout; this
marks the point where the widget fleet, the panels and the Claude
integration became one coherent system rather than a pile of scripts.

### The system

- **hyprcard** — 14 glass widget cards rendered by a single host
  (`hypr-cardhost`) from declarative TOML templates, replacing a fleet of
  13 conky processes. Clock, calendar, stats, netgraph, weather, now
  playing, GitHub, markets, dev repos, robotics bench, Claude usage,
  hidden-window stash, focus countdown, security.
- **TUI panels** instead of rofi-for-everything — settings, app launcher
  with real icons, window overview, wallpaper picker with live preview,
  tools hub, widget picker. All click-and-pick, all wallust-themed.
- **Grid edit mode** (`ALT+SHIFT+E`) with two snap modes: GRID lands every
  surface on a cell, FREE gives pixel placement with magnetic alignment
  guides. `g` switches. Applies to whatever you drag — cards, docks, the
  office, the visualiser.
- **Smart bar and docks** — waybar auto-hides on top-edge dwell, one
  auto-hiding app dock per monitor with its own pins.
- **Claude integration** — Studio (tabbed session workspace), the 2D
  office (a desk per live session), recall (full-text search across every
  past conversation), dropdown CLI, selection actions, region vision.
- **Ambient** — audio visualiser, sky-reactive wallpaper, a wandering pet.

### Added in this release

- **Recall** (`ALT+CTRL+H`) — full-text search across every past Claude
  conversation. ~280 MB across 800+ transcripts; ripgrep answers in under
  0.1 s, so it searches as you type. Previously only the newest 25 were
  reachable.
- **Habitica board** (`ALT+CTRL+K`, or click the focus card) — To-Dos as
  To do / Doing / Done, Dailies as their own strip. Columns come from a
  signal Habitica actually has (checklist progress), not an invented
  mapping.
- **Secrets store** — `~/.config/hyprdesk/secrets.json`, 0600, outside the
  repo. Credentials are validated against their API *before* being stored,
  so a typo fails at entry rather than silently later.
- **Pixel icons** on every card, from
  [pixelarticons](https://github.com/halfmage/pixelarticons) (MIT),
  rasterised offline so they land on whole pixels; colour still comes from
  the wallpaper.
- **Movable widget surfaces** — the office and visualiser join grid edit
  mode as a `widget` kind, repositioning live over SIGUSR1.
- **Session names everywhere** — the office and Studio tabs use Claude's
  own title for a conversation. Both previously showed the directory, so
  every session read `~`.
- **Markets card** takes stocks as well as crypto, capped at 7 with the
  overflow stated.
- **Focus history** — a weekly total and 7-day sparkline.
- **Depth setting** — per-surface layer (desktop / above windows / above
  everything) from Settings.

### Fixed

- **Unreadable status colours.** wallust maps wallpaper colours into ansi
  slots named after colours they are not: `@red` measured **1.00:1**
  against the background — invisible — while inking the power button and
  the firewall-off warning. Status ink is now pinned; module tints keep the
  wallpaper's hue with only their lightness corrected; `check-contrast.sh`
  runs after every wallpaper change and shouts if a new one regresses.
- **Focus sessions were never recorded.** `end()` deleted the state file as
  its first statement, destroying start time, label and duration at the
  moment they were known.
- **Crypto sparklines silently missing** — an N+1 API pattern was being
  rate-limited, so every crypto row rendered "no data" while stocks kept
  their charts.
- **A leaked file descriptor could hang the whole fleet.** `flock` with no
  timeout meant one daemon holding the lock wedged every later invocation;
  this killed the pet during a wallpaper change.
- **Dev repos silently skipped** anything that was not a git repo, so a
  typo'd path looked identical to a healthy repo.
- **Claude's own daemon appeared as a phantom background job** in every
  session picker, and sessions that outlived their terminal had no way to
  be stopped.
- Duplicate `claude --resume` on a live session, desks bound to the wrong
  conversation, and the office's attention state being unreachable.

### Theme

Every non-card surface audited against the fleet conventions and brought
into line: Hypr Settings, the launcher, the widget picker, the dock, the
Studio tab bar, waybar and swaync. They now take colour from
`hyprdesk.colors()` rather than parsing waybar's raw wallust slots, radii
collapse to one family (12 for surfaces, 6 for controls inside them), and
titles use a different ink from interactive state.

Two palette tiers were found to be *not tiers at all* on some wallpapers:
`accent2` measured 1.08:1 against `fg` (a title indistinguishable from body
text) and `muted` came out **byte-identical to `bg`** (an invisible border).
Both are now derived rather than taken raw, the same way `sub` always was.

Terminal panels get the palette and the ink hierarchy but never the pixel
icons — Textual draws characters, not sprites. That is a property of the
medium, not an omission.

### Performance

Fleet at idle went from **5.40% CPU to 3.24%** (40% less), RSS unchanged at
~253 MB. Two general rules came out of it: never `queue_draw()` on a fixed
timer, and never fork `pgrep` in a poll loop.

### Documentation

- `docs/architecture.md` — C4 diagrams of how the fleet fits together.
- `docs/claude-office.md` — where the office gets its facts, how a session
  is bound to a conversation, and every `office_*` key.

### Removed

- `ros2/` — 72 files of robot packages that had drifted into a dotfiles
  repo. They live in `~/ros2_ws` and RoboLLM; verified present there before
  removal.
