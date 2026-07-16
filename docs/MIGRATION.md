# Rust migration — acceptance criteria, benchmarks, verification

This document was written in two stages, in this order:

1. **Acceptance criteria & benchmark methodology** — committed *before* any
   benchmark was run, so the results could not shape the bar.
2. **Results, verification evidence & bias check** — appended afterwards.

## What was migrated

Everything in `bin/` (2,538 lines of bash + python) became **one Rust
binary** (`justlinux`, in `rust/`) installed as symlinks that keep the old
names, so every keybind in `hyprland.conf`, every waybar hook, and the
`pgrep -f waybar-autohide.sh` contract keep working unchanged.

| Old (bash/python) | New | Notes |
|---|---|---|
| `av-status.sh` (bash) | `justlinux av-status` | same JSON output, byte-identical |
| `fw-status.sh` (bash) | `justlinux fw-status` | ufw.conf read is native, no `grep` fork |
| `bar-toggle.sh` (bash) | `justlinux bar-toggle` | `/proc` scan + `kill()` — zero subprocesses (was `pgrep` + `pkill`) |
| `screenshot.sh` (bash) | `justlinux screenshot` | `hyprctl … -j \| python3 -c` JSON hop is now a native IPC call |
| `wallpaper.sh` (bash) | `justlinux wallpaper` | hyprpaper.conf parse/rewrite native; still drives `wallust`, `hyprctl hyprpaper` |
| `waybar-autohide.sh` (python daemon) | `justlinux waybar-autohide` | same 200 ms cursor poll over the IPC socket, same SIGUSR1 pin semantics |
| `hypr-tools.sh` (bash, 562 ln) | `justlinux hypr-tools` | all 25 dispatcher arms; the embedded `python3 - <<PY` JSON blocks (stash / hide / unhide / reorder) are native; rofi menus byte-identical |
| `hypr-settings` (python/Textual, 666 ln) | `justlinux hypr-settings` | ratatui port: sidebar+cards, steppers with gauges, live `hyprctl keyword`, Save block-rewriter, confirm dialogs, page-file focus protocol |
| `hypr-launcher` (python/Textual, 1029 ln) | `justlinux hypr-launcher` | ratatui port: icon grid via native kitty-graphics-protocol module, categories/favorites/recent, window switcher, wallpaper live preview, tools hub |
| `install.sh` | updated | builds `rust/` with cargo (already required for wallust), installs symlinks |

Where Rust genuinely changes the execution model:

- **Hyprland IPC is native.** The old tools forked `hyprctl` (and `python3`
  to parse its JSON) for every query/dispatch. The Rust tools open
  `$XDG_RUNTIME_DIR/hypr/$SIG/.socket.sock` directly. `hyprctl` is still
  spawned for `hyprctl hyprpaper …` only (hyprpaper's own socket protocol is
  not ours to re-implement).
- **`pgrep`/`pkill` are native** `/proc` scans + `kill(2)`.
- **The TUIs need no interpreter or import step.** The old panels paid
  `python3` start + `import textual` on every ALT+X / ALT+R press.
- External *tools* are still spawned where they are the feature: rofi,
  kitty, grim/slurp, wl-copy, notify-send, wallust, systemd-run, pkexec,
  timedatectl, magick. Replacing those with libraries was out of scope and
  would change behavior.

Deliberate small deviations (all noted in code):

- Launcher filter input is append/backspace only (no mid-line cursor
  editing); the python one debounced re-filtering by 150 ms — the Rust one
  re-filters instantly (native filtering is sub-millisecond).
- Icon/thumbnail cache keys use a different hash (std hasher instead of
  md5), so thumbnails regenerate once after migration; the
  `resolve.json`/`apps.json`/`state.json` formats are unchanged, so
  favorites, recents and old cache entries survive.
- Grid scrolling is by tile row, not pixel row.

## Acceptance criteria

Written before benchmarking. Every criterion gets a PASS/FAIL verdict with
evidence in the Verification section below.

### Functional parity (F)

| ID | Criterion | How it is verified |
|---|---|---|
| F1 | `av-status`/`fw-status` print byte-identical JSON for on/off states | integration tests `av_status_*`, `fw_status_*` |
| F2 | `screenshot region\|screen\|all`: same grim/slurp/wl-copy pipeline, monitor from live workspace, `~`-abbreviated notification, Esc-cancel is silent success | integration tests `screenshot_*` |
| F3 | `wallpaper`: hyprpaper.conf written in the same block format (byte-exact vs bash `printf`), per-monitor update keeps other assignments, `all` clears overrides, missing file → same error + exit 1, recolor pipeline order (wallust → reload → SIGUSR2 waybar → swaync) | unit tests `wallpaper::*`, integration tests `wallpaper_*` |
| F4 | stash: reading-order save (`(y,x)` sort), ordered restore from state file, special-workspace guard, same notification texts | integration tests `stash_*` |
| F5 | hide/unhide window: state file schema `{addr: {ws,title,cls}}`, restore-to-saved-ws follow vs restore-all silent, same rofi list format | integration tests `hide_window_*`, `unhide_*` |
| F6 | reorder: waybar config rewrite preserves key order + unrelated keys, 4-space indent, python's exact move semantics incl. move-before-itself and END-of-side rows | unit tests `reorder_*`, integration test `reorder_moves_module_and_restarts_bar` |
| F7 | keybind sheet: same grep/sed transforms (first-occurrence-only sed quirks preserved), editor opened at the exact line | unit test `keybind_display_transforms_like_sed`, integration test `keys_menu_*` |
| F8 | autohide daemon: hides on start, reveals at y≤1, hides at y>34, SIGUSR1 = show+pin / hide+resume, SIGTERM leaves bar visible, waybar pid cached & re-resolved | integration test `autohide_daemon_hides_shows_and_pins` |
| F9 | bar-toggle: delegates to daemon via SIGUSR1 when running, else SIGUSR1 to waybar | integration test `bar_toggle_*` |
| F10 | settings panel: stepper clamping incl. stretched limits, per-side gaps read-only, dirty-tracking Save (only touched keys, missing-line warning), Revert re-reads live values, power confirms, page-file navigation protocol | unit tests in `settings::tests` (state machine + TestBackend draw) |
| F11 | settings Save rewrites `hyprland.conf` by block path, preserving comments/indent/unrelated keys — file-corruption safety | unit tests `save_*` (5 cases) |
| F12 | launcher: python rank tiers (prefix/word-prefix/substring), category mapping first-hit-wins, favorites/recent grouping, recent trimmed to 20, .desktop parsing (first-key-wins, NoDisplay/Hidden, Terminal→kitty -e, %-code stripping), wallpaper monitor tiles with position subs, preview + Esc-restore | unit tests in `launcher::tests` (13 cases) |
| F13 | every `hypr-tools.sh` dispatcher arm exists with the same name and target | code-review table + `tools::run` match |
| F14 | state/cache file compatibility: `state.json` (favorites/recent), `resolve.json`, `hypr-stash-*.json`, `hypr-hidden.json`, `hypr-settings.page` keep their formats/paths | unit + integration tests above read/write real files |
| F15 | installed names stay `*.sh` / `hypr-*` via symlinks; `pgrep -f waybar-autohide.sh` still matches the daemon | install.sh + integration test spawns the daemon through the symlink |

### Performance (P) — thresholds set before measuring

Benchmarks run old vs new **against identical stubs/fake IPC socket** in
the same container; medians over ≥20 runs after warm-up. Thresholds are
deliberately far below the raw expectation to leave room for noise.

| ID | Criterion | Threshold |
|---|---|---|
| P1 | `hypr-settings` time-to-first-output in a pty (DRYRUN), cold process each run | median ≤ 100 ms **and** ≥ 5× faster than python/Textual |
| P2 | `hypr-launcher menu` time-to-first-output in a pty (DRYRUN) | median ≤ 100 ms **and** ≥ 5× faster than python/Textual |
| P3 | `hypr-tools stash` end-to-end with 20 windows (fake socket; old script gets an instant-reply `hyprctl` stub, which *favors* the old side) | median ≥ 3× faster |
| P4 | autohide daemon resident memory (VmRSS after 5 s against fake socket) | ≤ 5 MB **and** ≥ 5× smaller than python daemon |
| P5 | `av-status` end-to-end (same `systemctl` stub both sides) | median ≥ 2× faster |
| P6 | `screenshot screen` end-to-end (same grim/wl-copy/notify stubs; old side gets instant `hyprctl` + real `python3` for its JSON hop, as in the original script) | median ≥ 2× faster |
| P7 | process count: stash with 6 windows spawns 0 external processes for queries/dispatches (new) vs ≥ 9 (old: python3 + N hyprctl + …), measured from stub logs | evidence recorded |
| P8 | release binary ≤ 5 MB stripped (sanity: this replaces ~50 MB of python+Textual site-packages, but must not itself bloat) | recorded |

### Quality (Q)

| ID | Criterion |
|---|---|
| Q1 | `cargo test` — all unit + integration tests pass, 3 consecutive runs |
| Q2 | `cargo clippy --all-targets` — no warnings |
| Q3 | `cargo build --release` succeeds from a clean checkout with only Cargo.toml-declared deps |
| Q4 | benchmark scripts + raw results committed so numbers are reproducible on real hardware |
| Q5 | README + install.sh updated; old implementations preserved under `legacy/` for rollback and comparison |

## Benchmark methodology (fixed before running)

- Container: shared cloud CI container (CPU count and kernel recorded in
  results); **no real Hyprland/Wayland** — compositor interactions are
  served by a fake IPC socket answering canned JSON, identical for both
  sides. This measures *tool overhead*, not compositor behavior — see the
  bias check.
- Each timed sample is a **cold process start** (`fork`+`exec` of the tool
  under test), because that is what a keybind press costs on the desktop.
  Warm filesystem cache (3 discarded warm-up runs) — python .pyc compiled
  once before sampling, which *favors* python vs a true first launch.
- ≥ 20 samples per case; report median and p95. Timing via
  `clock_gettime(MONOTONIC)` wrappers in the harness script.
- The python TUIs run with the same `HYPRSETTINGS_DRYRUN=1` they shipped
  with; Rust TUIs run with `HYPR_BENCH_STARTUP=1` (draw one frame, exit) and
  the pty harness measures **time to first output byte** for both — the
  moment the app first touches the terminal.
- Old scripts are the untouched originals from `legacy/bin/`, run with the
  same `$HOME`, `$XDG_RUNTIME_DIR`, stub `$PATH` and fake socket as the new
  binary.

*Everything below this line was written after the benchmarks ran.*

<!-- RESULTS -->
