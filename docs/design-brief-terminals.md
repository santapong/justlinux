# Design brief — the three terminal surfaces of the justlinux desktop

For: a visual redesign pass (Claude design / artifact mockups).
Scope: **Claude Studio** (a tmux tab bar + a session tree), the
**Docker panel** (a two-pane control TUI), and the **Claude Office**
(a pixel-art floor scene). Three screenshots of the current state
accompany this brief.

## The one rule that shapes everything

These surfaces live on a wallpaper-driven desktop. Colors are **roles,
not hex values** — a palette generator (wallust) refills the roles on
every wallpaper change and every surface re-inks live. A design that
hardcodes colors will look right for exactly one wallpaper.

**Deliverable format: mockups + a role map** — for every element, say
which role it wears. Sample values below are today's wallpaper only.

| Role | Meaning | Today (sample) |
|---|---|---|
| `bg` | glass surface tint (~80–93 % opacity, compositor blurs behind) | `#120F0F` |
| `fg` | primary ink — key values, labels | `#F9F0F2` |
| `accent` | **"you are here" and nothing else** — selected tab, cursor bar, active pane border | `#E17963` |
| `accent2` | titles and brands only | (often = accent) |
| `sub` | secondary ink — everything at rest, ~7:1 contrast | mixed fg/bg |
| `muted` | a **surface** tone (buttons, dialog borders) — never text | `#3D3B3A` |
| `good` | status: running / bell / success — status content ONLY | `#8EC07C` |
| `bad` | status: failure / destructive action | `#E06C75` |
| `warn` | status: transitional / attention | `#E0B25C` |

Hard constraints that are law, not taste:
- accent means "you are here"; if a design puts accent on two different
  kinds of thing, it will be rejected.
- status colors never decorate non-status content.
- `muted` is never used as text ink (it is ~1.5:1 against bg).
- text must clear 4.5:1 contrast against `bg` (3:1 absolute floor) —
  the desktop has an automated contrast gate.
- font is JetBrainsMono Nerd Font everywhere; glyph choices must exist
  in it (e.g. `●` `○` `◌` `󰚩` `󰑮` `󰉋` `󰥔` `󰡨` `󱃾` `󰋊` all do; `◐` `⌕`
  `🟢` do NOT — they render as boxes).

## Surface 1 — Claude Studio (tmux + a ratatui tree)

A kitty window running tmux: a **two-row bar on top**, tabs below it,
and tab 0 is a session tree.

Current anatomy (see `design-current-studio.png`):
- Row 0: brand `󰚩 Claude Studio` (accent2, bold) · centre = current
  directory · true tab count · `[zoom]`/copy-mode indicators · right =
  `/` (jump palette), `│` and `─` (split buttons) as fg-on-muted
  blocks, session name in sub.
- Row 1: the tabs. Selected tab = accent fill with bg-colored bold
  text. Each tab: index, name (Claude's own conversation title),
  `+beside` second-conversation tag, attention marks (`●` good = bell,
  `○` sub = unread), `✕` close.
- Tab 0 (the tree): a hint row, then `Running (n)` / `Projects (n)`
  sections; `●` running sessions, `󰑮` background jobs, `󰉋` projects,
  `󰥔` past conversations; cursor bar = accent fill, hover = faint sub
  wash; a footer row of key hints (accent2 keys, sub labels).

What can change: spacing, ordering, separators between tabs, how the
attention marks read, the hint/footer rows' wording and layout, how
the centre of row 0 is composed, the tree's iconography and section
styling, the confirm dialog's look.
What cannot: the two-row structure (one-row was tried — cramped click
targets), per-glyph mouse hit regions on ✕ and tabs (their padding IS
the click target — min 3 cells), no horizontal scrolling of the tab
row (a tmux bug desyncs clicks; overflow is reached by a jump
palette), everything must be expressible as tmux format strings +
ratatui spans (cells and 256-color/truecolor — no gradients, no
images, no box-drawing that needs alignment across rows).

## Surface 2 — the Docker panel (ratatui, two panes)

See `design-current-docker.png`. A kitty window; left pane = tree of
compose projects (`󰡨`, expandable, `N/M up`), containers (`●`/`○`/`◌`
+ live CPU/MEM or exit status), and images (`󰋊` + size); right pane =
live logs with a left border and a title; one header row (Docker /
Kubernetes pane tabs + daemon stats); one footer row of context-aware
key hints. Modals: an armed confirm (Cancel focused by default,
destructive button wears bad ONLY when focused) and a pull prompt.

What can change: the pane split ratio and border treatment, list row
composition, how projects group visually, log line presentation, modal
styling, header composition.
What cannot: the mark grammar (● good running / ○ sub stopped / ◌ warn
transitional / ✘ bad), the armed-confirm pattern for destructive
actions, cells-only rendering.

## Surface 3 — the Claude Office (pixel canvas)

See `design-current-office.png`. NOT a terminal — a layer-shell pixel
canvas on the desktop (bottom layer, behind windows): a floor-plan
office where each live Claude session is a desk with a walking agent;
a meeting room hosts multi-agent workflows; dimmed "ghost" desks offer
resumable conversations. Pixel-art sprites, glass panels for labels.

What can change: floor layout, desk/agent sprite design, how session
names/states are shown, the meeting-room treatment, ghost-desk
styling.
What cannot: sprites re-ink from the same palette roles (no
fixed-color art), desks must stay legible at ~1600×900 with up to 6
desks, click targets ≥ 40×40 px, the canvas must never eat clicks
outside its interactive pixels.

## What to deliver

1. Mockups (images are fine) for each surface — at least the studio
   bar + tree, the docker panel's main view, and one office scene.
2. A **role map**: every colored element → which role it wears.
3. Glyph list used, so it can be checked against the font.
4. Anything that needs motion described as a state pair (rest → active)
   — these are terminals and a pixel canvas, not CSS; transitions are
   instant or a few sprite frames.
