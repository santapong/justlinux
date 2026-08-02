# Rust migration — plan of record

> **POLICY (1 Aug 2026, user decision): new features in this project are
> written in Rust.** Claude Office and Claude Studio migrate from Python
> to Rust. The "Textual apps stay Python forever" rule below is REVISED
> for the studio (see "The ladder, revised"); it still holds for
> settings/launcher/kanban until they are next rebuilt, not merely
> touched. The rationale sections below are kept as written — the
> measurements are still true; the *decision* on top of them changed.

Decided 30 Jul 2026, from measurements on this machine, not from taste.
The question was "what would a Rust rewrite buy, and can it happen feature
by feature?" — the answers are *some real things* and *yes, unusually
cleanly*, and the conclusion is a **permanently hybrid fleet**: migrate
what is stable and resident, keep what is churning and spawned in Python.

## What exists (measured 30 Jul 2026)

~16,000 lines of Python: 12.5k across `bin/`, 3.6k in `lib/hyprdesk`.
Two kinds of program with opposite economics:

| Kind | Programs | The cost that matters |
|---|---|---|
| Resident GTK/cairo layer surfaces | office, pet, dock, cardhost (+viz when on) | **memory** — 55–65 MB each, ~240 MB together, all-day uptime |
| Spawn-on-click Textual TUIs | settings, launcher, studio sidebar, kanban | **startup** — 229 ms to `import textual`, 169 ms for GTK, before any of our code runs |

Whole-fleet Python footprint at measurement time: **447 MB across 8
processes**. CPU is a non-issue — the busiest widget (office, animating)
holds 2.9% of one core; Chromium eats 40× more.

## What Rust buys, honestly

- **Startup ~5–15 ms** instead of ~250–400 ms. Felt on every keybind spawn.
- **Memory ~5–15 MB per widget** instead of 55–65. The fleet drops from
  ~450 MB to well under 100. Real, though this box has 14 GB free.
- **One static binary per tool.** No `pip --break-system-packages`, no
  Python version drift, and the entire class of bug where install.sh
  forgot `lib/` (v1.2.0's fix) cannot exist.
- **Compile-time surface** for the typo/attribute class of runtime error.

**What Rust does not buy — from this repo's own bug ledger.** The defects
that actually cost days here were *contract* bugs against external tools:
`kill-window -t` not format-expanding, `set-option -t` meaning
target-*pane*, NetworkManager aging its scan cache, glyphs missing from
JetBrainsMono, Rich markup injection via conversation titles. They live in
strings and other programs' semantics; **Rust catches none of them**, and
every scar encoded in the Python comments would need re-earning in a
rewrite.

**What Rust costs: velocity.** This desktop's defining property is
same-day conversational change. Textual has no Rust peer at that
productivity (ratatui is far lower-level), so a wholesale rewrite trades
the repo's fastest-moving asset for efficiency it mostly does not need.

## Why partial migration works here

Every surface is its **own process**; nothing shares in-process state.
The boundaries are already files, sockets, and CLIs — a Rust binary that
honours them is a drop-in citizen. No FFI, no big bang.

The contracts (this list IS the migration spec — a port is done when it
honours every row it touches):

| Contract | Grammar / semantics |
|---|---|
| `~/.config/conky/widgets.conf` | flat `key = value`; keys `\w+`, values `\S+`; **writes only via the confwrite discipline** (flock + tmp + rename, one-deep `.undo`) |
| Palette | parse `colors.lua` (or `themes/<t>.lua` when `theme=` says so) for `name = "#RRGGBB"`; derive `sub = mix(fg,bg,0.62)`; **pin** good/bad/warn `#8EC07C/#E06C75/#E0B25C`; contrast-fix accent2 (<1.6:1 vs fg → accent) and muted (<1.35:1 vs bg → mix 0.22) |
| Layer shell | namespace `hypr-<name>` (blur layerrules match it), layer bottom, exclusive zone 0, `<name>_pos/_x/_y/_mon` conf keys, POS_ANCHORS edge map, display-only surfaces set an **empty input region** |
| Signals | SIGUSR1 = reposition (re-read conf), SIGUSR2 = theme reload, both **must** have handlers (default disposition is terminate); SIGTERM/INT clean exit |
| Toggle convention | second invocation kills the first (`pgrep -xf` on the exact cmdline) and persists `<name>=off`; hotkey start never persists `on` |
| Hyprland | `hyprctl -j` JSON, event socket2 line protocol |
| Claude data | `~/.claude/projects/*/<sid>.jsonl`, `~/.claude/jobs/<sid8>/state.json` (shape drifts — every field access best-effort) |

Shared code strategy: `rust/hyprdesk/` grows the same helpers as
`lib/hyprdesk` **by parsing the same files** — the files are the
interface, so the two implementations cannot drift apart in any way that
matters without a visible symptom.

## Toolchain decision

**Pure Wayland, no GTK.** rustc 1.97 is installed (wallust brought
cargo); GTK *dev* headers are not, and they are not needed:
`smithay-client-toolkit` (wlr-layer-shell) with the pure-Rust wayland
backend plus software rendering into SHM buffers has zero C build
dependencies and produces the same ARGB surface the blur layerrules
expect. Drawing that is currently cairo (rounded rects, bars, pixel art)
ports to `tiny-skia` or direct pixel writes.

## The ladder

| Order | Component | Why | Expected gain |
|---|---|---|---|
| 1 — pilot | `hypr-viz` (224 lines) | hot loop, near-zero UI logic, off by default, fully reversible | proves the toolchain; the pilot's numbers decide the rest |
| 2 | `hypr-pet`, `hypr-cardhost` | resident, animated, stable feature set | ~115 MB, lowest rewrite risk |
| 3 | `hypr-appdock` | resident, moderate logic | ~64 MB |
| 4 | `hypr-claude-office` | resident but logic-dense; port only once its feature set stops moving | last of the GTK fleet |
| — | settings, studio, launcher, kanban | Textual, highest churn, spawn-on-demand | **stay Python.** Startup lag is their only sin and it is already mitigated (lazy panes: 750→398 ms) |

Each rung ships as `bin/<name>` replaced by the Rust binary **behind the
same conf toggle and the same argv conventions**, with the Python version
kept in git history for rollback. A rung is accepted when: same visual
output (screenshot diff), same conf/signal behaviour (the contract table),
and measured RSS/CPU/startup improvements recorded here.

## Salvage — the archived all-at-once attempt

A previous migration exists on `claude/migrate-project-rust-czto8s`
(8.5k lines; `optimize/migrate-to-rust` was its twin, deleted 30 Jul).
It built ONE `justlinux` multi-call binary swallowing settings, launcher,
tools and wallpaper as ratatui apps — the exact shape this plan rejects:
one release gate for everything, no way to stop halfway, and a rewrite of
the Textual apps this plan keeps.

Worth taking from it (`git show claude/migrate-project-rust-czto8s:rust/src/<f>`):

- `hypr.rs` — hyprctl IPC + event socket client
- `colors.rs` — palette parsing (check it against the contract table's
  pin/contrast rules before trusting it)
- `proc.rs`, `util.rs` — process + misc helpers
- `tests/integration.rs` — 697 lines of harness ideas

Do NOT merge the branch; cherry-pick files into per-tool crates as rungs
need them. The branch stays as an archive.

## Branch and release mechanics

Work happens on `migrate/rust` (branched from develop 30 Jul 2026),
merged to develop per RUNG, not at the end — the hybrid fleet is the
steady state, not a transition. Each rung rides the normal develop → main
release flow as its own minor version, and rolls back by restoring the
Python file from git history. `rust/` is a cargo workspace; built
binaries land in `bin/` under the same names, so `install.sh` and every
caller stay oblivious.

## The ladder, revised (1 Aug 2026 — Rust-for-new-features policy)

| Order | Component | Language plan |
|---|---|---|
| 1 — pilot | `hypr-viz` | **done, measured** (3.6 MB vs 55); pending audio + click-through acceptance |
| 2 | **`hypr-office2d`** — the Gather-style office | **NEW FEATURE, BORN IN RUST.** It is a rewrite of hypr-claude-office anyway; writing it in Python first and porting later would build it twice. Pulls the office's rung forward. |
| 3 | `hypr-pet`, `hypr-cardhost`, `hypr-appdock` | Rust, as before |
| 4 | **studio sidebar** → ratatui | REVISED from "stay Python". Costs named below. |
| hold | settings, launcher, kanban | Python until rebuilt — converting a working 2k-line Textual app with no feature driver is cost without benefit |

**Step 0 for rung 2: extract `rust/hyprdesk` as a workspace library crate**
from hypr-viz's `hyprdesk.rs` (conf grammar, palette + pins + contrast
fixes, confwrite discipline), and grow it what the office needs: layer
POS_ANCHORS placement, `hyprctl -j` clients, the claudesessions readers
(session rows, titles, `job_state`, `active_subagents`). One crate, every
rung reuses it, the files stay the interface.

## Rung 2 — the 2D office (design in docs/claude-office.md + concept render)

Gather-style floor: desks at x,y that grow with sessions, agents that
WALK — in from the door on session start, out on exit — and a MEETING
ROOM that fills with mini-Clawds when `active_subagents()` sees a
workflow. The architectural core is a reconcile/animate split: the 2 s
poll sets *desired* state, a ~160 ms tick moves actors toward it.
Manhattan paths only (corridor lane, then turn); no pathfinding.

Phases: P1 scene engine (floor plan, actors, waypoints, the split) →
P2 arrivals/departures + stable sid→desk map + ghost desks →
P3 meeting room (workflow name, minis, +N overflow) →
P4 parity (click/hover/tips, empty state, size presets, docs).

**Status 1 Aug 2026: P1–P4 SHIPPED and live.** office_layout=floor is
active on this machine; desktop-widgets.sh and ALT+CTRL+O route on the
key with an -x fallback to grid, install.sh builds the binary when cargo
exists. Measured: RSS 7.9 vs python 56 MB; cpu 2.9% while a session
types (python parity), near-idle otherwise via the dirty-flag pass —
animate() reports whether the frame visibly changed and a still scene is
not redrawn. Clicks user-verified. Two lessons that must not be
re-learned: fontdue eagerly outlines all 12k Nerd Font glyphs (49 MB —
use ab_glyph), and every transcript reader is byte-capped because a
48 MB transcript exists. The python office stays in bin/ as the grid
layout, no longer on the removal path — grid IS a layout now.

## Rung 3b — cardhost, re-scoped from evidence (1 Aug 2026)

Reading it corrects the plan: the ladder called cardhost "lowest rewrite
risk" and it is the OPPOSITE — the riskiest resident port. One process,
N layer surfaces (one per card), a ctl unix socket that hypr-arrange,
Settings and tools all speak (`--ctl ping|reload|reload-theme`), TOML
templates (cardspec), THIRTEEN row renderers (title/text/keyval/bar/
sparkline/graph/hr/clock/calgrid/notesfile/heatmap + repeat groups),
async script sources with generation tokens, staleness badges and
error-card degradation, monitor hotplug + reserved-inset rechecks.
~1,600 python lines across four files against the pet's 508.

**The keystone is DONE and proven: `hyprdesk::grid`** — hypr-arrange
(python, staying) writes the `<id>_col/_row` cells a rust cardhost
reads, so the two grids must agree to the pixel. They do:
`examples/grid_parity.rs` sweeps 1,584 cases across six workarea
geometries — identical, including exact-half roundings where python's
banker's rounding and rust's round-half-away disagree (`pyround` carries
the python semantics).

Port order for the rest, one commit each: cardspec (TOML → the `toml`
crate, template + instance params + legacy migration) → rows (the 13
renderers on tiny-skia + draw::Text, measure() first since placement
depends on card height) → the multi-surface host (N LayerSurfaces on one
event loop — new sctk ground, prove with two static cards before
wiring data) → sources (clock/stats/netgraph builtin, async cmd with
generation tokens via calloop) → ctl socket (calloop UnixListener,
same exit-if-owned probe) → hotplug/reserve rechecks. The ctl protocol
and the conf keys are the interfaces; both ends already have tests in
the parity example pattern.

**COMPLETE — swapped 2 Aug 2026.** Every step above landed with its
parity harness green: cardspec (14 templates, all derived values
identical), rows (`measure()` exact on every template), the
multi-surface host (14 cards, 14 pixel-exact positions vs the live
python geometry), then sources + ctl + hotplug in the final pass. Live
acceptance after the swap: 14/14 cards identical in position AND size
once real fetch data landed, `--ctl ping/reload/reload-theme` all
answer, hypr-arrange's `tool_ctl()` reload works unchanged, error
cards and "(stale)" badges verified. Cost: 0.58% CPU / 12.3 MB RSS vs
python's ~59 MB. `~/.local/bin/hypr-cardhost` is now the rust binary;
desktop-widgets.sh guards both cmdline forms and install.sh builds
`-p hypr-cardhost`. Remaining resident python: appdock (rung 3c),
then studio (rung 4, LAST).

## Rung 3c — appdock, hybrid by design (2 Aug 2026)

**COMPLETE — swapped 2 Aug 2026.** rust/hypr-appdock ports the resident
half: per-monitor docks (PNG icons via tiny-skia's native decoder,
2-letter fallback), 4px reveal strips with the 180 ms dwell and 600 ms
hide grace, the smart waybar (SIGUSR1 toggle + 400 ms cursor poll), the
Hyprland event socket with EOF→poll fallback, pins.json v2 under the
shared flock, and the full ctl vocabulary — ping|reload|reload-theme|
bar-pin|show-all|resume — with the exit-if-owned probe. Auto-hide
destroys/recreates the LayerSurface instead of GTK map/unmap.

The PICKER stays python on purpose: a GTK search dialog is exactly the
app class the plan keeps in python. It ships as `hypr-appdock-picker`
(the old python file, unchanged); the rust ＋ button — and the rust
binary's own `--picker` flag, kept for old callers — exec it, and its
pins.json edits propagate back through the 2 s mtime watch.

Live acceptance: 3 docks + 6 strips on the exact python namespaces
(hypr-appdock-<san>, hypr-dockedge-{bottom,top}-<san>), show-all maps
3 → resume unmaps to 0 (caught live: resume needed a refresh_soon — the
python called apply_visibility inside set_suspend), bar-pin toggles
"ok pinned"/"ok released", unknown verbs → err, hypr-arrange interop
via the installed path. Cost: 0.10% CPU / 14.1 MB RSS. Not ported:
button tooltips (layer surfaces have none — documented degradation).

## Rung 4 — studio sidebar in ratatui, costs named

The studio is mostly tmux orchestration (36 `tmux(...)` call sites —
trivial subprocess work in any language). The Rust cost is the Textual
UI: ratatui has no Tree, no ModalScreen, no CSS, no `run_test()`. The
port hand-rolls a tree list (~200 lines), a confirm dialog, and the
aim-then-open click contract, and replaces the headless-pilot test
harness with state-level tests (drive the app struct, assert the frame
buffer). Everything the current sidebar learned — escape() every title,
the q-confirm semantics, first-click-aims — is spec, listed in
docs/claude-studio.md. Do this LAST: it is the only rung where Rust
makes the code harder rather than smaller.

## Pilot acceptance criteria — hypr-viz

- [ ] identical placement (`viz_pos/_x/_y/_mon`), namespace `hypr-viz`
      (blur applies), empty input region (never eats a click)
- [ ] same audio path (`pw-record` monitor capture) incl. the
      reconnect-on-EOF loop — PipeWire suspends idle sinks
- [ ] same 12-band Goertzel response, decay 0.72, same bar geometry
- [ ] SIGUSR1 reposition / SIGUSR2 retheme / toggle-kill persists off
- [ ] measured: RSS, CPU while playing, binary size, cold start
