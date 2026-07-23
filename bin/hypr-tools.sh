#!/usr/bin/env bash
# ============================================================
#  hypr-tools.sh — desktop control menus (rofi-based)
#  Usage:
#    hypr-tools.sh            main menu
#    hypr-tools.sh wallpaper  wallpaper picker (with thumbnails)
#    hypr-tools.sh random     random wallpaper
#    hypr-tools.sh keys       keybinding cheat sheet / editor
# ============================================================
set -uo pipefail

CONF="$HOME/.config/hypr/hyprland.conf"
WALLDIR="$HOME/Pictures/wallpaper"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

pick_wallpaper() {
    # --- 1. which monitor? ---
    local target dir choice f
    target=$( { echo "󰍺  All monitors"
                hyprctl monitors | awk '/^Monitor/{print "󰍹  " $2}'
              } | rofi -dmenu -i -p "󰸉 Apply to")
    [ -z "${target:-}" ] && return 0
    case "$target" in
        *"All monitors"*) target="all" ;;
        *)                target=$(echo "${target#* }" | xargs) ;;
    esac

    # --- 2. browse folders / pick image (thumbnails) ---
    dir="$WALLDIR"
    while :; do
        choice=$( { [ "$dir" != "$HOME" ] && echo "󰁍  .."
                    echo "󰒝  Use this folder (slideshow: random image every 5 min)"
                    find "$dir" -mindepth 1 -maxdepth 1 -type d -printf '󰉋  %f\n' | sort
                    for f in "$dir"/*.jpg "$dir"/*.jpeg "$dir"/*.png; do
                        [ -f "$f" ] && printf '%s\0icon\x1f%s\n' "$(basename "$f")" "$f"
                    done
                  } | rofi -dmenu -i -p "󰸉 ${dir/#$HOME/~}" \
                      -theme-str 'element-icon { size: 56px; } listview { lines: 8; } window { width: 760px; }')
        [ -z "${choice:-}" ] && return 0
        case "$choice" in
            "󰁍  ..")   dir=$(dirname "$dir") ;;
            "󰒝  "*)    wallpaper.sh "$dir" "$target"; return 0 ;;
            "󰉋  "*)    dir="$dir/${choice#󰉋  }" ;;
            *)          wallpaper.sh "$dir/$choice" "$target"; return 0 ;;
        esac
    done
}

# ---------- workspace stash (hide/unhide all windows) ----------

stash_toggle() {
    python3 - <<'PY'
import json, os, subprocess

def hypr(*a):
    return subprocess.run(["hyprctl", *a], capture_output=True, text=True).stdout

ws = json.loads(hypr("activeworkspace", "-j"))
wsid = ws["id"]
if str(ws.get("name", "")).startswith("special"):
    subprocess.run(["notify-send", "Workspace", "You're viewing a hidden stack — go to a normal workspace first"])
    raise SystemExit
stash = f"special:stash{wsid}"
state_file = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), f"hypr-stash-{wsid}.json")
clients = json.loads(hypr("clients", "-j"))

stashed = {c["address"]: c for c in clients if c["workspace"]["name"] == stash}
if stashed:
    # restore in the exact order they were hidden (saved layout order)
    order = []
    if os.path.exists(state_file):
        try:
            order = json.load(open(state_file))
        except Exception:
            order = []
    ordered = [a for a in order if a in stashed] + [a for a in stashed if a not in order]
    for addr in ordered:
        hypr("dispatch", "movetoworkspacesilent", f"{wsid},address:{addr}")
    try:
        os.remove(state_file)
    except OSError:
        pass
    msg = f"󰘸 Restored {len(ordered)} window(s) on workspace {wsid}"
else:
    current = [c for c in clients if c["workspace"]["id"] == wsid]
    if not current:
        msg = f"Workspace {wsid} has no windows to hide"
    else:
        # save layout order: top-left window first, then reading order
        current.sort(key=lambda c: (c["at"][1], c["at"][0]))
        json.dump([c["address"] for c in current], open(state_file, "w"))
        for c in current:
            hypr("dispatch", "movetoworkspacesilent", f"{stash},address:{c['address']}")
        msg = f"󰘸 Hid {len(current)} window(s) — ALT+A again to bring them back"
subprocess.run(["notify-send", "Workspace", msg])
PY
}

# ---------- per-window hide / unhide (minimize) ----------

hide_window() {
    python3 - <<'PY'
import json, os, subprocess

def hypr(*a):
    return subprocess.run(["hyprctl", *a], capture_output=True, text=True).stdout

w = json.loads(hypr("activewindow", "-j") or "{}")
if not w.get("address"):
    subprocess.run(["notify-send", "Hide window", "No focused window"])
    raise SystemExit
if str(w["workspace"]["name"]).startswith("special"):
    subprocess.run(["notify-send", "Hide window", "This window is already on a hidden workspace"])
    raise SystemExit

state = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "hypr-hidden.json")
d = {}
if os.path.exists(state):
    try: d = json.load(open(state))
    except Exception: d = {}
d[w["address"]] = {"ws": w["workspace"]["id"], "title": w.get("title", ""), "cls": w.get("class", "")}
json.dump(d, open(state, "w"))
hypr("dispatch", "movetoworkspacesilent", f"special:hidden,address:{w['address']}")
subprocess.run(["notify-send", "󰘸 Window hidden", f"{w.get('title','')[:60]}\nALT+CTRL+A to bring it back"])
PY
}

unhide_window() {
    python3 - <<'PY'
import json, os, subprocess

def hypr(*a):
    return subprocess.run(["hyprctl", *a], capture_output=True, text=True).stdout

state = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "hypr-hidden.json")
saved = {}
if os.path.exists(state):
    try: saved = json.load(open(state))
    except Exception: saved = {}

clients = [c for c in json.loads(hypr("clients", "-j"))
           if c["workspace"]["name"] == "special:hidden"]
if not clients:
    subprocess.run(["notify-send", "Hidden windows", "Nothing is hidden (per-window). ALT+SHIFT+A hides the focused window."])
    raise SystemExit

lines = [f'{c["class"]}  —  {c["title"][:60]}' for c in clients]
lines.append("󰗐  Restore ALL hidden windows")
r = subprocess.run(
    ["rofi", "-dmenu", "-i", "-format", "i", "-p", "󰘸 Hidden",
     "-mesg", "Enter = bring the window back to its workspace"],
    input="\n".join(lines), capture_output=True, text=True)
out = r.stdout.strip()
if out == "":
    raise SystemExit
idx = int(out)

active_ws = json.loads(hypr("activeworkspace", "-j"))["id"]

def restore(c, follow):
    ws = saved.get(c["address"], {}).get("ws", active_ws)
    verb = "movetoworkspace" if follow else "movetoworkspacesilent"
    hypr("dispatch", verb, f"{ws},address:{c['address']}")
    saved.pop(c["address"], None)

if idx == len(clients):          # Restore ALL
    for c in clients:
        restore(c, follow=False)
    subprocess.run(["notify-send", "󰘸 Restored", f"{len(clients)} window(s) back on their workspaces"])
else:
    restore(clients[idx], follow=True)   # follow: jump to it

json.dump(saved, open(state, "w"))
PY
}

# ---------- power menu ----------

power_menu() {
    local pick c
    pick=$(printf '%s\n' \
        "󰌾  Lock screen" \
        "󰤄  Suspend (sleep)" \
        "󰍃  Logout" \
        "󰜉  Reboot" \
        "⏻  Shutdown" \
        | rofi -dmenu -i -p "⏻ Power" \
            -theme-str 'listview { lines: 5; } window { width: 360px; } element { padding: 12px; } element-text { font: "JetBrainsMono Nerd Font 14"; }')
    case "${pick:-}" in
        *"Lock"*)     hyprlock ;;
        *"Suspend"*)  systemctl suspend ;;
        *"Logout"*)   c=$(printf 'No — stay\nYes — log out\n' | rofi -dmenu -i -p "󰍃 Log out?" -theme-str 'listview { lines: 2; } window { width: 320px; }')
                      [[ "$c" == Yes* ]] && hyprctl dispatch exit ;;
        *"Reboot"*)   c=$(printf 'No — stay\nYes — reboot\n' | rofi -dmenu -i -p "󰜉 Reboot?" -theme-str 'listview { lines: 2; } window { width: 320px; }')
                      [[ "$c" == Yes* ]] && systemctl reboot ;;
        *"Shutdown"*) c=$(printf 'No — stay\nYes — shut down\n' | rofi -dmenu -i -p "⏻ Shut down?" -theme-str 'listview { lines: 2; } window { width: 320px; }')
                      [[ "$c" == Yes* ]] && systemctl poweroff ;;
    esac
}

# ---------- security: firewall / antivirus ----------

firewall_menu() {
    local state="OFF" toggle="turn ON" pick port
    if grep -q '^ENABLED=yes' /etc/ufw/ufw.conf 2>/dev/null; then state="ON"; toggle="turn OFF"; fi
    pick=$(printf '%s\n' \
        "󰕥  Firewall is $state → $toggle" \
        "󰋗  View rules & status" \
        "󰐕  Allow a port…" \
        | rofi -dmenu -i -p "󰕥 Firewall")
    case "${pick:-}" in
        *"turn OFF") pkexec ufw disable && notify-send -u critical "󰕥 Firewall" "DISABLED" ;;
        *"turn ON")  pkexec ufw enable  && notify-send "󰕥 Firewall" "Enabled ✔" ;;
        *"View"*)    kitty --class floatterm --title "Firewall rules" bash -c 'sudo ufw status verbose; echo; read -rp "— Enter to close —"' & ;;
        *"Allow"*)
            port=$(rofi -dmenu -p "󰐕 Port to allow" -l 0 -mesg "e.g. 22, 8080, or 443/tcp")
            [ -n "${port:-}" ] && pkexec ufw allow "$port" \
                && notify-send "󰕥 Firewall" "Port $port allowed" ;;
    esac
}

clamav_menu() {
    local pick gui="󰍜  Open ClamTk (graphical antivirus app)"
    command -v clamtk >/dev/null || gui="󰐕  Install ClamTk — a GUI for ClamAV"
    pick=$(printf '%s\n' \
        "󰃤  Scan Downloads (quick)" \
        "󰋊  Scan whole home folder (slow)" \
        "󰉋  Scan a folder I pick…" \
        "$gui" \
        "󰚰  Update virus definitions now" \
        "󰋗  Service status" \
        | rofi -dmenu -i -p "󰃤 Antivirus")
    case "${pick:-}" in
        *"Downloads"*)
            kitty --class floatterm --title "Virus scan: Downloads" bash -c \
                'echo "Scanning ~/Downloads …"; clamdscan --multiscan --fdpass "$HOME/Downloads"; echo; read -rp "— Enter to close —"' & ;;
        *"home folder"*)
            kitty --class floatterm --title "Virus scan: home" bash -c \
                'echo "Scanning $HOME — this can take a long time. Ctrl+C to stop."; clamdscan --multiscan --fdpass "$HOME"; echo; read -rp "— Enter to close —"' & ;;
        *"folder I pick"*)
            local dir
            dir=$(find "$HOME" -mindepth 1 -maxdepth 2 -type d -not -path '*/.*' 2>/dev/null | sed "s|^$HOME|~|" \
                | rofi -dmenu -i -p "󰉋 Scan which folder?")
            [ -n "${dir:-}" ] && kitty --class floatterm --title "Virus scan" bash -c \
                "echo 'Scanning ${dir} …'; clamdscan --multiscan --fdpass \"\${HOME}${dir#\~}\"; echo; read -rp '— Enter to close —'" & ;;
        *"Open ClamTk"*)     clamtk & ;;
        *"Install ClamTk"*)
            kitty --class floatterm --title "Install ClamTk" bash -c \
                'sudo apt install -y clamtk; echo; read -rp "— Done. Enter to close —"' & ;;
        *"Update"*)
            kitty --class floatterm --title "Update virus definitions" bash -c \
                'sudo systemctl stop clamav-freshclam && sudo freshclam; sudo systemctl start clamav-freshclam; echo; read -rp "— Enter to close —"' & ;;
        *"status"*)
            kitty --class floatterm --title "ClamAV status" bash -c \
                'systemctl status clamav-daemon clamav-freshclam --no-pager | head -30; echo; read -rp "— Enter to close —"' & ;;
    esac
}

# ---------- clock / reminders ----------

new_reminder() {
    local text when unit target now
    text=$(rofi -dmenu -p "⏰ Remind me about…" -l 0 \
        -mesg "Type your reminder text, then press Enter")
    [ -z "${text:-}" ] && return 0
    when=$(printf '5m\n10m\n15m\n30m\n1h\n2h\n' | rofi -dmenu -i -p "󰔛 When?" \
        -mesg "Pick one, or type your own: 45m, 3h, or a clock time like 17:30")
    [ -z "${when:-}" ] && return 0
    unit="reminder-$(date +%s%N)"
    if [[ "$when" =~ ^[0-9]+[smh]$ ]]; then
        systemd-run --user --on-active="$when" --unit="$unit" \
            --description="$text" --setenv=RTEXT="$text" \
            sh -c 'export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"; notify-send -u critical -t 0 "⏰ Reminder" "$RTEXT"' \
            >/dev/null 2>&1
    elif [[ "$when" =~ ^([01]?[0-9]|2[0-3]):[0-5][0-9]$ ]]; then
        target=$(date -d "today $when" +%s); now=$(date +%s)
        [ "$target" -le "$now" ] && target=$(date -d "tomorrow $when" +%s)
        systemd-run --user --on-calendar="$(date -d "@$target" '+%Y-%m-%d %H:%M:00')" --unit="$unit" \
            --description="$text" --setenv=RTEXT="$text" \
            sh -c 'export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"; notify-send -u critical -t 0 "⏰ Reminder" "$RTEXT"' \
            >/dev/null 2>&1
    else
        notify-send -u low "Reminder" "Didn't understand \"$when\" — use 45m, 2h, or 17:30"
        return 0
    fi
    notify-send "⏰ Reminder set" "\"$text\" — $when"
}

list_reminders() {
    local lines unit units sel
    units=$(systemctl --user list-timers 'reminder-*' --no-legend 2>/dev/null | grep -o 'reminder-[0-9]*\.timer')
    if [ -z "$units" ]; then
        notify-send "Reminders" "No reminders scheduled — set one from the clock menu"
        return 0
    fi
    lines=$(for unit in $units; do
        printf '%s │ %s │ %s\n' \
            "$(systemctl --user show "$unit" -p Description --value)" \
            "$(systemctl --user list-timers "$unit" --no-legend | awk '{print $1, $2, $3}')" \
            "$unit"
    done)
    sel=$(printf '%s\n' "$lines" | rofi -dmenu -i -p "󰃰 Reminders" \
        -mesg "Press Enter on a reminder to CANCEL it — Esc to close")
    [ -z "${sel:-}" ] && return 0
    unit="${sel##*│ }"
    systemctl --user stop "$(echo "$unit" | xargs)" 2>/dev/null
    notify-send "Reminders" "Cancelled: ${sel%% │*}"
}

clock_menu() {
    local pick tz t
    pick=$(printf '%s\n' \
        "⏰  New reminder" \
        "󰃰  Reminders — list / cancel" \
        "󰅐  Set timezone" \
        "󰅑  Set date & time manually" \
        "󰑓  Enable automatic time sync (NTP)" \
        | rofi -dmenu -i -p "󰥔 Clock")
    case "${pick:-}" in
        *"New reminder")     new_reminder ;;
        *"Reminders"*)       list_reminders ;;
        *"timezone")
            tz=$(timedatectl list-timezones | rofi -dmenu -i -p "󰅐 Timezone")
            [ -n "$tz" ] && timedatectl set-timezone "$tz" \
                && notify-send "Clock" "Timezone set to $tz" ;;
        *"manually")
            t=$(rofi -dmenu -p "󰅑 New time" -l 0 \
                -mesg "Format: 2026-07-15 20:30:00  or just  20:30  — (this turns NTP off)")
            [ -n "$t" ] && timedatectl set-ntp false && timedatectl set-time "$t" \
                && notify-send "Clock" "Time set to $t (auto-sync disabled)" ;;
        *"NTP")
            timedatectl set-ntp true && notify-send "Clock" "Automatic time sync enabled" ;;
    esac
}

restart_bar() {
    pkill -x waybar; sleep 0.5
    hyprctl dispatch exec waybar >/dev/null
    # a fresh waybar starts visible — restart the auto-hide daemon so its state matches
    if pgrep -f "waybar-autohid[e]" >/dev/null; then
        pkill -f "waybar-autohid[e]"; sleep 0.5
        "$HOME/.local/bin/waybar-autohide.sh" & disown
    fi
}

edit_keybinds() {
    # Build a readable cheat sheet from the config: "line: KEYS → action  # comment"
    local sel line
    sel=$(grep -nE '^\s*bind[elm]*\s*=' "$CONF" \
        | sed -e 's/bind[elm]*\s*=\s*//' \
              -e 's/\$mod SHIFT/ALT+SHIFT/' \
              -e 's/\$mod CTRL/ALT+CTRL/' \
              -e 's/\$mod/ALT/' \
              -e 's/,\s*/ , /' \
        | rofi -dmenu -i -p "󰌌 Keybinds (Enter = edit)" \
            -theme-str 'listview { lines: 12; } window { width: 860px; }')
    [ -z "${sel:-}" ] && return 0
    line=${sel%%:*}
    # Open the config at that exact line; reload Hyprland when the editor closes
    kitty --title "Edit keybinding (save & quit to apply)" \
        sh -c "\${EDITOR:-nvim} +$line '$CONF'; hyprctl reload" &
}

reorder_modules() {
    local CFG="$HOME/.config/waybar/config" list i1 i2 moved
    list=$(python3 - "$CFG" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
NAME = {
    'custom/menu':         '\U000f035c  Menu button',
    'custom/files':        '\U000f024b  File manager',
    'custom/windows':      '\U000f05af  Window switcher',
    'hyprland/workspaces': '\U000f09e0  Workspace numbers',
    'hyprland/window':     '\U000f05b2  Window title',
    'clock':               '\U000f0954  Clock',
    'custom/wallpaper':    '\U000f0e09  Wallpaper picker',
    'custom/keybinds':     '\U000f030c  Keybindings',
    'pulseaudio':          '  Volume',
    'network':             '  Network',
    'cpu':                 '  CPU usage',
    'temperature':         '\U000f050f  CPU temperature',
    'memory':              '  RAM usage',
    'disk':                '\U000f02ca  Disk usage',
    'tray':                '\U000f1294  System tray (app icons)',
    'custom/notification': '\U000f009a  Notification bell',
    'custom/power':        '⏻  Power button',
}
side = {'modules-left': 'LEFT', 'modules-center': 'CENTER', 'modules-right': 'RIGHT'}
for s in side:
    for m in cfg.get(s, []):
        print(f"{NAME.get(m, m)}   — {side[s]} side")
PY
)
    i1=$(printf '%s\n' "$list" | rofi -dmenu -i -format i -p "󰜬 Move" \
        -mesg "Step 1/2 — Which button do you want to move?" \
        -theme-str 'listview { lines: 17; } window { width: 560px; }')
    [ -z "${i1:-}" ] && return 0
    moved=$(printf '%s\n' "$list" | sed -n "$((i1+1))p" | sed 's/ *—.*//')
    i2=$(printf '%s\n󰁔  …to the END of the LEFT side\n󰁔  …to the END of the CENTER\n󰁔  …to the END of the RIGHT side\n' "$list" \
        | rofi -dmenu -i -format i -p "󰜬 Place" \
        -mesg "Step 2/2 — Where should「$moved」go? (it will be placed BEFORE what you pick)" \
        -theme-str 'listview { lines: 20; } window { width: 560px; }')
    [ -z "${i2:-}" ] && return 0
    python3 - "$CFG" "$i1" "$i2" <<'PY'
import json, sys
p, i1, i2 = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
cfg = json.load(open(p))
secs = ['modules-left', 'modules-center', 'modules-right']
flat = [(s, m) for s in secs for m in cfg.get(s, [])]
src_sec, src_mod = flat[i1]
cfg[src_sec].remove(src_mod)
if i2 < len(flat):
    tgt_sec, tgt_mod = flat[i2]
    if (tgt_sec, tgt_mod) != (src_sec, src_mod):
        pos = cfg[tgt_sec].index(tgt_mod)
        cfg[tgt_sec].insert(pos, src_mod)
    else:
        cfg[src_sec].append(src_mod)   # moved before itself: no-op, put it back
else:
    cfg.setdefault(secs[i2 - len(flat)], []).append(src_mod)
json.dump(cfg, open(p, 'w'), indent=4)
PY
    restart_bar
    pgrep -f "waybar-autohid[e]" >/dev/null && notify-send "Waybar" "Reordered (bar is in auto-hide — touch top edge)"
}

toggle_autohide() {
    # smart bar (hypr-appdock bar_smart) superseded the standalone
    # waybar-autohide.sh daemon — this tile now flips the conf key
    if grep -qx 'bar_smart=on' "$HOME/.config/conky/widgets.conf" 2>/dev/null; then
        python3 -c "import sys,os; sys.path.insert(0, os.path.expanduser('~/.local/lib'));
from hyprdesk.confwrite import conf_set; conf_set({'bar_smart': 'off'})"
        "$HOME/.local/bin/hypr-appdock" --ctl reload >/dev/null 2>&1
        notify-send "Waybar" "Smart bar OFF — bar always visible"
    else
        pkill -f waybar-autohide.sh 2>/dev/null   # never run both hiders
        python3 -c "import sys,os; sys.path.insert(0, os.path.expanduser('~/.local/lib'));
from hyprdesk.confwrite import conf_set; conf_set({'bar_smart': 'on'})"
        "$HOME/.local/bin/hypr-appdock" --ctl reload >/dev/null 2>&1
        notify-send "Waybar" "Smart bar ON — touch the top edge to reveal (ALT+B pins)"
    fi
}

main_menu() {
    local pick
    local autohide_state="OFF → turn ON"
    grep -qx 'bar_smart=on' "$HOME/.config/conky/widgets.conf" 2>/dev/null \
        && autohide_state="ON → turn OFF"
    pick=$(printf '%s\n' \
        "  Settings — control panel (click & pick) ALT+X" \
        "󰀻  Apps — launcher                         ALT+R" \
        "󰉋  Apps — file manager                     ALT+E" \
        "󰖯  Apps — window switcher                  ALT+W" \
        "󰸉  Wallpaper — pick image / folder / monitor" \
        "󰒝  Wallpaper — random                      ALT+SHIFT+W" \
        "⏰  Reminder — set new                      ALT+T" \
        "󰃰  Reminder — list / cancel" \
        "󰅐  Clock — time, timezone & sync" \
        "󰕥  Security — firewall (ufw)" \
        "󰃤  Security — antivirus scan (ClamAV)" \
        "󰌌  Keybinds — view / edit / learn          ALT+K" \
        "󰘸  Workspace — hide/unhide all windows     ALT+A" \
        "󰖰  Window — hide focused (minimize)        ALT+SHIFT+A" \
        "󰖯  Window — unhide… (pick from list)       ALT+CTRL+A" \
        "󰜬  Bar — reorder buttons" \
        "󰊠  Bar — hide/show now                     ALT+B" \
        "󰗕  Bar — auto-hide: $autohide_state" \
        "  Config — edit Hyprland" \
        "  Config — edit bar (waybar)" \
        "󰑓  Config — reload Hyprland" \
        | rofi -dmenu -i -p "󰍜 Tools" \
            -mesg "Type to search: <b>wall</b>, <b>bar</b>, <b>remind</b>, <b>key</b>… — every tool is here" \
            -theme-str 'listview { lines: 16; } window { width: 640px; }')
    case "${pick:-}" in
        *"Settings — control"*)  settings_panel ;;
        *"Apps — launcher"*)     launcher apps ;;
        *"file manager"*)        thunar & ;;
        *"window switcher"*)     launcher windows ;;
        *"Wallpaper — pick"*)    launcher wallpaper ;;
        *"Wallpaper — random"*)  wallpaper.sh ;;
        *"Reminder — set"*)      new_reminder ;;
        *"Reminder — list"*)     list_reminders ;;
        *"Clock —"*)             clock_menu ;;
        *"firewall"*)            firewall_menu ;;
        *"antivirus"*)           clamav_menu ;;
        *"Keybinds"*)            edit_keybinds ;;
        *"Workspace —"*)         stash_toggle ;;
        *"Window — hide"*)       hide_window ;;
        *"Window — unhide"*)     unhide_window ;;
        *"Bar — reorder"*)       reorder_modules ;;
        *"Bar — hide/show"*)     "$HOME/.local/bin/bar-toggle.sh" ;;
        *"Bar — auto-hide"*)     toggle_autohide ;;
        *"edit Hyprland")        kitty --title "hyprland.conf" sh -c "\${EDITOR:-nvim} '$CONF'; hyprctl reload" & ;;
        *"edit bar"*)            kitty --title "waybar config" sh -c "\${EDITOR:-nvim} '$HOME/.config/waybar/config' '$HOME/.config/waybar/style.css'; '$HOME/.local/bin/hypr-tools.sh' restart-bar" & ;;
        *"reload Hyprland")      hyprctl reload && notify-send "Hyprland" "Config reloaded ✔" ;;
    esac
}

# If the same window was spawned a moment ago and hasn't mapped yet (slow at
# boot), a repeated keypress must not spawn a duplicate. $1: lock tag, $2: pid to store.
spawn_guard_busy() {
    local lock="${XDG_RUNTIME_DIR:-/tmp}/hypr-spawn-$1.pid" pid
    pid=$(cat "$lock" 2>/dev/null)
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        return 0    # previous spawn still alive but window not mapped yet
    fi
    return 1
}
spawn_guard_set() {
    printf '%s' "$2" > "${XDG_RUNTIME_DIR:-/tmp}/hypr-spawn-$1.pid"
}

settings_panel() {
    # Hypr Settings — click-and-pick TUI control panel (textual) in a glass float
    # $1 (optional): page to open — appearance|bar|wallpaper|widgets|security|power
    local page="${1:-}" addr
    # exact class+title match (a browser tab named "Hypr Settings" must not count)
    addr=$(hyprctl clients -j | python3 -c "
import json, sys
for c in json.load(sys.stdin):
    if c.get('class') == 'floatterm' and c.get('title') == 'Hypr Settings':
        print(c['address']); break" 2>/dev/null)
    if [ -n "$addr" ]; then
        [ -n "$page" ] && printf '%s' "$page" > "${XDG_RUNTIME_DIR:-/tmp}/hypr-settings.page"
        hyprctl dispatch focuswindow "address:$addr" >/dev/null
    elif ! spawn_guard_busy settings; then
        kitty --class floatterm -o background_opacity=0.93 --title "Hypr Settings" python3 "$HOME/.local/bin/hypr-settings" $page &
        spawn_guard_set settings $!
        disown
    fi
}

launcher() {
    # Hypr Launcher — app/window/wallpaper/tools pickers in the Hypr Settings style
    # $1: apps | windows | wallpaper | menu
    local mode="$1" title
    case "$mode" in
        windows)   title="Hypr Windows" ;;
        wallpaper) title="Hypr Wallpaper" ;;
        menu)      title="Hypr Tools" ;;
        *)         title="Hypr Apps" ;;
    esac
    local addr
    addr=$(hyprctl clients -j | TITLE="$title" python3 -c "
import json, os, sys
want = os.environ['TITLE']
for c in json.load(sys.stdin):
    if c.get('class') == 'hyprlauncher' and c.get('title') == want:
        print(c['address']); break" 2>/dev/null)
    if [ -n "$addr" ]; then
        hyprctl dispatch focuswindow "address:$addr" >/dev/null
    elif ! spawn_guard_busy "launcher-$mode"; then
        kitty --class hyprlauncher -o background_opacity=0.93 --title "$title" python3 "$HOME/.local/bin/hypr-launcher" "$mode" &
        spawn_guard_set "launcher-$mode" $!
        disown
    fi
}

case "${1:-menu}" in
    settings)    settings_panel "${2:-}" ;;
    power)       settings_panel power ;;   # power button & ALT+ESC open the Power page
    power-menu)  power_menu ;;             # old rofi power menu, still available
    apps)        launcher apps ;;
    windows)     launcher windows ;;
    reorder)     reorder_modules ;;
    autohide)    toggle_autohide ;;
    wallpaper)   launcher wallpaper ;;
    wallpaper-rofi) pick_wallpaper ;;      # old rofi thumbnail picker, still available
    reminders)   list_reminders ;;
    edit-hypr)   kitty --title "hyprland.conf" sh -c "\${EDITOR:-nvim} '$CONF'; hyprctl reload" & disown ;;
    edit-bar)    kitty --title "waybar config" sh -c "\${EDITOR:-nvim} '$HOME/.config/waybar/config' '$HOME/.config/waybar/style.css'; '$HOME/.local/bin/hypr-tools.sh' restart-bar" & disown ;;
    reload)      hyprctl reload && notify-send "Hyprland" "Config reloaded ✔" ;;
    menu-rofi)   main_menu ;;              # old rofi tools menu, still available
    random)      wallpaper.sh ;;
    keys)        edit_keybinds ;;
    restart-bar) restart_bar ;;
    clock)       clock_menu ;;
    remind)      new_reminder ;;
    stash)       stash_toggle ;;
    hide-window)   hide_window ;;
    unhide-window) unhide_window ;;
    firewall)    firewall_menu ;;
    clamav)      clamav_menu ;;
    *)           launcher menu ;;   # ALT+D / bar Tools button -> tile UI
esac
