# Claude Studio — idle cost baseline (29 Aug 2026)

Measured with `draveniq --bench 20` (real machine state: 5 claude
processes, 4 studio windows, 25 recent transcripts) and 30 s `/proc/<pid>/stat`
tick sampling of every `--sidebar` instance (one per window).

| hot path | before | after | how |
|---|---|---|---|
| `session_rows` (every 6 s, visible instance) | 42 ms | **5 ms** | one `hyprctl clients` per reload (cached 30 s) instead of one per running session; one `/proc` walk shared by claude/daemon/codex scans (1 s cache); `daemon_hosted` reads only `comm == claude` cmdlines; `session_meta`/`session_title` cached by (path, mtime, len) |
| `rename_open_tabs` (every 30 s) | 37 ms, 1 + 2·windows forks | **4 ms**, 1–2 forks | list carries current name + `@beside`, no-ops skipped, changes batched into one `tmux a ; b ; c` call |
| `window_active` (every 2 s, **every** instance) | 4.7 ms fork × N | **stat()** every 500 ms | hooks write `$XDG_RUNTIME_DIR/draveniq.active`; tmux asked only every 20 s to reconcile |
| idle CPU, 4 sidebars, 30 s | **1.50 %** | **0.53 %** | inactive instances: 0 ticks; the visible one carries the reload |

Not changed: reload cadence (6 s), rename cadence (30 s), the UX contract.
Remaining cost is the visible instance's reload + redraw (~0.3 %); the next
lever would be an inotify watch on `~/.claude/projects` replacing the timer.

Re-measure: `draveniq --bench 20`, then the 30 s tick script in
`bin/studio-idle.py`.
