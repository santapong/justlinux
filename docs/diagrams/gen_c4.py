#!/usr/bin/env python3
"""Generate the C4 diagrams in this folder.

Every .svg here is produced by this file — do not hand-edit them, edit the
declarations at the bottom and re-run:

    python3 docs/diagrams/gen_c4.py

Why generated: the diagrams have to be re-drawn every time the fleet
changes shape, and hand-editing SVG made that expensive enough to skip.
A box is four numbers and three strings here.

Style rules, kept deliberately narrow so the output survives everywhere:
  · inline attributes only — no CSS, no <style>, no external fonts
  · polygon arrowheads rather than markers (minimal renderers drop markers)
  · self-colored fills with white text, mid-gray connectors, so the same
    file is legible on GitHub's light AND dark themes
  · C4's own palette: person #08427b, this system #1168bd, container
    #438dd5, component #85bbf0 with dark text, external #999999
"""
import html
from pathlib import Path

HERE = Path(__file__).parent

PERSON = "#08427b"
SYSTEM = "#1168bd"
CONTAINER = "#438dd5"
COMPONENT = "#85bbf0"
STORE = "#3a6ea5"
EXTERNAL = "#999999"
LINE = "#888888"
TITLE = "#888888"


def _wrap(text, width):
    """Greedy wrap by estimated glyph width, so a long description does not
    run out of its box."""
    words, lines, cur = text.split(), [], ""
    for w in words:
        trial = f"{cur} {w}".strip()
        if len(trial) <= width or not cur:
            cur = trial
        else:
            lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines


class Diagram:
    def __init__(self, title, w, h, render_w=None):
        self.title, self.w, self.h = title, w, h
        self.render_w = render_w or min(w, 900)
        self.parts = []

    # ----- shapes -----
    def box(self, x, y, w, h, name, kind="", desc="", fill=CONTAINER,
            ink="#ffffff", sub="#d0e0f0", rx=8):
        self.parts.append(
            f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" '
            f'fill="{fill}"/>')
        cx = x + w // 2
        ty = y + 22
        self.parts.append(f'  <g text-anchor="middle">')
        for line in _wrap(name, int(w / 7.2)):
            self.parts.append(
                f'    <text x="{cx}" y="{ty}" font-size="13" '
                f'font-weight="600" fill="{ink}">{html.escape(line)}</text>')
            ty += 15
        if kind:
            self.parts.append(
                f'    <text x="{cx}" y="{ty}" font-size="10" '
                f'font-style="italic" fill="{sub}">[{html.escape(kind)}]'
                f'</text>')
            ty += 15
        for line in _wrap(desc, int(w / 5.4)):
            self.parts.append(
                f'    <text x="{cx}" y="{ty}" font-size="10" fill="{sub}">'
                f'{html.escape(line)}</text>')
            ty += 12
        self.parts.append("  </g>")

    def person(self, x, y, w, h, name, desc=""):
        cx = x + w // 2
        self.parts.append(
            f'  <circle cx="{cx}" cy="{y - 14}" r="18" fill="{PERSON}"/>')
        self.box(x, y, w, h, name, "Person", desc, fill=PERSON)

    def component(self, x, y, w, h, name, kind="", desc=""):
        self.box(x, y, w, h, name, kind, desc, fill=COMPONENT,
                 ink="#0b2545", sub="#123a63")

    def external(self, x, y, w, h, name, kind="", desc=""):
        self.box(x, y, w, h, name, kind, desc, fill=EXTERNAL)

    def store(self, x, y, w, h, name, kind="", desc=""):
        """A datastore: same box with a lid, C4's cylinder in spirit."""
        self.parts.append(
            f'  <ellipse cx="{x + w // 2}" cy="{y + 8}" rx="{w // 2}" '
            f'ry="8" fill="{STORE}"/>')
        self.box(x, y + 8, w, h - 8, name, kind, desc, fill=STORE, rx=4)

    def boundary(self, x, y, w, h, label):
        self.parts.append(
            f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" '
            f'fill="none" stroke="{LINE}" stroke-width="1.5" '
            f'stroke-dasharray="7 5"/>')
        self.parts.append(
            f'  <text x="{x + 12}" y="{y + 20}" font-size="11" '
            f'font-weight="600" fill="{LINE}">{html.escape(label)}</text>')

    # ----- connectors -----
    def arrow(self, x1, y1, x2, y2, label="", dash=False, lx=None, ly=None,
              anchor="middle", both=False):
        d = ' stroke-dasharray="6 4"' if dash else ""
        self.parts.append(
            f'  <line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" '
            f'stroke="{LINE}" stroke-width="1.6"{d}/>')
        self.parts.append(self._head(x1, y1, x2, y2))
        if both:
            self.parts.append(self._head(x2, y2, x1, y1))
        if label:
            mx = lx if lx is not None else (x1 + x2) // 2
            my = ly if ly is not None else (y1 + y2) // 2
            for i, line in enumerate(_wrap(label, 26)):
                self.parts.append(
                    f'  <text x="{mx}" y="{my + i * 12}" font-size="10" '
                    f'text-anchor="{anchor}" fill="{LINE}">'
                    f'{html.escape(line)}</text>')

    def _head(self, x1, y1, x2, y2):
        import math
        a = math.atan2(y2 - y1, x2 - x1)
        s = 7
        p = [(x2, y2),
             (x2 - s * math.cos(a - 0.42), y2 - s * math.sin(a - 0.42)),
             (x2 - s * math.cos(a + 0.42), y2 - s * math.sin(a + 0.42))]
        pts = " ".join(f"{px:.1f},{py:.1f}" for px, py in p)
        return f'  <polygon points="{pts}" fill="{LINE}"/>'

    def note(self, x, y, text, size=10, anchor="start"):
        for i, line in enumerate(text.split("\n")):
            self.parts.append(
                f'  <text x="{x}" y="{y + i * 13}" font-size="{size}" '
                f'text-anchor="{anchor}" fill="{LINE}">{html.escape(line)}'
                f'</text>')

    # ----- output -----
    def write(self, name):
        body = "\n".join(self.parts)
        svg = (f'<svg xmlns="http://www.w3.org/2000/svg" '
               f'viewBox="0 0 {self.w} {self.h}" width="{self.render_w}" '
               f'font-family="Segoe UI, Helvetica, Arial, sans-serif">\n'
               f'  <!-- GENERATED by docs/diagrams/gen_c4.py — do not edit. -->\n'
               f'  <text x="24" y="34" font-size="16" font-weight="700" '
               f'fill="{TITLE}">{html.escape(self.title)}</text>\n'
               f'{body}\n</svg>\n')
        (HERE / name).write_text(svg)
        print(f"  wrote {name} ({len(svg)} bytes)")


# ===================== Level 1 — System Context =====================
def context():
    d = Diagram("System Context — justlinux desktop (C4 level 1)", 1040, 700)
    d.person(420, 78, 200, 66, "santapong",
             "runs the desktop, presses the keys")
    d.box(370, 250, 300, 120, "justlinux desktop fleet",
          "Software System",
          "Widget cards, docks, TUI panels, grid edit mode, the Claude "
          "workspace and the Habitica board", fill=SYSTEM)
    d.external(40, 250, 220, 96, "Hyprland 0.55", "Wayland compositor",
               "hosts every surface; queried with hyprctl -j and watched "
               "on socket2")
    d.external(40, 430, 220, 86, "waybar", "top panel",
               "driven, not drawn: the fleet toggles and re-themes it")
    d.external(780, 250, 220, 96, "wallust", "palette generator",
               "turns the wallpaper into the colours every surface uses")
    d.external(780, 430, 220, 110, "Claude Code", "CLI + daemon",
               "transcripts under ~/.claude, job state, MCP servers")
    d.external(420, 560, 200, 86, "Habitica", "REST API v3",
               "to-dos, dailies, habits, tags")
    d.external(140, 590, 220, 76, "Data APIs", "HTTP",
               "GitHub, CoinGecko, wttr.in")
    d.external(780, 590, 220, 76, "Docker / k8s", "daemon + kubectl",
               "containers, compose, pods")
    d.arrow(520, 150, 520, 246, "uses", lx=530, ly=200, anchor="start")
    d.arrow(370, 300, 266, 300, "lays out surfaces on", lx=318, ly=286)
    d.arrow(400, 370, 300, 428, "hides and reveals", lx=250, ly=400)
    d.arrow(670, 300, 776, 300, "re-colours from", lx=723, ly=286)
    d.arrow(660, 370, 790, 428, "opens sessions, reads state",
            lx=800, ly=400, anchor="start")
    d.arrow(520, 374, 520, 556, "reads and writes tasks",
            lx=530, ly=470, anchor="start")
    d.arrow(430, 372, 300, 586, "polls, caches, marks stale",
            lx=372, ly=520, anchor="start")
    d.arrow(620, 372, 800, 586, "controls, tails logs",
            lx=640, ly=520, anchor="start")
    d.note(24, 676, "External systems are grey. waybar is outside the "
                    "system on purpose: the fleet drives it, it is not "
                    "part of it.")
    d.write("c4-context.svg")


# ===================== Level 2 — Containers =========================
def containers():
    """Laid out in layers, with every connector running in a clear
    channel between boxes. Since v1.4.0 the RESIDENT row is 100% Rust
    (4.65% CPU / 64 MB total, measured); python remains only where a
    process runs on demand and exits — the permanently-hybrid policy
    (docs/language-policy.md)."""
    d = Diagram("Containers — justlinux desktop (C4 level 2)", 1240, 1080)
    d.boundary(30, 60, 1180, 740, "justlinux desktop fleet [Software System]")
    d.note(56, 88, "RESIDENT — always running, all Rust "
                   "(rust/, one binary each, ~2-16 MB)")
    xs = [56, 286, 516, 746, 976]
    yA = 100
    for x, (n, k, dsc) in zip(xs, [
            ("hypr-cardhost", "Rust / layer-shell",
             "all 14 glass cards from TOML, one process, ctl socket"),
            ("hypr-appdock", "Rust / layer-shell",
             "per-monitor docks, reveal strips, the smart top bar"),
            ("hypr-office2d", "Rust / layer-shell",
             "the floor office: a walking agent per Claude session"),
            ("hypr-pet", "Rust / layer-shell",
             "the desktop creatures"),
            ("serial-watch", "Rust",
             "board hotplug toasts; busts the robotics cache")]):
        d.box(x, yA, 210, 96, n, k, dsc)
    yB = 228
    for x, (n, k, dsc) in zip(xs, [
            ("draveniq", "Rust / kitty+tmux+ratatui",
             "tabbed session workspace; sidebar is tab 0"),
            ("hypr-docker", "Rust / ratatui",
             "containers, compose, images, logs, a Kubernetes pane"),
            ("hypr-viz", "Rust / layer-shell",
             "ambient audio visualizer (toggle, not always-on)"),
            ("hypr-launcher", "Python / Textual",
             "apps, windows, wallpaper, tools, session recall"),
            ("hypr-settings", "Python / Textual",
             "every setting; network, widgets, MCP, Habitica")]):
        d.box(x, yB, 210, 96, n, k, dsc)
    d.note(746, 220, "ON DEMAND — opens, does its job, exits (zero resident cost)")
    yC = 356
    for x, (n, k, dsc) in zip(xs, [
            ("hypr-kanban", "Python / Textual",
             "the Habitica board: today, sprint, week"),
            ("hypr-arrange", "Python / GTK3",
             "ALT+SHIFT+E grid edit mode for movable surfaces"),
            ("hypr-widgetpicker", "Python / GTK3",
             "add, remove and parameterise cards"),
            ("appdock picker", "Python / GTK3",
             "the dock's searchable app checklist"),
            ("hypr-tools.sh", "bash",
             "the dispatcher every keybind goes through")]):
        d.box(x, yC, 210, 96, n, k, dsc)
    yD = 484
    d.box(56, yD, 320, 84, "desktop-widgets.sh", "bash",
          "exec-once supervisor; guards BOTH python and binary cmdlines")
    d.box(396, yD, 320, 84, "widget-*.sh", "bash",
          "data fetchers; --fields emits key=value for the card host")
    d.box(736, yD, 450, 84, "wallpaper.sh · check-contrast.sh", "bash",
          "re-theme everything (wallust -> fleet USR2/ctl -> kitty USR1)")
    yE = 600
    d.box(56, yE, 550, 76, "rust/hyprdesk", "Rust crate",
          "conf · theme · grid · cardspec · rows · draw · sessions — "
          "parity-tested against the python twin")
    d.box(636, yE, 550, 76, "lib/hyprdesk", "Python library",
          "same contracts for the on-demand python: conf · theme · "
          "layer · claudesessions · habitica · secrets")
    yF = 700
    for x, (n, k, dsc) in zip([56, 326, 596, 866], [
            ("widgets.conf", "key=value",
             "single source of truth; atomic flock'd writes"),
            ("pins.json", "JSON v2", "per-monitor, per-workspace dock pins"),
            ("secrets.json", "JSON, mode 0600",
             "Habitica credentials — never the public repo"),
            ("~/.claude", "JSONL + JSON",
             "Claude's transcripts, job state, daemon roster")]):
        d.store(x, yF, 240, 84, n, k, dsc)
    d.note(60, yF - 10, "the FILES are the interface: both libraries parse the "
                        "same bytes, so the two implementations cannot drift "
                        "apart without a visible symptom")
    ext = [("Hyprland", "compositor", "hyprctl -j, socket2 events",
            "hosted by"),
           ("wallust", "palette", "wallpaper colours", "re-colours from"),
           ("Docker / kubectl", "daemons",
            "ps, compose, logs, pods", "controls"),
           ("Claude Code", "CLI + daemon",
            "claude --resume, jobs, transcripts", "opens and reads")]
    for x, (n, k, dsc, rel) in zip([56, 326, 596, 866], ext):
        d.external(x, 880, 240, 96, n, k, dsc)
        d.arrow(x + 120, 804, x + 120, 876, rel, lx=x + 128, ly=845,
                anchor="start")
    d.note(60, 1030,
           "Two rules hold it together. One config file: everything "
           "user-tunable lives in widgets.conf and every write goes "
           "through one atomic writer.\n"
           "One process per concern over one SHARED CONTRACT: palette "
           "roles, conf grammar, ctl verbs and layer namespaces are "
           "identical in both languages,\n"
           "so the desktop re-colours together. Terminal panels take the "
           "palette and the ink hierarchy but never the pixel icons.")
    d.write("c4-container.svg")


# ============ Level 3 — Components: Claude Studio ===================
def studio():
    d = Diagram("Components — Claude Studio (C4 level 3)", 1080, 760)
    d.boundary(30, 60, 1020, 570, "draveniq [Container]")
    d.box(60, 96, 250, 110, "launch()", "Python",
          "focuses the existing window and summons it to this workspace, "
          "or spawns kitty running tmux")
    d.component(370, 96, 250, 110, "sidebar (tab 0)", "Textual App",
                "the session tree: Enter opens a tab, s opens one beside "
                "it, t a terminal, m the settings panel")
    d.component(680, 96, 250, 110, "style_tmux()", "tmux options",
                "the fleet's colours, the per-tab close button, the "
                "split buttons; "
                "idempotent, so it re-runs on every start")
    d.component(60, 268, 250, 110, "open_session_beside()", "Python",
                "splits the tab you last read and resumes the chosen "
                "conversation in the new pane")
    d.component(370, 268, 250, 110, "_work_window()", "Python",
                "picks which tab a split lands in — never window 0, which "
                "the tree owns")
    d.component(680, 268, 250, 110, "rename_open_tabs()", "Python",
                "gives each tab Claude's own name for its FIRST pane")
    # the band is narrower than the row so two connectors can run down
    # the outside of it: the tmux server does not read claudesessions,
    # and a diagram that says it does is worse than one arrow fewer
    d.box(200, 440, 590, 76, "tmux server", "own socket",
          "-L draveniq with -f /dev/null: the studio can never "
          "inherit or restyle your real tmux", fill=STORE)
    d.external(60, 590, 250, 86, "claudesessions", "library",
               "session_rows, session_title, transcript_for")
    d.external(370, 590, 250, 86, "hypr-settings", "Textual TUI",
               "run as a tab for the MCP servers panel")
    d.external(680, 590, 250, 86, "claude --resume", "CLI",
               "one process per pane")
    d.arrow(310, 150, 366, 150, "runs as tab 0", ly=142)
    d.arrow(620, 150, 676, 150)
    d.arrow(430, 206, 250, 264, "s opens one beside", lx=250, ly=232)
    d.arrow(560, 206, 740, 264, "every 30 s", lx=640, ly=232)
    for x in (250, 495, 740):
        d.arrow(x, 378, x, 436)
    d.arrow(140, 378, 140, 586, "reads", lx=150, ly=470, anchor="start")
    d.arrow(495, 516, 495, 586, "runs as a tab", lx=505, ly=556,
            anchor="start")
    d.arrow(870, 378, 870, 586, "one per pane", lx=862, ly=470,
            anchor="end")
    d.note(60, 706,
           "The sidebar is a Textual app that owns its whole window, "
           "which is why a split button clicked there opens a terminal "
           "TAB instead of cutting the tree in half.\n"
           "A tab's name comes from its first pane only: walking every "
           "pane meant opening a second conversation beside the first "
           "renamed the whole tab to it.")
    d.write("c4-component-studio.svg")


# ============ Level 3 — Components: the 2D office ===================
def office():
    d = Diagram("Components — the 2D Claude office (C4 level 3)", 1080, 740)
    d.boundary(30, 60, 1020, 470, "hypr-claude-office [Container]")
    d.component(60, 96, 230, 100, "claude_procs()", "library",
                "live sessions, from /proc — no pgrep fork in the poll "
                "loop")
    d.component(60, 236, 230, 100, "daemon_roster()", "library",
                "sessions whose terminal closed but which the daemon "
                "still hosts")
    d.component(370, 160, 250, 116, "transcript_for()", "library",
                "binds a pid to a conversation: argv first, then unclaimed "
                "transcripts paired by start time within 120 s")
    d.component(700, 160, 230, 116, "state decision", "Python",
                "Claude's own job state, with CPU only as a weak floor; "
                "attention is tested BEFORE sleep")
    d.component(370, 340, 250, 100, "draw()", "cairo",
                "the desks, repainted only as fast as something is "
                "actually moving")
    d.component(700, 340, 230, 100, "_write_badge()", "Python",
                "~/.cache/hyprdesk/office.json for the waybar badge")
    d.external(60, 566, 230, 76, "~/.claude/projects", "JSONL",
               "transcripts, with Claude's own title")
    d.external(370, 566, 250, 76, "~/.claude/jobs", "JSON",
               "job state per session")
    d.external(700, 566, 230, 76, "waybar", "panel",
               "reads the badge file")
    d.arrow(290, 146, 366, 190)
    d.arrow(290, 286, 366, 240)
    d.arrow(620, 218, 696, 218)
    d.arrow(815, 276, 600, 340, "state per desk", lx=700, ly=316)
    d.arrow(815, 276, 815, 336)
    d.arrow(175, 336, 175, 562, "reads", lx=185, ly=460, anchor="start")
    d.arrow(495, 440, 495, 562, "reads")
    d.arrow(815, 440, 815, 562, "writes")
    d.note(60, 690, "Every 2 s the desk list is rebuilt; the scene "
                    "repaints at a rate that follows the work, not on a "
                    "fixed timer — a static scene\nrepainted six times a "
                    "second cost 4% of a core.")
    d.write("c4-component-office.svg")


# ============ Level 3 — Components: the Habitica board ==============
def habitica():
    d = Diagram("Components — the Habitica board (C4 level 3)", 1160, 830)
    d.boundary(30, 60, 1100, 520, "hypr-kanban [Container]")
    d.component(60, 96, 220, 116, "six TabPanes", "Textual",
                "Today, To-Dos, Sprint, Week, Dailies, Habits — one per "
                "question Habitica can answer")
    d.component(60, 250, 220, 116, "Card", "Textual widget",
                "one task; press, move, release to drag — the carried card "
                "dims, the target zone lights")
    d.component(340, 96, 230, 116, "DropZone", "Textual container",
                "carries a key like col:doing or due:2026-07-29, so one "
                "drag serves board, sprint and week")
    d.component(340, 250, 230, 116, "repaint()", "Python",
                "redraws every tab from memory — no network, so a move "
                "costs one request, not six")
    d.component(630, 96, 230, 116, "DuePicker", "Modal screen",
                "today, tomorrow, +3 days, next Monday, end of sprint, "
                "clear")
    d.component(630, 250, 230, 116, "board_column()", "library",
                "which column a to-do is in: completed, the doing tag, "
                "sprint membership")
    d.box(910, 96, 190, 270, "hyprdesk.habitica", "API client",
          "board, set_column, set_due, tags, score; a 60 s cache in front "
          "of a ~30 request per minute limit", fill=CONTAINER)
    # Habitica on the right, secrets on the left: the two connectors
    # leave the client at different corners and never cross
    d.store(340, 570, 250, 106, "secrets.json", "0600",
            "user id and API token, validated before they are stored")
    d.external(700, 590, 250, 86, "Habitica REST v3", "HTTPS",
               "tasks, completedTodos, tags, score")
    d.arrow(280, 154, 336, 154)
    d.arrow(280, 308, 336, 308)
    d.arrow(570, 154, 626, 154)
    d.arrow(570, 308, 626, 308)
    d.arrow(860, 210, 906, 210)
    d.arrow(860, 308, 906, 308)
    d.arrow(1050, 366, 850, 586, "reads and writes", lx=980, ly=480,
            anchor="start")
    d.arrow(890, 366, 480, 566, "reads the keys", lx=600, ly=470)
    d.note(60, 730,
           "Doing and sprints are TAGS — `doing` and `sprint-2026-W31` —\n"
           "because Habitica has neither state. A tag is real: the phone\n"
           "and the website see it, and can undo it. Neither is created\n"
           "by opening the board, only by moving a card that needs it.")
    d.write("c4-component-habitica.svg")


# ============ Level 3 — Components: hypr-appdock =====================
def appdock():
    d = Diagram("Components — hypr-appdock (C4 level 3)", 1060, 790)
    d.boundary(30, 60, 1000, 520, "hypr-appdock [Container]")
    d.component(60, 96, 250, 120, "Manager", "Python",
                "owns everything: spawns docks per monitor, debounces the "
                "socket2 stream, respawns on hotplug, 5 s hide-sanity pass")
    d.component(390, 96, 250, 120, "EdgeStrip", "GTK layer surface",
                "an invisible 4 px strip; a 180 ms dwell separates resting "
                "on the edge from crossing between stacked monitors")
    d.component(720, 96, 250, 120, "Dock", "GTK layer surface",
                "the bottom bar itself: pinned apps, running windows, the "
                "picker button")
    d.component(60, 270, 250, 120, "bar_* controller", "Python",
                "drives waybar, a FOREIGN surface with no GTK crossing "
                "events: probes hyprctl layers, toggles with SIGUSR1")
    d.component(390, 270, 250, 120, "ctl socket", "unix socket",
                "reload, reload-theme, show-all/resume, bar-pin")
    d.component(720, 270, 250, 120, "Picker", "GTK window",
                "add or remove a pinned app; also reachable from Settings, "
                "since the desktop may be covered")
    d.store(390, 430, 250, 116, "pins.json", "JSON v2",
            "per-monitor, per-workspace; flock'd read-modify-write")
    d.external(60, 600, 250, 86, "Hyprland", "compositor",
               "hyprctl layers, socket2 events")
    d.external(390, 600, 250, 86, "waybar", "panel", "SIGUSR1, CSS class")
    d.external(720, 600, 250, 86, "hypr-tools.sh", "bash",
               "every keybind arrives here")
    d.arrow(310, 140, 386, 140)
    d.arrow(640, 140, 716, 140)
    d.arrow(185, 216, 185, 266)
    d.arrow(310, 330, 386, 330)
    d.arrow(640, 330, 716, 330)
    d.arrow(515, 390, 515, 426, "reads and writes")
    d.arrow(185, 390, 185, 596, "polls and signals", lx=195, ly=500,
            anchor="start")
    # waybar is driven by the bar_* controller, not by the pins file.
    # This line stays left of pins.json for its whole length.
    d.arrow(300, 390, 400, 596, "toggles with SIGUSR1", lx=310, ly=582,
            anchor="end")
    d.arrow(845, 390, 845, 596, "", both=True)
    d.note(60, 730, "Docks are suppressed only by TRUE fullscreen "
                    "(client mode 2) — double-click-maximize is mode 1 "
                    "and keeps every edge\nsurface reachable. The Picker "
                    "is also a Settings button, because a bottom-layer "
                    "dock is unreachable\nwhen a window covers the "
                    "desktop.")
    d.write("c4-component-appdock.svg")


# ===================== Dynamic — the smart-bar cycle ================
def smartbar():
    d = Diagram("Dynamic — the smart top bar (C4 dynamic)", 1080, 560)
    d.person(60, 110, 200, 66, "santapong", "moves the pointer")
    d.component(320, 96, 230, 96, "EdgeStrip (top)", "4 px layer surface",
                "invisible; 180 ms dwell")
    d.component(320, 260, 230, 96, "bar_* (appdock)", "Rust",
                "the smart-bar controller")
    d.external(650, 96, 230, 96, "waybar", "panel",
               "runs exclusive: false")
    d.external(650, 260, 230, 96, "Hyprland", "compositor",
               "hyprctl layers")
    d.note(60, 300,
           "1  pointer rests on the top edge\n"
           "2  after 180 ms, reveal\n"
           "3  probe: is it already shown?\n"
           "4  SIGUSR1 toggles it\n"
           "5  poll the cursor at 400 ms\n"
           "   ONLY while it is revealed\n"
           "6  pointer leaves -> hide again")
    d.arrow(260, 144, 316, 144, "1  dwell", ly=136)
    d.arrow(550, 144, 646, 144, "2  reveal", ly=136)
    d.arrow(435, 192, 435, 256, "")
    d.arrow(550, 308, 646, 308, "3  probe state", ly=300)
    d.arrow(550, 290, 646, 180, "4  SIGUSR1", lx=600, ly=250)
    d.arrow(320, 290, 244, 182, "6  hide", lx=236, ly=250, anchor="end")
    d.note(60, 460,
           "waybar runs exclusive: false on purpose. An exclusive bar "
           "releases its zone when hidden, which would re-tile\n"
           "every window by 30 px on each reveal. Hidden means opacity 0 "
           "plus a layerrule ignore_alpha, so an\nunseen bar cannot eat "
           "a desktop click, and a respawned waybar is re-tucked by the "
           "5 s sanity pass.")
    d.write("c4-dynamic-smartbar.svg")


# ===================== 4+1 — Process view ===========================
def process_view():
    """Kruchten 4+1 process view: what RUNS, and every IPC edge between
    the running things. This is the diagram to read before touching a
    socket, a signal or the conf file."""
    d = Diagram("Process view — runtime processes and IPC (4+1)", 1240, 800)
    d.boundary(30, 56, 720, 380, "resident fleet [one Rust process each]")
    d.component(60, 96, 200, 76, "hypr-cardhost", "ctl: hyprcard.sock",
                "ping | reload | reload-theme")
    d.component(290, 96, 200, 76, "hypr-appdock", "ctl: hypr-appdock.sock",
                "+ bar-pin | show-all | resume")
    d.component(520, 96, 200, 76, "hypr-office2d", "SIGUSR1 / USR2",
                "reposition / retheme")
    d.component(60, 200, 200, 76, "hypr-pet", "SIGUSR1 / USR2", "")
    d.component(290, 200, 200, 76, "hypr-viz", "SIGUSR1 / USR2",
                "run-again kills (toggle)")
    d.component(520, 200, 200, 76, "serial-watch", "2 s poll",
                "/dev/serial/by-id")
    d.component(60, 304, 430, 76, "draveniq tmux server",
                "own socket -L draveniq",
                "sidebar (ratatui) is pane 0; tabs run claude")
    d.component(520, 304, 200, 76, "hypr-docker", "child streams",
                "docker/kubectl logs -f")
    d.external(800, 96, 180, 76, "Hyprland", "socket2",
               "events: workspaces, monitors")
    d.external(800, 200, 180, 76, "waybar", "SIGUSR1/USR2",
               "toggle / restyle")
    d.external(800, 304, 180, 76, "docker daemon", "unix socket",
               "via the docker CLI")
    d.store(1010, 96, 180, 76, "widgets.conf", "flock + rename",
            "every writer atomic")
    d.store(1010, 200, 180, 76, "pins.json", "flock",
            "dock <-> picker handshake")
    d.store(1010, 304, 180, 76, "~/.claude", "mtime watch",
            "transcripts drive trees")
    d.arrow(724, 134, 796, 134, "watch", ly=126)
    d.arrow(724, 238, 796, 238, "SIGUSR1", ly=230)
    d.arrow(724, 342, 796, 342, "spawn", ly=334)
    d.arrow(984, 134, 1006, 134)
    d.arrow(984, 238, 1006, 238)
    d.arrow(984, 342, 1006, 342)
    d.note(40, 470,
           "Contracts every process honours:\n"
           "  - ctl sockets answer ok/err and PROBE-EXIT if another "
           "instance owns the endpoint (no duplicate fleets)\n"
           "  - USR1 = reposition, USR2 = retheme, everywhere a surface "
           "can move or re-ink\n"
           "  - conf writes: flock + tmp + rename, undo file kept; "
           "readers grep key=value (NO spaces — measured trap)\n"
           "  - process guards match BOTH cmdline forms (python3 <path> "
           "and <path>) with pgrep/pkill -xf, never -f\n"
           "  - async child streams carry generation tokens: a "
           "superseded fetch can never write into the pane it lost")
    d.note(40, 720,
           "Wake sources, not busy loops: calloop timers (cardhost 1 s, "
           "appdock 200 ms), tmux/ratatui event polls,\n"
           "socket2 line events. Total resident cost, measured 4 Aug "
           "2026: 4.65% CPU / 64 MB.")
    d.write("c4-process-view.svg")


# ===================== 4+1 — Development view =======================
def development():
    d = Diagram("Development view — repo layout and build (4+1)", 1240, 620)
    d.boundary(30, 56, 560, 470, "rust/ [cargo workspace]")
    d.component(56, 96, 240, 70, "hyprdesk", "shared crate",
                "conf, theme, grid, cardspec, rows, draw, sessions")
    d.component(316, 96, 240, 70, "examples/", "parity harnesses",
                "run python + rust on real data, diff outputs")
    for i, (n, dsc) in enumerate([
            ("hypr-cardhost", "cards"), ("hypr-appdock", "docks"),
            ("hypr-office2d", "office"), ("hypr-pet", "pet"),
            ("hypr-viz", "visualizer"), ("hypr-studio", "studio"),
            ("hypr-docker", "docker+k8s"), ("hypr-serialwatch", "hotplug")]):
        x = 56 + (i % 2) * 260
        y = 196 + (i // 2) * 80
        d.component(x, y, 240, 64, n, "bin crate", dsc)
    d.boundary(640, 56, 560, 470, "python + bash [on-demand]")
    d.box(666, 96, 240, 70, "bin/", "python TUIs + shell",
          "settings, launcher, kanban, pickers, arrange, widget-*.sh")
    d.box(926, 96, 240, 70, "lib/hyprdesk/", "python library",
          "the same contracts, for the on-demand half")
    d.box(666, 196, 500, 64, "config/", "deployed dotfiles",
          "hypr, waybar, kitty, wallust templates, applications")
    d.box(666, 290, 500, 64, "docs/ + docs/diagrams/gen_c4.py", "this",
          "diagrams are GENERATED - edit the script, not the svg")
    d.box(666, 384, 500, 64, "install.sh", "one entry point",
          "copies bin+lib+config, then cargo builds and installs the "
          "eight binaries LAST so the binary wins")
    d.note(40, 560,
           "Branch model: develop is the integration branch, main gets "
           "--no-ff release merges + annotated tags (v1.4.0).\n"
           "Policy: docs/language-policy.md — resident = Rust, "
           "on-demand = python, the files are the interface.")
    d.write("c4-development-view.svg")


# ===================== 4+1 — Physical view ==========================
def deployment():
    d = Diagram("Physical view — where things land and draw (4+1)", 1240, 640)
    d.boundary(30, 56, 1180, 300, "one machine [Kali linux, Wayland]")
    d.store(56, 96, 260, 84, "~/.local/bin", "install target",
            "8 rust binaries + python tools; PATH runs these")
    d.store(346, 96, 260, 84, "~/.config", "hypr, waybar, kitty,\n"
            "wallust, conky", "widgets.conf lives in conky/")
    d.store(636, 96, 260, 84, "~/.local/state + runtime", "pins, caches,\n"
            "ctl sockets", "$XDG_RUNTIME_DIR/*.sock")
    d.store(926, 96, 260, 84, "~/.claude", "transcripts",
            "sessions the studio and office render")
    d.component(56, 220, 360, 100, "3 monitors x layer shell",
                "background < bottom < top < overlay",
                "cards+office+viz on bottom; docks, strips, bar on top; "
                "arrange overlay on top of everything")
    d.component(446, 220, 360, 100, "kitty windows", "floating, blurred",
                "studio (draveniq), docker (hyprdocker), "
                "exec shells (hyprdockerexec)")
    d.component(836, 220, 350, 100, "layerrules", "blur + ignore_alpha",
                "namespaces are the CONTRACT: hypr-card-*, "
                "hypr-appdock*, hypr-dockedge-*, hypr-viz")
    d.note(40, 400,
           "Install is one script: install.sh copies configs and python, "
           "then cargo-builds the workspace and installs binaries over "
           "the python twins.\n"
           "A box without cargo still works - it simply keeps the python "
           "fleet. That is the whole portability story, and why the "
           "python twins stay in bin/.")
    d.note(40, 500,
           "Layer discipline: a display-only surface has an EMPTY input "
           "region; an interactive one claims exactly its pixels.\n"
           "A stretch-anchored surface must adopt the configure-granted "
           "size before drawing - a wayland surface IS its buffer "
           "(the 1 px strip trap, v1.3.0).")
    d.write("c4-physical-view.svg")


# ============ Dynamic — wallpaper recolor (the +1 scenario) =========
def recolor():
    d = Diagram("Dynamic — one wallpaper change re-inks everything (+1)",
                1140, 620)
    d.person(40, 96, 180, 66, "santapong", "picks a wallpaper")
    d.component(280, 96, 220, 76, "wallpaper.sh", "bash",
                "the ONE entry point")
    d.external(560, 96, 200, 76, "wallust", "palette",
               "wallpaper -> colours")
    d.store(820, 96, 260, 76, "generated palettes",
            "colors.lua, colors-kitty.conf,\ncolors-*.css, colors.conf", "")
    d.component(280, 260, 220, 76, "rust fleet", "ctl + USR2",
                "cardhost, appdock, office2d, pet, viz")
    d.component(560, 260, 200, 76, "kitty windows", "SIGUSR1",
                "studio + docker + terminals re-ink live")
    d.component(820, 260, 260, 76, "studio tab bar", "--style",
                "tmux re-dressed in place")
    d.component(280, 400, 220, 76, "waybar + swaync", "USR2 / -rs", "")
    d.component(560, 400, 200, 76, "check-contrast.sh", "gate",
                "refuses an unreadable palette")
    d.arrow(224, 130, 276, 130, "runs", ly=122)
    d.arrow(504, 130, 556, 130, "1", ly=122)
    d.arrow(764, 130, 816, 130, "2 writes", ly=122)
    d.arrow(390, 176, 390, 256, "3 reload-theme / USR2", lx=398, ly=215,
            anchor="start")
    d.arrow(660, 176, 660, 256, "4 USR1", lx=668, ly=215, anchor="start")
    d.arrow(950, 176, 950, 256, "5 --style", lx=958, ly=215, anchor="start")
    d.arrow(390, 340, 390, 396, "6", lx=398, ly=368, anchor="start")
    d.arrow(660, 340, 660, 396, "7 verify", lx=668, ly=368, anchor="start")
    d.note(40, 520,
           "Every step is LIVE - nothing restarts except a python pet "
           "(bakes its palette at spawn). The v1.4.0 fix that makes this "
           "true:\nthe pipeline guards match both python and binary "
           "cmdline forms, so the rust fleet actually hears about the "
           "new palette.")
    d.write("c4-dynamic-recolor.svg")


if __name__ == "__main__":
    context()
    containers()
    studio()
    office()
    habitica()
    appdock()
    smartbar()
    process_view()
    development()
    deployment()
    recolor()
    print("  all diagrams regenerated")
