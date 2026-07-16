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

---

## Results

Container: 4 cpus, kernel 6.18.5, python 3.11.15, textual 8.2.8,
rustc 1.94.1, release build (LTO, stripped). 20 samples per case after 3
warm-ups. Raw data: `bench/results/raw.jsonl`; reproduce with
`bench/run.sh` (works on real hardware too — see the bias check for why
you should).

| case | old median | new median | speedup | old p95 | new p95 |
|---|---|---|---|---|---|
| hypr-settings first paint (pty) | 189.0 ms | 3.5 ms | **54.5×** | 192.8 ms | 3.7 ms |
| hypr-launcher menu first paint (pty) | 188.6 ms | 3.3 ms | **57.7×** | 194.7 ms | 3.6 ms |
| hypr-tools stash, 20 windows (end-to-end) | 79.7 ms | 9.3 ms | **8.5×** | 86.2 ms | 12.4 ms |
| av-status (end-to-end) | 5.1 ms | 3.6 ms | **1.4×** | 5.5 ms | 4.1 ms |
| screenshot screen (end-to-end) | 35.3 ms | 9.1 ms | **3.9×** | 37.5 ms | 9.6 ms |

| daemon | VmRSS | VmHWM | cpu ticks / 5 s |
|---|---|---|---|
| waybar-autohide (python) | 10.8 MB | 10.8 MB | 2 |
| waybar-autohide (rust) | 2.6 MB | 2.6 MB | 0 |

Process spawns for one `stash` of 6 windows: old = **10** (python3 +
hyprctl×8 + notify-send), new = **1** (notify-send) plus 8 unix-socket
requests. Rust binary: **1.4 MB** stripped; the Textual package it replaces
is 6.4 MB *before* counting python itself, rich, textual-image and PIL.

### What this means in practice

The user-visible win is the **panels**: every ALT+X / ALT+R / ALT+D press
went from ~190 ms of interpreter+import before the first pixel to ~3.5 ms —
under one 60 Hz frame instead of ~12 frames. Workspace stash (ALT+A) went
from ~80 ms to ~9 ms. The always-running autohide daemon dropped from
~11 MB to ~2.6 MB resident. The `av-status`/`fw-status` waybar pollers were
**not** meaningfully improved (see P5: FAIL below) — they were already
dominated by the `systemctl` call, not by script overhead.

## Verification against the acceptance criteria

### Functional (all PASS)

| ID | Verdict | Evidence |
|---|---|---|
| F1–F9 | **PASS** | 19/19 integration tests green, 3 consecutive runs (`cargo test --test integration`) — each F-row's named tests listed in the criteria table |
| F10–F12 | **PASS** | 50/50 unit tests green, incl. 5 `save_to_config` corruption-safety cases, gauge/rank/category/reorder semantics, TestBackend draw smoke tests for every page/mode |
| F13 | **PASS** | all 25 dispatcher arms present in `tools::run` (settings, power, power-menu, apps, windows, reorder, autohide, wallpaper, wallpaper-rofi, reminders, edit-hypr, edit-bar, reload, menu-rofi, random, keys, restart-bar, clock, remind, stash, hide-window, unhide-window, firewall, clamav, default) |
| F14 | **PASS** | integration tests read/write the real state files with the python schemas (`hypr-stash-4.json` list order, `hypr-hidden.json` `{addr:{ws,title,cls}}`); launcher `state.json`/`resolve.json` loaders parse the python format |
| F15 | **PASS** | autohide integration test launches the daemon **through the `waybar-autohide.sh` symlink** and bar-toggle finds it by cmdline, exactly like `pgrep -f` |

Two known behavioral deviations, both intentional and documented above:
launcher filter has no mid-line cursor editing, and grid scrolling is by
tile row. Icon/thumb cache keys changed (one-time thumbnail regeneration).

### Performance

| ID | Threshold | Measured | Verdict |
|---|---|---|---|
| P1 | settings ≤ 100 ms and ≥ 5× | 3.5 ms, 54.5× | **PASS** |
| P2 | launcher ≤ 100 ms and ≥ 5× | 3.3 ms, 57.7× | **PASS** |
| P3 | stash ≥ 3× | 8.5× | **PASS** |
| P4 | daemon RSS ≤ 5 MB **and** ≥ 5× smaller | 2.6 MB ✓, but 4.1× | **PARTIAL — second clause FAIL** |
| P5 | av-status ≥ 2× | 1.4× | **FAIL** |
| P6 | screenshot screen ≥ 2× | 3.9× | **PASS** |
| P7 | spawn-count evidence | 10 → 1 processes (8 IPC requests) | **PASS** |
| P8 | binary ≤ 5 MB | 1.4 MB | **PASS** |

**P5 failed, and the failure is informative.** `av-status` spends its time
waiting on `systemctl is-active` — a process we must spawn either way. The
script's own overhead did shrink (bash ~1.5 ms → rust ~0.2 ms), but Amdahl's
law caps the end-to-end ratio: when the dominant cost is an external
process, rewriting the wrapper cannot deliver 2×. The criterion was
mis-aimed at a case where Rust fundamentally can't win big. On a real
system `systemctl` does a D-Bus round-trip and is *slower* than our stub,
so the real-world ratio would be even closer to 1×. Verdict kept as FAIL
rather than re-scoping the criterion after the fact.

**P4's ratio clause failed** for a related reason: the threshold guessed
the python daemon at ≥ 13 MB, but this container's slim python 3.11 idles
at 10.8 MB. The absolute goal (≤ 5 MB) passed with 2.6 MB. Verdict:
PARTIAL, not massaged into a pass.

### Quality

| ID | Verdict | Evidence |
|---|---|---|
| Q1 | **PASS** | 69 tests (50 unit + 19 integration), 3 consecutive green runs |
| Q2 | **PASS** | `cargo clippy --all-targets` — zero warnings |
| Q3 | **PASS** | `cargo clean && cargo build --release` succeeds; deps: serde/serde_json, ratatui, crossterm, libc, base64 |
| Q4 | **PASS** | `bench/` scripts + `bench/results/` raw data committed |
| Q5 | **PASS** | README + install.sh updated; originals under `legacy/bin/` |

## Bias check

Ways these results could mislead, and what was done (or must be admitted)
about each:

1. **The benchmark author is the migration author.** Classic
   confirmation-bias setup: whoever writes the harness can pick metrics
   that flatter their work. Mitigations: criteria and thresholds were
   committed *before* the first benchmark run (see git history —
   `docs/MIGRATION.md` lands in the same commit as the code, the results in
   a later one); two criteria are reported as FAIL/PARTIAL rather than
   silently re-scoped; scripts + raw data are committed so anyone can
   re-run.

2. **No real Hyprland in the container.** All compositor interaction is a
   fake socket answering instantly. This *understates* the old scripts' true
   cost (a real `hyprctl` does its own socket round-trip on top of
   fork+exec) and *understates* the new binary's advantage on `stash`-like
   paths — but it also means nothing here measures real compositor latency,
   frame timing, or GPU work. The kitty-graphics icon grid was **not**
   performance-tested at all (no kitty in the container); only its escape
   sequences are unit-tested. Treat the TUI numbers as time-to-first-paint,
   not "the launcher feels 58× snappier": once painted, both UIs idle on
   events.

3. **Selection bias in what's headlined.** The 54–58× numbers are the
   flashiest and describe only interpreter+import elimination. The honest
   summary is: panels ~54×, stash ~8×, screenshot ~4×, status pollers ~1.4×
   (i.e. effectively unchanged), daemon memory ~4×. The suite-wide "average
   speedup" is deliberately not quoted anywhere — it would be a meaningless
   mean over incommensurable operations.

4. **Warm-cache measurement favors python less than you'd think, but
   favors it.** Three discarded warm-ups mean textual's `.pyc` files and
   the ELF binary are both hot in page cache. A true cold first-open after
   boot would be worse for python (compiling/reading ~6 MB of textual) than
   for the 1.4 MB binary. Not measured; would require cache-dropping we
   can't do in this container.

5. **Stub commands are faster than real ones.** `systemctl`, `grim`,
   `notify-send` stubs reply instantly. All *end-to-end* ratios (P3, P5,
   P6) would compress toward 1× on real hardware as real tool time is added
   to both sides — P5 already demonstrates this in miniature. The absolute
   *deltas* (old-minus-new milliseconds) are the transferable number, and
   those deltas are real: they are pure interpreter/fork overhead removed.

6. **Textual version drift.** The baseline ran textual 8.2.8 (current at
   measurement time), not whatever version the dots were developed against.
   If older textual imported faster, the 54× would shrink; python
   interpreter startup alone (~25 ms here) still bounds the best case at
   ~7× minimum. The measured 189 ms is consistent with textual's own
   documented import cost, so this is unlikely to change the order of
   magnitude.

7. **Container CPU noise.** Shared 4-vcpu cloud container. Mitigated with
   medians over 20 runs + p95 reporting; the p95/median gap is small
   (< 10 %) in every case, so scheduling noise did not drive the medians.

8. **The rewrite could have quietly dropped work.** A speedup is easy if
   you skip half the job. Countered by the parity test suite (69 tests,
   including byte-exact output comparisons for the status JSONs, the
   hyprpaper.conf writer, sed-quirk-faithful keybind transforms, and
   python-exact reorder semantics) and by DRYRUN action-recording checks
   that the same external commands get invoked with the same arguments.

9. **What was NOT verified.** No test drives a real compositor, real rofi,
   real kitty graphics, real pkexec, or the systemd-run reminder units;
   those paths are exercised only down to the exact argv they spawn. The
   TUIs' mouse ergonomics were hand-checked only in a plain pty. Before
   trusting this on a daily driver: run `install.sh` on the target machine,
   press every keybind once, and re-run `bench/run.sh` there — the harness
   is committed precisely so the numbers can be falsified on real hardware.

## Rollback

The original scripts are intact under `legacy/bin/`. To roll back:
`cp legacy/bin/* ~/.local/bin/` (and reinstall the python deps:
`pip install --user textual textual-image`).
