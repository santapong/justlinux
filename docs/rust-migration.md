# Rust migration — plan of record

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

## Pilot acceptance criteria — hypr-viz

- [ ] identical placement (`viz_pos/_x/_y/_mon`), namespace `hypr-viz`
      (blur applies), empty input region (never eats a click)
- [ ] same audio path (`pw-record` monitor capture) incl. the
      reconnect-on-EOF loop — PipeWire suspends idle sinks
- [ ] same 12-band Goertzel response, decay 0.72, same bar geometry
- [ ] SIGUSR1 reposition / SIGUSR2 retheme / toggle-kill persists off
- [ ] measured: RSS, CPU while playing, binary size, cold start
