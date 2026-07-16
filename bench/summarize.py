#!/usr/bin/env python3
"""Turn bench/results/raw.jsonl into a markdown summary with speedups."""
import json
import sys

rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
by = {r["name"]: r for r in rows}

PAIRS = [
    ("hypr-settings first paint (pty)", "settings_old", "settings_new"),
    ("hypr-launcher menu first paint (pty)", "launcher_old", "launcher_new"),
    ("hypr-tools stash, 20 windows (end-to-end)", "stash_old", "stash_new"),
    ("av-status (end-to-end)", "av_old", "av_new"),
    ("screenshot screen (end-to-end)", "shot_old", "shot_new"),
]

print("| case | old median | new median | speedup | old p95 | new p95 |")
print("|---|---|---|---|---|---|")
for label, o, n in PAIRS:
    if o not in by or n not in by:
        continue
    om, nm = by[o]["median_ms"], by[n]["median_ms"]
    op, np_ = by[o]["p95_ms"], by[n]["p95_ms"]
    speed = om / nm if nm else float("inf")
    print(f"| {label} | {om:.1f} ms | {nm:.1f} ms | **{speed:.1f}×** | {op:.1f} ms | {np_:.1f} ms |")

if "autohide_old" in by and "autohide_new" in by:
    o, n = by["autohide_old"], by["autohide_new"]
    ratio = o["vmrss_kb"] / n["vmrss_kb"] if n["vmrss_kb"] else float("inf")
    print()
    print("| daemon | VmRSS | VmHWM | cpu ticks / 5 s |")
    print("|---|---|---|---|")
    print(f"| waybar-autohide (python) | {o['vmrss_kb']/1024:.1f} MB | {o['vmhwm_kb']/1024:.1f} MB | {o['cpu_ticks_5s']} |")
    print(f"| waybar-autohide (rust) | {n['vmrss_kb']/1024:.1f} MB | {n['vmhwm_kb']/1024:.1f} MB | {n['cpu_ticks_5s']} |")
    print(f"\nResident-memory ratio: **{ratio:.1f}× smaller**")

if "spawns_stash6" in by:
    s = by["spawns_stash6"]
    print()
    print(f"Process spawns for one `stash` of 6 windows: old = **{s['old_processes']}** "
          f"(bash + python3 + hyprctl×N + notify-send), new = **{s['new_processes']}** "
          f"(notify-send only) with {s['new_ipc_requests']} socket requests instead.")

if "sizes" in by:
    s = by["sizes"]
    print()
    print(f"Rust binary: **{s['rust_binary_bytes']/1e6:.1f} MB** stripped; "
          f"the Textual package alone (not counting python itself, rich, "
          f"textual-image, PIL): {s['textual_pkg_bytes']/1e6:.1f} MB.")
