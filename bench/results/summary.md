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

Resident-memory ratio: **4.1× smaller**

Process spawns for one `stash` of 6 windows: old = **10** (bash + python3 + hyprctl×N + notify-send), new = **1** (notify-send only) with 8 socket requests instead.

Rust binary: **1.4 MB** stripped; the Textual package alone (not counting python itself, rich, textual-image, PIL): 6.4 MB.
