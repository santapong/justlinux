# Changelog

All notable changes to this desktop. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is [SemVer](https://semver.org/spec/v2.0.0.html).

## [1.1.5] — 2026-07-27

### Performance

- **Hypr Settings built all seven pages before showing you one.**
  `compose()` constructed every pane at mount — **374 widgets, ~655 ms of
  CPU and ~750 ms of wall time** before the window was usable — when six of
  those pages were off screen. Each pane body now lives in a `_build_<name>`
  generator and is mounted the first time that page is opened, through
  `ContentSwitcher.add_content()` (which mounts hidden inside a
  `batch_update`, so no frame ever shows a half-built page).

  **Startup: 750 → 398 ms wall, 655 → 347 ms CPU, 374 → 61 widgets.**

  The pane bodies were moved verbatim, and the whole widget tree — every
  type, id, class, rendered text, switch value and ordering, 373 lines of
  it — is byte-identical before and after.

  Tab switches are **not** improved: a page's cost is its layout, Textual
  charges that on first *display* rather than on mount, and the Widgets
  page is expensive because it genuinely contains ~200 widgets (17 rows of
  ~10 controls). Warm switches measure 10–70 ms in a real terminal. An idle
  prebuild of the off-screen pages was tried and dropped — it cost 60 ms of
  startup and saved nothing, for exactly that reason.

## [1.1.4] — 2026-07-27

### Performance

- **Hypr Settings spawned a Node process on every page switch.** The
  Integrations page lists MCP servers, and `claude mcp list` health-checks
  every configured server over the network: ~12 s wall, **3.1 s of CPU**,
  **282 MB** peak, and 136% of a core at its worst. It ran from
  `on_list_view_highlighted` — so it fired on *every* page, not just its
  own. Arrowing once down the seven-item sidebar left **seven of them
  running at 1.5 GB**, and `exclusive=True` did not help: it cancels the
  worker, but that worker is parked in `to_thread` around
  `subprocess.run` and the process it already started runs to completion.

  Each page now refreshes only what it shows. The server list is fetched
  on entering Integrations, reused for 5 minutes, and forced by the
  Re-check button; a flag stops runs overlapping at all. Walking the whole
  sidebar now spawns **nothing** unless you open Integrations, and then
  exactly one.

  Measured on a single clean instance afterwards: **0.00–0.38% CPU on
  every page, 55 MB RSS**. Three other mount-time refreshes were checked
  and left alone — `firewall_on`, `service_active`, `current_wallpapers`
  and `autohide_on` are all under 0.1 ms.

## [1.1.3] — 2026-07-27

### Fixed

- **The board's 2-minute refresh stole your selection.** Every card is
  rebuilt on a repaint, so the focused one stopped existing and focus fell
  back to the first card in the tab. If you were about to press Enter or
  `d`, it would have landed on the wrong task. The refresh now remembers
  which *task* was selected, not which widget, and puts focus back on it.
- **`claude --verbose doctor` still counted as a session.** The
  subcommand check tried to guess which flags take a value, so a boolean
  flag swallowed the word after it. It no longer guesses: the result is
  only ever membership-tested, so reading a flag's value by mistake is
  harmless, while missing a real subcommand paints a phantom desk.
- **Drop targets were only correct by accident.** `zone_at` filtered on
  `display`, which is `True` for a widget in a hidden tab — the guard
  never fired. It worked because Textual gives hidden panes a zero-area
  region. Matching the wrong zone would have moved a card in a tab you
  cannot see, which is not a thing to leave resting on an implementation
  detail; it now checks the pane explicitly.
- **"end of this sprint"** in the due-date picker meant whichever sprint
  *today* falls in, even while you were looking at a different one.

## [1.1.2] — 2026-07-27

### Fixed

- **Opening Hypr Settings appeared to start a Claude session.** It did
  not — but the office painted a desk for it, and every session picker
  listed it as a background job, for the ~20 s the Integrations page spends
  health-checking MCP servers. `claude` is one binary for a conversation
  and for a pile of one-shot subcommands (`mcp`, `update`, `doctor`,
  `plugin`, `auth`), and the fleet counted any process by that name. A
  session is now `claude` with no subcommand. The same rule covers
  `claude update` and friends, which had the same effect and had simply
  never been noticed.

## [1.1.1] — 2026-07-27

### Documentation

- **Every diagram is now generated.** `docs/diagrams/gen_c4.py` emits all
  seven SVGs from short declarations; the `.svg` files are output and
  should not be hand-edited. They were hand-authored once and the cost of
  editing SVG by hand was high enough that they stopped being updated — a
  box is four numbers and three strings now.
- **Three new C4 component diagrams**: Claude Studio, the 2D office, and
  the Habitica board. The office's Mermaid flowchart and the Studio's
  ASCII sketch were replaced by real C4 diagrams; nothing in `docs/` uses
  Mermaid any more.
- **Context and container diagrams brought up to date** — they predated
  the Studio, the office, the board, the secrets store and MCP. The
  container diagram was also re-laid-out: connectors used to run straight
  through four boxes each.
- `docs/habitica-board.md` — the tag trick behind Doing and sprints, the
  ~30 requests-a-minute budget that shapes the board, and the two faults
  that made the old one look broken.
- README opens with the context diagram and indexes all four docs.

### Removed

- `ros2/` leftovers on disk — one gitignored `.pyc` whose source had
  already gone. The package itself is untouched in `~/ros2_ws`.

## [1.1.0] — 2026-07-27

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
- **`s` — open a conversation beside the one you are reading.** The point
  of splits here: a bare split gives a shell, `s` runs `claude --resume` in
  the new pane, so two conversations sit side by side. Already open, and it
  focuses that tab instead of resuming a second copy.
- **`m` — the settings panel as a Studio tab**, so MCP servers are
  reachable without leaving the studio. A tab is just a tmux window and
  Hypr Settings is a terminal app, so this is the same panel rather than a
  second settings framework built inside the studio.
- A tab holding several panes shows `·N` in the bar.
- **A sprint board and a schedule.** Three more tabs: **Sprint** puts one
  ISO week's work behind a Backlog column — dragging a card out of Backlog
  joins the sprint, dropping it back leaves — with `< >` to move between
  sprints. **Week** is seven day columns; drop a card on a day and that
  becomes its due date. **Today** answers what to do now: overdue, due
  today, in progress, and today's dailies. `d` on a card sets a date
  without leaving the board. A sprint is a `sprint-2026-W31` tag, so it is
  real state Habitica keeps and the phone can see, and the tag is created
  only when you first move something into that sprint.
- **The Habitica board, rebuilt.** Three tabs — To-Dos, Dailies, Habits —
  because Habitica has three kinds of task and squeezing them into one
  board misrepresents all three. Tasks are cards now: difficulty, checklist
  progress, due date, streak, and a Habit's ＋/− counters, with its
  `:shortcode:` emoji resolved. **Drag a card between columns**; a terminal
  has no ghost to float under the cursor, so the card you are carrying dims
  and the column that will take it lights up. `[` and `]` do the same from
  the keyboard, `1 2 3` switch tabs.
- `docs/claude-studio.md`.

### Fixed

- **Nothing could be ticked off after switching tabs.** `1 2 3` changed the
  visible tab but left focus behind on a card that was no longer on screen,
  so Enter did nothing — and said nothing about why. Switching tabs now
  hands you the first card in the new one, and Enter with nothing selected
  says so instead of silently doing nothing. A **double-click** also ticks a
  card, so the mouse can finish a task and not just move one.
- **The board never refreshed by itself.** Work ticked off on the phone
  never appeared. It now refreshes every 2 minutes — one refresh costs five
  of the ~30 requests a minute Habitica allows, so that is 2.5/min and
  leaves the rest for what you do. The status line says when it last ran and
  when it will run again; `r` is still immediate; a refresh never lands
  mid-drag.
- **The Habitica board could never show Done, and Doing was a fiction.**
  Two separate faults. `GET /tasks/user?type=todos` returns only the
  *unfinished* to-dos, so ticking one made it vanish from the board rather
  than move — `completedTodos` is now fetched too, and Done holds 30 real
  cards. And "Doing" was inferred from checklist progress, a signal almost
  no to-do carries; it is a `doing` tag now, which is real state Habitica
  will keep, show on the phone, and let you set from the website.
- **The board could rate-limit itself.** Habitica allows roughly 30
  requests a minute and a refresh spends five, yet every drag and every tick
  triggered a full refresh — three actions in a row hit the wall. Actions
  now update in place and only reconcile with the server when one fails.
- **The session tree collapsed under you every 6 seconds.** Its refresh
  rebuilt the tree unconditionally, so an expanded project snapped shut and
  the cursor jumped to the top mid-scroll. Almost every poll finds nothing
  new, so almost every poll now does nothing; when something has changed,
  the expanded projects and the cursor are restored.
- **A tab took the name of the wrong conversation** once it held two. The
  rename walked every pane, so whichever it reached last won; a tab's
  identity is its first pane.
- **Names could render blank in the launcher.** The detail column was
  `width: auto` against a `1fr` name, so a long detail starved the name to
  nothing — two rows had no name at all. The name keeps its share now.
- **Live sessions listed as `~`.** The session tree and the launcher named a
  running conversation by its directory, so everything started in `$HOME`
  read as `~` and told you nothing. They now use Claude's own title, with
  the directory kept on the detail line. The rule for deciding which
  conversation a process is actually on — argv first, then time-paired
  transcripts — was worked out in the office and now lives in
  `claudesessions.py` where every session list can reach it, rather than
  being duplicated. Resumable rows carry their title too, so the launcher's
  flat list no longer shows twenty rows reading `~`.

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
