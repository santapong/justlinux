#!/usr/bin/env bash
# ============================================================
# Benchmark: legacy bash/python tools vs the Rust port.
# Both sides get the SAME stubs, fake IPC socket, HOME and PATH.
# Old scripts additionally get a canned-reply `hyprctl` stub —
# instant replies, which FAVORS the old side (a real hyprctl does
# a socket round-trip; ours costs one fork+exec less than that).
# Results: bench/results/*.json + bench/results/summary.md
# ============================================================
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
RUST_BIN="$ROOT/rust/target/release/justlinux"
[ -x "$RUST_BIN" ] || { echo "build first: (cd rust && cargo build --release)"; exit 1; }
RUNS="${BENCH_RUNS:-20}"

BENCH=$(mktemp -d /tmp/jl-bench-XXXXXX)
RES="$ROOT/bench/results"
mkdir -p "$RES"
: > "$RES/raw.jsonl"

# ---------- bench world ----------
export BENCH_HOME="$BENCH/home"
RUNTIME="$BENCH/runtime"
STUBS="$BENCH/stubs"
mkdir -p "$BENCH_HOME/.config/hypr" "$BENCH_HOME/.config/waybar" \
         "$BENCH_HOME/.local/bin" "$BENCH_HOME/Pictures/Screenshots" \
         "$RUNTIME" "$STUBS"
cp "$ROOT/config/hypr/hyprland.conf" "$BENCH_HOME/.config/hypr/" 2>/dev/null || true
cp "$ROOT/config/waybar/colors.css" "$BENCH_HOME/.config/waybar/" 2>/dev/null || true

SIG=BENCHSIG
SOCKLOG="$BENCH/sock.log"
python3 "$ROOT/bench/fake_hypr.py" "$RUNTIME" "$SIG" 20 "$SOCKLOG" &
FAKE_PID=$!
trap 'kill $FAKE_PID 2>/dev/null || true' EXIT
until [ -S "$RUNTIME/hypr/$SIG/.socket.sock" ]; do sleep 0.05; done

# ---------- stubs (identical for both sides) ----------
mkstub() { # $1 name, rest = script body
    local name="$1"; shift
    printf '#!/bin/bash\n%s\n' "$*" > "$STUBS/$name"
    chmod +x "$STUBS/$name"
}
mkstub systemctl 'exit 0'
mkstub notify-send 'exit 0'
mkstub grim ': > "${@: -1}"'
mkstub wl-copy 'cat > /dev/null'
mkstub wallust 'exit 0'
mkstub swaync-client 'exit 0'
# canned hyprctl for the LEGACY scripts (instant reply => favors legacy)
CLIENTS_JSON=$(python3 - <<'PY'
import json
print(json.dumps([{"address": f"0x{i:04x}", "at": [(i % 5) * 320, (i // 5) * 180],
                   "workspace": {"id": 4, "name": "4"},
                   "title": f"window {i}", "class": "kitty"} for i in range(20)]))
PY
)
cat > "$STUBS/hyprctl" <<EOF
#!/bin/bash
case "\$*" in
    *clients*)          cat "$BENCH/clients.json" ;;
    *activeworkspace*)  echo '{"id":4,"name":"4","monitor":"DP-1"}' ;;
    *activewindow*)     head -c -2 "$BENCH/clients.json" | tail -c +2 > /dev/null; echo '{"address":"0x0000","at":[0,0],"workspace":{"id":4,"name":"4"},"title":"window 0","class":"kitty"}' ;;
    *monitors*)         echo '[{"name":"DP-1","x":0,"y":0,"focused":true},{"name":"HDMI-A-1","x":1600,"y":0,"focused":false}]' ;;
    *)                  echo ok ;;
esac
EOF
chmod +x "$STUBS/hyprctl"
printf '%s' "$CLIENTS_JSON" > "$BENCH/clients.json"

BPATH="$STUBS:/usr/local/bin:/usr/bin:/bin"
COMMON_ENV=(--env "HOME=$BENCH_HOME" --env "XDG_RUNTIME_DIR=$RUNTIME" \
            --env "HYPRLAND_INSTANCE_SIGNATURE=$SIG" --env "PATH=$BPATH")

say() { echo; echo "== $*"; }
bench() { # writes one json line
    python3 "$ROOT/bench/measure.py" "$@" | tee -a "$RES/raw.jsonl"
}

echo "container: $(nproc) cpus · $(uname -r) · $(date -u +%FT%TZ)" | tee "$RES/environment.txt"
python3 -c "import textual; print('textual', textual.__version__)" | tee -a "$RES/environment.txt"
rustc --version | tee -a "$RES/environment.txt"
python3 --version | tee -a "$RES/environment.txt"

# ---------- 1/2: TUI time-to-first-paint (pty) ----------
say "settings TUI first paint"
bench --mode pty --runs "$RUNS" --name settings_old "${COMMON_ENV[@]}" \
    --env HYPRSETTINGS_DRYRUN=1 -- python3 "$ROOT/legacy/bin/hypr-settings"
bench --mode pty --runs "$RUNS" --name settings_new "${COMMON_ENV[@]}" \
    --env HYPRSETTINGS_DRYRUN=1 --env HYPR_BENCH_STARTUP=1 -- "$RUST_BIN" hypr-settings

say "launcher (tools menu) TUI first paint"
bench --mode pty --runs "$RUNS" --name launcher_old "${COMMON_ENV[@]}" \
    --env HYPRSETTINGS_DRYRUN=1 -- python3 "$ROOT/legacy/bin/hypr-launcher" menu
bench --mode pty --runs "$RUNS" --name launcher_new "${COMMON_ENV[@]}" \
    --env HYPRSETTINGS_DRYRUN=1 --env HYPR_BENCH_STARTUP=1 -- "$RUST_BIN" hypr-launcher menu

# ---------- 3: stash (20 windows) ----------
say "stash toggle, 20 windows"
# legacy stash writes a state file then next run restores; delete between runs
# by pointing XDG_RUNTIME_DIR at a per-side scratch dir is not enough — same
# run alternates hide/restore. Both sides get the same alternation, so it is
# symmetric: each sample is one hide OR one restore of 20 windows.
bench --mode exec --runs "$RUNS" --name stash_old "${COMMON_ENV[@]}" \
    -- "$ROOT/legacy/bin/hypr-tools.sh" stash
bench --mode exec --runs "$RUNS" --name stash_new "${COMMON_ENV[@]}" \
    -- "$RUST_BIN" hypr-tools stash

# ---------- 5: av-status ----------
say "av-status"
bench --mode exec --runs "$RUNS" --name av_old "${COMMON_ENV[@]}" \
    -- "$ROOT/legacy/bin/av-status.sh"
bench --mode exec --runs "$RUNS" --name av_new "${COMMON_ENV[@]}" \
    -- "$RUST_BIN" av-status

# ---------- 6: screenshot screen ----------
say "screenshot screen"
bench --mode exec --runs "$RUNS" --name shot_old "${COMMON_ENV[@]}" \
    -- "$ROOT/legacy/bin/screenshot.sh" screen
bench --mode exec --runs "$RUNS" --name shot_new "${COMMON_ENV[@]}" \
    -- "$RUST_BIN" screenshot screen
rm -f "$BENCH_HOME"/Pictures/Screenshots/* 2>/dev/null || true

# ---------- 4: autohide daemon RSS ----------
say "autohide daemon RSS after 5s"
measure_rss() { # $1 name, rest: command
    local name="$1"; shift
    env HOME="$BENCH_HOME" XDG_RUNTIME_DIR="$RUNTIME" \
        HYPRLAND_INSTANCE_SIGNATURE="$SIG" PATH="$BPATH" "$@" &
    local pid=$!
    sleep 5
    local rss hwm
    rss=$(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
    hwm=$(awk '/VmHWM/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
    # cpu time in clock ticks (utime+stime) over the 5s window
    local ticks
    ticks=$(awk '{print $14+$15}' "/proc/$pid/stat" 2>/dev/null || echo 0)
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true
    echo "{\"name\":\"$name\",\"vmrss_kb\":$rss,\"vmhwm_kb\":$hwm,\"cpu_ticks_5s\":$ticks}" \
        | tee -a "$RES/raw.jsonl"
}
measure_rss autohide_old python3 "$ROOT/legacy/bin/waybar-autohide.sh"
measure_rss autohide_new "$RUST_BIN" waybar-autohide

# ---------- 7: spawn-count evidence (single stash run, logging stubs) ----------
say "spawn counts for one stash run (6 windows)"
SPAWN="$BENCH/spawn"; mkdir -p "$SPAWN"
for tool in hyprctl python3 notify-send; do
    # resolve through the stub dir too — hyprctl/notify-send don't exist
    # for real in this container (the stub IS the "real" one here)
    real=$(PATH="$STUBS:$PATH" command -v "$tool")
    cat > "$SPAWN/$tool" <<EOF
#!/bin/bash
echo "$tool" >> "$BENCH/spawns.log"
exec "$real" "\$@"
EOF
    chmod +x "$SPAWN/$tool"
done
# 6-window world for the spawn count
python3 - "$BENCH/clients6.json" <<'PY'
import json, sys
print(json.dumps([{"address": f"0x{i:04x}", "at": [0, 0],
                   "workspace": {"id": 4, "name": "4"},
                   "title": f"w{i}", "class": "kitty"} for i in range(6)]),
      file=open(sys.argv[1], "w"))
PY
cat > "$SPAWN/hyprctl" <<EOF
#!/bin/bash
echo hyprctl >> "$BENCH/spawns.log"
case "\$*" in
    *clients*)         cat "$BENCH/clients6.json" ;;
    *activeworkspace*) echo '{"id":4,"name":"4","monitor":"DP-1"}' ;;
    *)                 echo ok ;;
esac
EOF
chmod +x "$SPAWN/hyprctl"
# a second fake socket serving the SAME 6-window world for the new binary,
# so both sides of the spawn count see identical state
RUNTIME6="$BENCH/runtime6"; SOCKLOG6="$BENCH/sock6.log"
python3 "$ROOT/bench/fake_hypr.py" "$RUNTIME6" "$SIG" 6 "$SOCKLOG6" &
FAKE6_PID=$!
trap 'kill $FAKE_PID $FAKE6_PID 2>/dev/null || true' EXIT
until [ -S "$RUNTIME6/hypr/$SIG/.socket.sock" ]; do sleep 0.05; done
: > "$BENCH/spawns.log"
rm -f "$RUNTIME"/hypr-stash-4.json
env HOME="$BENCH_HOME" XDG_RUNTIME_DIR="$RUNTIME" HYPRLAND_INSTANCE_SIGNATURE=NOSOCK \
    PATH="$SPAWN:$STUBS:/usr/local/bin:/usr/bin:/bin" \
    "$ROOT/legacy/bin/hypr-tools.sh" stash > /dev/null 2>&1 || true
OLD_SPAWNS=$(wc -l < "$BENCH/spawns.log")
: > "$BENCH/spawns.log"; : > "$SOCKLOG6"
env HOME="$BENCH_HOME" XDG_RUNTIME_DIR="$RUNTIME6" HYPRLAND_INSTANCE_SIGNATURE="$SIG" \
    PATH="$SPAWN:$STUBS:/usr/local/bin:/usr/bin:/bin" \
    "$RUST_BIN" hypr-tools stash > /dev/null 2>&1 || true
NEW_SPAWNS=$(wc -l < "$BENCH/spawns.log")
NEW_IPC=$(wc -l < "$SOCKLOG6")
echo "{\"name\":\"spawns_stash6\",\"old_processes\":$OLD_SPAWNS,\"new_processes\":$NEW_SPAWNS,\"new_ipc_requests\":$NEW_IPC}" \
    | tee -a "$RES/raw.jsonl"
# note: old run used NOSOCK so its embedded python could not bypass the stub;
# new run's notify-send also goes through the logging PATH — both counted.

# ---------- 8: binary size ----------
SIZE=$(stat -c %s "$RUST_BIN")
PYSIZE=$(python3 - <<'PY'
import os, textual
root = os.path.dirname(textual.__file__)
total = sum(os.path.getsize(os.path.join(d, f))
            for d, _, fs in os.walk(root) for f in fs)
print(total)
PY
)
echo "{\"name\":\"sizes\",\"rust_binary_bytes\":$SIZE,\"textual_pkg_bytes\":$PYSIZE}" | tee -a "$RES/raw.jsonl"

# ---------- summary ----------
python3 "$ROOT/bench/summarize.py" "$RES/raw.jsonl" > "$RES/summary.md"
echo
cat "$RES/summary.md"
