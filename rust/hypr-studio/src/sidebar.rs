//! Tab 0 — the session tree, hand-rolled in ratatui. The Textual app in
//! bin/hypr-claude-studio is the spec: everything that sidebar learned —
//! first-click-aims, the q-confirm that names its stake, the 6 s
//! signature-checked reload, cursor/expansion preservation — is contract
//! here, not decoration (docs/claude-studio.md).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::io::Write as _;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use hyprdesk::Row;

fn col(c: hyprdesk::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// a ~12% wash of `ink` over `base` — the hover tone the design wants,
/// visually distinct from the muted cursor block
fn wash(base: hyprdesk::Rgb, ink: hyprdesk::Rgb, t: f32) -> Color {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color::Rgb(m(base.0, ink.0), m(base.1, ink.1), m(base.2, ink.2))
}

// ----- pinned projects (shared format with the python spec) -----

fn pins_file() -> std::path::PathBuf {
    hyprdesk::home().join(".local/state/hyprdesk/studio-pins.json")
}

/// Corrupt or missing file reads as no pins — and must never be
/// overwritten by that fallback (GUI conventions: shared state files).
fn load_pins() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(pins_file()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

fn save_pins(pins: &[String]) {
    let f = pins_file();
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = f.with_extension("json.tmp");
    let body = serde_json::to_string(pins).unwrap_or_else(|_| "[]".into());
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &f); // temp + rename: never half-written
    }
}

/// Which folder-group a project cwd belongs to. /tmp is its own group
/// because test harnesses (pytest, live-sample runs) mint dozens of
/// throwaway projects there and they were drowning the real ones.
fn project_group(cwd: &str) -> usize {
    let p = format!("{}/", cwd);
    if p.starts_with("/tmp/") {
        return G_SCRATCH;
    }
    if p.contains("/company/") {
        return G_COMPANY;
    }
    if p.starts_with(&hyprdesk::home().display().to_string()) {
        return G_HOME;
    }
    G_OTHER
}

const G_PINNED: usize = 0;
const G_HOME: usize = 1;
const G_COMPANY: usize = 2;
const G_OTHER: usize = 3;
const G_SCRATCH: usize = 4;
// glyphs verified in JetBrainsMono NF via fc-list :charset=
const GROUP_TITLES: [&str; 5] =
    ["󰐃 Pinned", "󱂵 Home", "󰉋 Company", "󰉖 Other", "󰪺 Scratch /tmp"];

#[derive(Clone, PartialEq)]
enum NodeKind {
    Section,
    Project,
    Leaf,
    OpenTab, // a studio window; Enter/second-click selects it
}

#[derive(Clone, PartialEq)]
pub struct OpenWin {
    idx: String,
    name: String,
    beside: String,
    active: bool,
    bell: bool,
    activity: bool,
}

pub fn open_windows() -> Vec<OpenWin> {
    super::tmux_out(&[
        "list-windows",
        "-F",
        "#{window_index}|#{window_name}|#{@beside}|#{window_active}|#{window_bell_flag}|#{window_activity_flag}",
    ])
    .lines()
    .filter_map(|l| {
        let p: Vec<&str> = l.splitn(6, '|').collect();
        if p.len() < 6 || p[1] == "sessions" {
            return None; // the tree's own window is furniture
        }
        Some(OpenWin {
            idx: p[0].into(),
            name: p[1].into(),
            beside: p[2].into(),
            active: p[3] == "1",
            bell: p[4] == "1",
            activity: p[5] == "1",
        })
    })
    .collect()
}

#[derive(Clone)]
struct Node {
    key: String, // identity for cursor/expansion restore (label.plain)
    depth: u16,
    kind: NodeKind,
    expanded: bool,
    row: Option<Row>,
    project_cwd: String,
    win: Option<OpenWin>,
}

struct Confirm {
    tabs: usize,
    convos: usize,
    names: Vec<String>,
    focus_quit: bool, // Cancel holds focus by default (spec)
    cancel_rect: Rect,
    quit_rect: Rect,
}

/// Does the file mention `needle`? Searched from the END in 1 MB chunks
/// (with overlap) and capped at 64 MB, so a plan written a moment ago is
/// found in the first chunk of even a 48 MB transcript, and an old plan
/// costs at most one bounded read — this runs only when a plan file
/// appears, never on the 6 s reload.
fn mentions(path: &std::path::Path, needle: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK: u64 = 1 << 20;
    const CAP: u64 = 64 << 20;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let nb = needle.len() as u64;
    let floor = len.saturating_sub(CAP);
    let mut end = len;
    let mut buf = Vec::new();
    while end > floor {
        let start = end.saturating_sub(CHUNK).max(floor);
        let want = (end - start) as usize;
        if f.seek(SeekFrom::Start(start)).is_err() {
            return false;
        }
        buf.clear();
        buf.resize(want, 0);
        if f.read_exact(&mut buf).is_err() {
            return false;
        }
        if buf.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            return true;
        }
        if start == floor {
            break;
        }
        end = start + nb; // overlap so a needle straddling chunks is seen
    }
    false
}

pub struct App {
    pal: hyprdesk::Palette,
    rows: Vec<Row>,
    sig: Vec<(String, String, String, i32, String)>,
    nodes: Vec<Node>,
    cursor: usize,
    scroll: usize,
    hover: Option<usize>,
    collapsed_sections: HashSet<String>,
    expanded_projects: HashSet<String>,
    pins: Vec<String>,
    filter: String,
    filter_mode: bool, // '/' captures the keyboard until Enter/Esc
    confirm: Option<Confirm>,
    notify: Option<(String, Instant, bool)>, // (msg, expires, warning)
    tree_area: Rect,
    header_area: Rect,
    plans: HashMap<String, PathBuf>, // window index -> its plan-mode plan
    plans_dir_seen: Option<std::time::SystemTime>,
    plans_auto_done: HashSet<(String, PathBuf)>,
    dirty: bool,
    exit: bool,
}

impl App {
    fn new() -> App {
        App {
            pal: hyprdesk::colors(),
            rows: Vec::new(),
            sig: Vec::new(),
            nodes: Vec::new(),
            cursor: 0,
            scroll: 0,
            hover: None,
            // scratch/other start folded — that is the whole point of
            // grouping; a click or → reopens them and the choice sticks
            collapsed_sections: [GROUP_TITLES[G_OTHER], GROUP_TITLES[G_SCRATCH]]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            expanded_projects: HashSet::new(),
            pins: load_pins(),
            filter: String::new(),
            filter_mode: false,
            confirm: None,
            notify: None,
            plans: HashMap::new(),
            plans_dir_seen: None,
            plans_auto_done: HashSet::new(),
            tree_area: Rect::default(),
            header_area: Rect::default(),
            dirty: true,
            exit: false,
        }
    }

    fn say(&mut self, msg: &str, warning: bool) {
        self.notify = Some((msg.to_string(), Instant::now() + Duration::from_secs(4), warning));
        self.dirty = true;
    }

    /// python reload_tree: almost every poll finds nothing new, so almost
    /// every poll must do nothing — the signature check carries that.
    fn reload(&mut self, force: bool) {
        let rows = hyprdesk::session_rows();
        self.scan_plans();
        let winsig: String = open_windows()
            .iter()
            .map(|w| {
                format!(
                    "{}|{}|{}{}{}|{}|{};",
                    w.idx, w.name, w.active as u8, w.bell as u8, w.activity as u8, w.beside,
                    self.plans.get(&w.idx).map(|p| p.display().to_string()).unwrap_or_default()
                )
            })
            .collect();
        let mut sig: Vec<(String, String, String, i32, String)> = rows
            .iter()
            .map(|r| (r.kind.clone(), r.label.clone(), r.sid.clone(), r.pid, r.detail.clone()))
            .collect();
        sig.push((winsig, String::new(), String::new(), 0, String::new()));
        if !force && sig == self.sig {
            return;
        }
        self.sig = sig;
        self.rows = rows;
        self.rebuild(true);
    }


    /// Which tab is a plan-mode plan for? `~/.claude/plans/` changes its
    /// mtime when Claude creates a plan file; only then (not every reload)
    /// do we ask tmux for the tabs' session ids and grep each transcript's
    /// tail for the plan's file name — Claude's plan-mode text and the
    /// Write call both carry the path. The viewer itself follows edits.
    fn scan_plans(&mut self) {
        let dir = hyprdesk::home().join(".claude/plans");
        let now = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
        if now.is_none() || now == self.plans_dir_seen {
            return;
        }
        self.plans_dir_seen = now;
        let cutoff = std::time::SystemTime::now() - Duration::from_secs(2 * 86400);
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                    .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                    .filter(|(m, _)| *m >= cutoff)
                    .collect()
            })
            .unwrap_or_default();
        files.sort(); // oldest first: the newest mention wins
        if files.is_empty() {
            return;
        }
        let txs = hyprdesk::recent_transcripts(40);
        let auto = hyprdesk::conf_get("studio_plan_auto", "0") == "1";
        for (idx, sid) in super::window_sids() {
            let tx = hyprdesk::tx_for_sid(&sid, &txs);
            if tx.is_empty() {
                continue;
            }
            for (_, f) in &files {
                let Some(name) = f.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
                if mentions(std::path::Path::new(&tx), &name) {
                    let fresh = self.plans.get(&idx) != Some(f);
                    self.plans.insert(idx.clone(), f.clone());
                    if auto && fresh && !self.plans_auto_done.contains(&(idx.clone(), f.clone())) {
                        self.plans_auto_done.insert((idx.clone(), f.clone()));
                        super::open_plan_pane(&idx, f);
                    }
                }
            }
        }
    }

    fn action_plan(&mut self) {
        let Some(w) = self.nodes.get(self.cursor).and_then(|n| n.win.clone()) else {
            self.say("P works on an open tab — plans belong to tabs", true);
            return;
        };
        match self.plans.get(&w.idx).cloned() {
            Some(f) => super::open_plan_pane(&w.idx, &f),
            None => self.say("no plan-mode plan seen for this tab yet", true),
        }
    }

    /// Flatten rows → visible nodes, keeping the shape the user arranged.
    fn rebuild(&mut self, keep_cursor: bool) {
        let cur_key = if keep_cursor {
            self.nodes.get(self.cursor).map(|n| n.key.clone())
        } else {
            None
        };
        let mut nodes = Vec::new();
        let wins = open_windows();
        if !wins.is_empty() {
            nodes.push(Node {
                key: format!("OPEN  · {}", wins.len()),
                depth: 0,
                kind: NodeKind::Section,
                expanded: true,
                row: None,
                project_cwd: String::new(),
                win: None,
            });
            for w in &wins {
                nodes.push(Node {
                    key: format!("open:{}", w.idx),
                    depth: 1,
                    kind: NodeKind::OpenTab,
                    expanded: false,
                    row: None,
                    project_cwd: String::new(),
                    win: Some(w.clone()),
                });
            }
        }
        let running: Vec<&Row> =
            self.rows.iter().filter(|r| r.kind == "run" || r.kind == "bg").collect();
        if !running.is_empty() {
            let key = format!("RUNNING  · {}", running.len());
            let open = !self.collapsed_sections.contains("RUNNING");
            nodes.push(Node {
                key,
                depth: 0,
                kind: NodeKind::Section,
                expanded: open,
                row: None,
                project_cwd: String::new(),
                win: None,
            });
            if open {
                for r in &running {
                    nodes.push(Node {
                        key: r.label.clone(),
                        depth: 1,
                        kind: NodeKind::Leaf,
                        expanded: false,
                        row: Some((*r).clone()),
                        project_cwd: String::new(),
                        win: None,
                    });
                }
            }
        }
        let mut projects: Vec<(String, Vec<&Row>)> = Vec::new();
        for r in self.rows.iter().filter(|r| r.kind == "past") {
            match projects.iter_mut().find(|(l, _)| *l == r.label) {
                Some((_, v)) => v.push(r),
                None => projects.push((r.label.clone(), vec![r])),
            }
        }
        let filt = self.filter.trim().to_lowercase();
        // transcripts arrive newest-first, so first-appearance order IS
        // most-recent-activity order — the alphabetical sort is gone on
        // purpose: active projects float, stale ones sink
        let push_project =
            |nodes: &mut Vec<Node>, label: &str, items: &[&Row], open: bool| {
                nodes.push(Node {
                    key: label.to_string(),
                    depth: 1,
                    kind: NodeKind::Project,
                    expanded: open,
                    row: None,
                    project_cwd: items[0].cwd.clone(),
                    win: None,
                });
                if open {
                    for it in items {
                        nodes.push(Node {
                            key: it.detail.clone(),
                            depth: 2,
                            kind: NodeKind::Leaf,
                            expanded: false,
                            row: Some((*it).clone()),
                            project_cwd: String::new(),
                            win: None,
                        });
                    }
                }
            };
        if !filt.is_empty() {
            // filtered: one flat MATCHES section, everything force-open.
            // Match the project path AND what happened inside it, so
            // "aegis" and "deploy" are both live queries.
            let mut matched: Vec<(String, Vec<&Row>)> = Vec::new();
            for (label, items) in &projects {
                if label.to_lowercase().contains(&filt) {
                    matched.push((label.clone(), items.clone()));
                    continue;
                }
                let hit: Vec<&Row> = items
                    .iter()
                    .copied()
                    .filter(|it| {
                        it.title.to_lowercase().contains(&filt)
                            || it.detail.to_lowercase().contains(&filt)
                    })
                    .collect();
                if !hit.is_empty() {
                    matched.push((label.clone(), hit));
                }
            }
            nodes.push(Node {
                key: format!("MATCHES  · {}", matched.len()),
                depth: 0,
                kind: NodeKind::Section,
                expanded: true,
                row: None,
                project_cwd: String::new(),
                win: None,
            });
            for (label, items) in &matched {
                push_project(&mut nodes, label, items, true);
            }
        } else {
            // folder groups: /tmp scratch was 2/3 of the flat list — it
            // folds to one collapsed row; pins get their own top section
            let mut groups: [Vec<(String, Vec<&Row>)>; 5] = Default::default();
            for (label, items) in projects {
                let g = if self.pins.contains(&items[0].cwd) {
                    G_PINNED
                } else {
                    project_group(&items[0].cwd)
                };
                groups[g].push((label, items));
            }
            for (g, entries) in groups.iter().enumerate() {
                if entries.is_empty() {
                    continue;
                }
                let title = GROUP_TITLES[g];
                let open = !self.collapsed_sections.contains(title);
                nodes.push(Node {
                    key: format!("{title}  · {}", entries.len()),
                    depth: 0,
                    kind: NodeKind::Section,
                    expanded: open,
                    row: None,
                    project_cwd: String::new(),
                    win: None,
                });
                if open {
                    for (label, items) in entries {
                        let popen = self.expanded_projects.contains(label);
                        push_project(&mut nodes, label, items, popen);
                    }
                }
            }
        }
        self.nodes = nodes;
        if let Some(k) = cur_key {
            if let Some(i) = self.nodes.iter().position(|n| n.key == k) {
                self.cursor = i;
            }
        }
        self.cursor = self.cursor.min(self.nodes.len().saturating_sub(1));
        self.dirty = true;
    }

    fn toggle(&mut self, i: usize) {
        let (kind, key) = (self.nodes[i].kind.clone(), self.nodes[i].key.clone());
        match kind {
            NodeKind::Section => {
                // the id is the title without its live count, so the
                // choice survives the count changing under it
                let name = key.split("  · ").next().unwrap_or(&key).to_string();
                if !self.collapsed_sections.remove(&name) {
                    self.collapsed_sections.insert(name);
                }
            }
            NodeKind::Project => {
                if !self.expanded_projects.remove(&key) {
                    self.expanded_projects.insert(key);
                }
            }
            NodeKind::Leaf | NodeKind::OpenTab => return,
        }
        self.rebuild(true);
    }

    fn current(&self) -> Option<&Row> {
        self.nodes.get(self.cursor).and_then(|n| n.row.as_ref())
    }

    // ----- actions (python parity) -----
    fn open_entry(&mut self, row: Row) {
        match row.kind.as_str() {
            "past" => super::open_session_tab(&row.sid, &row.cwd, &row.agent),
            "run" => {
                if !row.addr.is_empty() {
                    let _ = std::process::Command::new("hyprctl")
                        .args(["dispatch", "focuswindow", &format!("address:{}", row.addr)])
                        .status();
                } else if !self.focus_own_pane(&row.tty) {
                    self.say("Can't find that session's window", true);
                }
            }
            "bg" => self.say(
                "Background session — press x to stop it, or watch it on claude.ai",
                true,
            ),
            _ => {}
        }
    }

    /// A session opened inside the studio has no Hyprland window of its
    /// own — match its tty against our panes and focus that tab instead.
    fn focus_own_pane(&self, tty: &str) -> bool {
        if tty.is_empty() {
            return false;
        }
        for line in
            super::tmux_out(&["list-panes", "-s", "-F", "#{window_index}|#{pane_tty}"]).lines()
        {
            let (idx, ptty) = line.split_once('|').unwrap_or(("", ""));
            if ptty == tty {
                super::tmux(&["select-window", "-t", idx]);
                return true;
            }
        }
        false
    }

    fn action_open(&mut self) {
        if let Some(w) = self.nodes.get(self.cursor).and_then(|n| n.win.clone()) {
            super::tmux(&["select-window", "-t", &format!("{}:{}", super::TMUX_SESSION, w.idx)]);
            return;
        }
        if let Some(row) = self.current().cloned() {
            self.open_entry(row);
        }
    }

    fn action_new(&mut self) {
        let cwd = self
            .current()
            .map(|r| r.cwd.clone())
            .or_else(|| self.nodes.get(self.cursor).map(|n| n.project_cwd.clone()))
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| hyprdesk::home().display().to_string());
        super::new_session_tab(&cwd, "claude");
    }

    /// N: a fresh Codex conversation in the highlighted project.
    fn action_new_codex(&mut self) {
        let cwd = self
            .current()
            .map(|r| r.cwd.clone())
            .or_else(|| self.nodes.get(self.cursor).map(|n| n.project_cwd.clone()))
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| hyprdesk::home().display().to_string());
        super::new_session_tab(&cwd, "codex");
    }

    fn action_beside(&mut self) {
        let Some(row) = self.current().cloned() else {
            self.say("Highlight a conversation to open it beside the one you are reading", false);
            return;
        };
        if row.kind != "past" || row.sid.is_empty() {
            self.say("Highlight a conversation to open it beside the one you are reading", false);
            return;
        }
        if super::open_session_beside(&row.sid, &row.cwd, &row.agent) == "focused" {
            self.say("Already open — focused its tab instead", false);
        }
    }

    fn action_term(&mut self) {
        let cwd = self
            .current()
            .map(|r| r.cwd.clone())
            .or_else(|| self.nodes.get(self.cursor).map(|n| n.project_cwd.clone()))
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| hyprdesk::home().display().to_string());
        super::new_term_tab(&cwd);
    }

    fn action_kill_bg(&mut self) {
        let Some(row) = self.current().cloned() else {
            self.say("Highlight a 󰑮 background job first", false);
            return;
        };
        if row.kind != "bg" || row.pid == 0 {
            self.say("Highlight a 󰑮 background job first", false);
            return;
        }
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe { kill(row.pid, 15) };
        self.say("Stopped — its transcript stays resumable", false);
        // the reload interval picks up the corpse a beat later, like the
        // python's 1 s set_timer
    }

    /// p: toggle a pin on the aimed project (or the project owning the
    /// aimed session). Pins live in their own top section and persist.
    fn action_pin(&mut self) {
        let cwd = self
            .nodes
            .get(self.cursor)
            .and_then(|n| {
                if n.kind == NodeKind::Project {
                    Some(n.project_cwd.clone())
                } else {
                    n.row
                        .as_ref()
                        .filter(|r| r.kind == "past")
                        .map(|r| r.cwd.clone())
                }
            })
            .filter(|c| !c.is_empty());
        let Some(cwd) = cwd else {
            self.say("Aim at a project (or one of its sessions) to pin it", true);
            return;
        };
        if let Some(i) = self.pins.iter().position(|p| *p == cwd) {
            self.pins.remove(i);
            self.say("Unpinned", false);
        } else {
            self.pins.push(cwd);
            self.say("Pinned — it now sits at the top", false);
        }
        save_pins(&self.pins);
        self.rebuild(true);
    }

    fn action_quit(&mut self) {
        let (tabs, convos, names) = super::studio_stake();
        if tabs == 0 {
            // nothing open but the tree: do not confirm an empty room
            super::tmux(&["kill-session", "-t", super::TMUX_SESSION]);
            self.exit = true;
            return;
        }
        self.confirm = Some(Confirm {
            tabs,
            convos,
            names,
            focus_quit: false,
            cancel_rect: Rect::default(),
            quit_rect: Rect::default(),
        });
        self.dirty = true;
    }

    fn confirm_done(&mut self, kill: bool) {
        self.confirm = None;
        self.dirty = true;
        if kill {
            super::tmux(&["kill-session", "-t", super::TMUX_SESSION]);
            self.exit = true;
        }
    }

    // ----- input -----
    fn on_key(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        if self.confirm.is_some() {
            match k.code {
                // q, Esc and Enter all cancel; killing needs Tab+Enter
                KeyCode::Esc | KeyCode::Char('q') => self.confirm_done(false),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    if let Some(c) = &mut self.confirm {
                        c.focus_quit = !c.focus_quit;
                        self.dirty = true;
                    }
                }
                KeyCode::Enter => {
                    let kill = self.confirm.as_ref().is_some_and(|c| c.focus_quit);
                    self.confirm_done(kill);
                }
                _ => {}
            }
            return;
        }
        if self.filter_mode {
            // '/' owns the keyboard until Enter (keep matches, back to
            // the tree) or Esc (clear and restore the grouped view)
            match k.code {
                KeyCode::Esc => {
                    self.filter_mode = false;
                    self.filter.clear();
                    self.rebuild(true);
                }
                KeyCode::Enter => {
                    self.filter_mode = false;
                    // land the cursor on the first match, not a header
                    if let Some(i) =
                        self.nodes.iter().position(|n| n.kind == NodeKind::Project)
                    {
                        self.cursor = i;
                    }
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.rebuild(true);
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.rebuild(true);
                }
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild(true);
                }
            }
            KeyCode::Char('/') => {
                self.filter_mode = true;
                self.dirty = true;
            }
            KeyCode::Char('p') => self.action_pin(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                self.dirty = true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.nodes.len() {
                    self.cursor += 1;
                }
                self.dirty = true;
            }
            KeyCode::Right => {
                if self.nodes.get(self.cursor).is_some_and(|n| {
                    n.kind != NodeKind::Leaf && !n.expanded
                }) {
                    self.toggle(self.cursor);
                }
            }
            KeyCode::Left => {
                if self.nodes.get(self.cursor).is_some_and(|n| {
                    n.kind != NodeKind::Leaf && n.expanded
                }) {
                    self.toggle(self.cursor);
                }
            }
            KeyCode::Enter => {
                let kind = self.nodes.get(self.cursor).map(|n| n.kind.clone());
                match kind {
                    Some(NodeKind::Section) | Some(NodeKind::Project) => self.toggle(self.cursor),
                    _ => self.action_open(),
                }
            }
            KeyCode::Char('n') => self.action_new(),
            KeyCode::Char('N') => self.action_new_codex(),
            KeyCode::Char('P') => self.action_plan(),
            KeyCode::Char('s') => self.action_beside(),
            KeyCode::Char('t') => self.action_term(),
            KeyCode::Char('m') => super::open_settings_tab("claude"),
            KeyCode::Char('x') => self.action_kill_bg(),
            KeyCode::Char('r') => self.reload(true),
            KeyCode::Char('w') => self.toggle_width(),
            KeyCode::Char('q') => self.action_quit(),
            _ => {}
        }
    }

    fn node_at(&self, mx: u16, my: u16) -> Option<usize> {
        let a = self.tree_area;
        if mx < a.x || mx >= a.x + a.width || my < a.y || my >= a.y + a.height {
            return None;
        }
        let idx = (my - a.y) as usize + self.scroll;
        if idx < self.nodes.len() {
            Some(idx)
        } else {
            None
        }
    }

    fn toggle_width(&mut self) {
        let pane = std::env::var("TMUX_PANE").unwrap_or_default();
        if !pane.is_empty() {
            let cur = super::tmux_out(&["display-message", "-p", "-t", &pane, "#{pane_width}"])
                .trim()
                .parse::<u32>()
                .unwrap_or(34);
            let next = if cur <= 40 { "56" } else { "34" };
            super::tmux(&["resize-pane", "-t", &pane, "-x", next]);
            self.dirty = true;
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        // the ⟷ button lives in the header row's last cells
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            let h = self.header_area;
            if m.row == h.y && m.column >= h.x + h.width.saturating_sub(4) {
                self.toggle_width();
                return;
            }
        }
        if let Some(c) = &self.confirm {
            if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                let hit = |r: Rect| {
                    m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
                };
                if hit(c.cancel_rect) {
                    self.confirm_done(false);
                } else if hit(c.quit_rect) {
                    self.confirm_done(true);
                }
            }
            return;
        }
        match m.kind {
            MouseEventKind::Moved => {
                let h = self.node_at(m.column, m.row);
                if h != self.hover {
                    self.hover = h;
                    self.dirty = true;
                }
            }
            MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(2);
                self.dirty = true;
            }
            MouseEventKind::ScrollDown => {
                let max = self.nodes.len().saturating_sub(self.tree_area.height as usize);
                self.scroll = (self.scroll + 2).min(max);
                self.dirty = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(i) = self.node_at(m.column, m.row) else { return };
                if i == self.cursor {
                    // second click on the aimed row: commit (spec)
                    match self.nodes[i].kind {
                        NodeKind::Leaf | NodeKind::OpenTab => self.action_open(),
                        _ => self.toggle(i),
                    }
                    return;
                }
                // first click: aim only — and expandables still toggle,
                // because a mistoggle is free (spec)
                self.cursor = i;
                if matches!(self.nodes[i].kind, NodeKind::Section | NodeKind::Project) {
                    self.toggle(i);
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    // ----- drawing -----
    fn draw(&mut self, f: &mut Frame) {
        let pal = &self.pal;
        let narrow = f.area().width < 60;
        let show_filter = self.filter_mode || !self.filter.is_empty();
        let mut constraints = vec![Constraint::Length(1)];
        if show_filter {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1));
        constraints.push(Constraint::Length(if narrow { 4 } else { 1 }));
        let chunks = Layout::vertical(constraints).split(f.area());
        let ti = if show_filter { 2 } else { 1 }; // tree chunk index
        // hint row: the MOUSE contract — the one thing the footer's keys
        // cannot teach (spec)
        self.header_area = chunks[0];
        let left = "󰚩 Sessions  ";
        let mid = if narrow { "C-b g jumps" } else { "click picks · click again opens · C-b g jumps" };
        let used = left.chars().count() + mid.chars().count();
        let pad = (chunks[0].width as usize).saturating_sub(used + 4).max(1);
        let hint = Line::from(vec![
            Span::styled(
                left.to_string(),
                Style::default().fg(col(pal.accent)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(mid.to_string(), Style::default().fg(col(pal.sub))),
            Span::raw(" ".repeat(pad)),
            // the expand button: click toggles 34 ↔ 56 (w does the same;
            // the pane border also drags — tmux mouse is on)
            Span::styled(
                " ⟷ ",
                Style::default().fg(col(pal.fg)).bg(col(pal.muted)),
            ),
        ]);
        f.render_widget(Paragraph::new(hint), chunks[0]);

        if show_filter {
            // live filter readout; the block cursor says typing goes here
            let caret = if self.filter_mode { "▌" } else { "" };
            let tail = if self.filter_mode { "  ↵ tree · Esc clears" } else { "  Esc clears" };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " / ",
                        Style::default()
                            .fg(col(pal.bg))
                            .bg(col(pal.accent))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {}{caret}", self.filter),
                        Style::default().fg(col(pal.fg)),
                    ),
                    Span::styled(tail.to_string(), Style::default().fg(col(pal.sub))),
                ])),
                chunks[1],
            );
        }

        self.tree_area = chunks[ti];
        let h = chunks[ti].height as usize;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + h {
            self.scroll = self.cursor + 1 - h;
        }
        let mut lines: Vec<Line> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate().skip(self.scroll).take(h) {
            let mut spans: Vec<Span> = vec![Span::raw(" ".repeat(n.depth as usize))];
            match n.kind {
                NodeKind::Section => {
                    // headers are fg, their counts sub — accent2 on every
                    // group header would smear accent across the screen
                    let (head, count) = n.key.split_once("  · ").unwrap_or((n.key.as_str(), ""));
                    spans.push(Span::styled(
                        head.to_string(),
                        Style::default().fg(col(pal.fg)).add_modifier(Modifier::BOLD),
                    ));
                    if !count.is_empty() {
                        spans.push(Span::styled(
                            format!("  · {count}"),
                            Style::default().fg(col(pal.sub)),
                        ));
                    }
                }
                NodeKind::OpenTab => {
                    let w = n.win.as_ref().unwrap();
                    spans.push(Span::styled(
                        if w.active { "▸" } else { " " }.to_string(),
                        Style::default().fg(col(pal.sub)),
                    ));
                    spans.push(Span::styled(
                        format!("{}  ", w.idx),
                        Style::default().fg(col(pal.sub)),
                    ));
                    spans.push(Span::styled(
                        w.name.clone(),
                        Style::default().fg(col(pal.fg)),
                    ));
                    if !w.beside.is_empty() {
                        spans.push(Span::styled(
                            format!("  +{}", w.beside),
                            Style::default().fg(col(pal.sub)),
                        ));
                    }
                    if self.plans.contains_key(&w.idx) {
                        spans.push(Span::styled("  󰈙", Style::default().fg(col(pal.accent2))));
                    }
                    if !w.active {
                        if w.bell {
                            spans.push(Span::styled("  ●", Style::default().fg(col(pal.good))));
                        } else if w.activity {
                            spans.push(Span::styled("  ○", Style::default().fg(col(pal.sub))));
                        }
                    }
                }
                NodeKind::Project => {
                    spans.push(Span::styled(
                        if n.expanded { "▾" } else { "▸" }.to_string(),
                        Style::default().fg(col(pal.sub)),
                    ));
                    let glyph = if self.pins.contains(&n.project_cwd) {
                        "󰐃  " // pinned wears its own mark wherever it shows
                    } else {
                        "󰉋  "
                    };
                    spans.push(Span::styled(glyph.to_string(), Style::default().fg(col(pal.sub))));
                    let count = self
                        .rows
                        .iter()
                        .filter(|r| r.kind == "past" && r.label == n.key)
                        .count();
                    spans.push(Span::styled(n.key.clone(), Style::default().fg(col(pal.fg))));
                    spans.push(Span::styled(
                        format!("  {count}"),
                        Style::default().fg(col(pal.sub)),
                    ));
                }
                NodeKind::Leaf => {
                    let r = n.row.as_ref().unwrap();
                    match r.kind.as_str() {
                        "run" => {
                            spans.push(Span::styled("●  ", Style::default().fg(col(pal.good))));
                            if r.agent == "codex" {
                                spans.push(Span::styled(
                                    format!("{} ", super::CODEX_GLYPH),
                                    Style::default().fg(col(pal.sub)),
                                ));
                            }
                            spans.push(Span::styled(r.label.clone(), Style::default().fg(col(pal.fg))));
                            if !r.dir.is_empty() {
                                spans.push(Span::styled(
                                    format!("  {}", r.dir),
                                    Style::default().fg(col(pal.sub)),
                                ));
                            }
                        }
                        "bg" => {
                            spans.push(Span::styled("󰑮  ", Style::default().fg(col(pal.warn))));
                            spans.push(Span::styled(r.label.clone(), Style::default().fg(col(pal.fg))));
                        }
                        _ => {
                            // 󰥔 age   title (two-ink: age sub, name fg)
                            let age = r.detail.split(" — ").next().unwrap_or("").replace(" ago", "");
                            let title = if r.title.is_empty() {
                                r.detail.split(" — ").nth(1).unwrap_or(&r.detail).to_string()
                            } else {
                                r.title.clone()
                            };
                            spans.push(Span::styled(
                                format!("󰥔 {age:<4} "),
                                Style::default().fg(col(pal.sub)),
                            ));
                            if r.agent == "codex" {
                                spans.push(Span::styled(
                                    format!("{} ", super::CODEX_GLYPH),
                                    Style::default().fg(col(pal.sub)),
                                ));
                            }
                            spans.push(Span::styled(title, Style::default().fg(col(pal.fg))));
                        }
                    }
                }
            }
            let mut line = Line::from(spans);
            if i == self.cursor {
                // this pane holds focus: the cursor wears accent
                line = line.style(
                    Style::default()
                        .bg(col(pal.accent))
                        .fg(col(pal.bg))
                        .add_modifier(Modifier::BOLD),
                );
            } else if Some(i) == self.hover {
                // hover is a wash, never the cursor block (design)
                line = line.style(Style::default().bg(wash(pal.bg, pal.sub, 0.12)));
            }
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), chunks[ti]);

        // footer: the keys, in sub ink. In the 34-cell pane the handoff
        // gives it a rule and two rows.
        if narrow {
            let key = |k: &str, v: &str| {
                vec![
                    Span::styled(
                        format!(" {k} "),
                        Style::default().fg(col(pal.accent2)).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(v.to_string(), Style::default().fg(col(pal.sub))),
                ]
            };
            let mut lines = vec![Line::from(Span::styled(
                "─".repeat(f.area().width as usize),
                Style::default().fg(col(pal.muted)),
            ))];
            if let Some((msg, _, warn)) = &self.notify {
                lines.push(Line::from(Span::styled(
                    format!(" {msg}"),
                    Style::default().fg(if *warn { col(pal.warn) } else { col(pal.good) }),
                )));
                lines.push(Line::from(""));
            } else {
                lines.push(Line::from(
                    [key("↵", "open"), key("s", "beside"), key("n", "new"), key("N", "codex")].concat(),
                ));
                lines.push(Line::from(
                    [key("t", "term"), key("w", "wide"), key("q", "quit")].concat(),
                ));
                lines.push(Line::from(
                    [key("p", "pin"), key("/", "filter")].concat(),
                ));
            }
            f.render_widget(Paragraph::new(lines), chunks[ti + 1]);
            if self.confirm.is_some() {
                self.draw_confirm(f);
            }
            return;
        }
        let footer = if let Some((msg, _, warn)) = &self.notify {
            Line::from(Span::styled(
                format!(" {msg}"),
                Style::default().fg(if *warn { col(pal.warn) } else { col(pal.good) }),
            ))
        } else {
            Line::from(
                [
                    ("↵", "open"),
                    ("s", "beside"),
                    ("n", "new"),
                    ("N", "codex"),
                    ("t", "term"),
                    ("m", "mcp"),
                    ("p", "pin"),
                    ("/", "filter"),
                    ("x", "stop"),
                    ("r", "refresh"),
                    ("q", "quit"),
                ]
                .iter()
                .flat_map(|(k, v)| {
                    vec![
                        Span::styled(
                            format!(" {k} "),
                            Style::default().fg(col(pal.accent2)).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(v.to_string(), Style::default().fg(col(pal.sub))),
                    ]
                })
                .collect::<Vec<_>>(),
            )
        };
        f.render_widget(Paragraph::new(footer), chunks[ti + 1]);

        if self.confirm.is_some() {
            self.draw_confirm(f);
        }
    }

    fn draw_confirm(&mut self, f: &mut Frame) {
        let pal = &self.pal;
        let c = self.confirm.as_ref().unwrap();
        let area = f.area();
        let w = 58.min(area.width.saturating_sub(2));
        let needs_note = c.convos > 0;
        let hgt = if needs_note { 9 } else { 8 };
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(hgt)) / 2;
        let boxr = Rect::new(x, y, w, hgt.min(area.height));
        f.render_widget(Clear, boxr);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col(pal.muted)))
            .style(Style::default().bg(col(pal.bg)));
        f.render_widget(block, boxr);

        let inner = Rect::new(boxr.x + 2, boxr.y + 1, boxr.width.saturating_sub(4), boxr.height - 2);
        let mut shown = c.names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        if c.names.len() > 3 {
            shown += &format!(", +{} more", c.names.len() - 3);
        }
        let what = if c.tabs == 1 {
            let mut w = "1 tab".to_string();
            if c.convos > 0 {
                w += ", a live conversation";
            }
            w
        } else {
            let mut w = format!("{} tabs", c.tabs);
            if c.convos == 1 {
                w += ", one of them a live conversation";
            } else if c.convos > 0 {
                w += &format!(", {} of them live conversations", c.convos);
            }
            w
        };
        let mut lines = vec![
            Line::from(Span::styled(
                "Close the studio?",
                Style::default().fg(col(pal.fg)).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(format!("This closes {what}."), Style::default().fg(col(pal.fg)))),
            Line::from(Span::styled(shown, Style::default().fg(col(pal.sub)))),
        ];
        if needs_note {
            lines.push(Line::from(Span::styled(
                "Every conversation stays resumable — only its terminal goes away.",
                Style::default().fg(col(pal.sub)),
            )));
        }
        f.render_widget(Paragraph::new(lines), inner);

        // buttons, right-aligned; the destructive one wears bad ink only
        // when focused, so the dialog never reads as an error already made
        let cancel = " Cancel ";
        let quitb = " Close studio ";
        let bw = (cancel.len() + quitb.len() + 2) as u16;
        let bx = inner.x + inner.width.saturating_sub(bw);
        let by = inner.y + inner.height - 1;
        let cancel_rect = Rect::new(bx, by, cancel.len() as u16, 1);
        let quit_rect = Rect::new(bx + cancel.len() as u16 + 2, by, quitb.len() as u16, 1);
        let (cs, qs) = if c.focus_quit {
            (
                Style::default().fg(col(pal.fg)).bg(col(pal.muted)),
                Style::default().fg(col(pal.bg)).bg(col(pal.bad)).add_modifier(Modifier::BOLD),
            )
        } else {
            (
                Style::default().fg(col(pal.bg)).bg(col(pal.accent)).add_modifier(Modifier::BOLD),
                Style::default().fg(col(pal.fg)).bg(col(pal.muted)),
            )
        };
        f.render_widget(Paragraph::new(Span::styled(cancel, cs)), cancel_rect);
        f.render_widget(Paragraph::new(Span::styled(quitb, qs)), quit_rect);
        let c = self.confirm.as_mut().unwrap();
        c.cancel_rect = cancel_rect;
        c.quit_rect = quit_rect;
    }
}

pub fn run() {
    // --attached instances are spawned per window by tree_attach; the
    // launch instance already styled the server — do not restyle N times
    let attached = std::env::args().any(|a| a == "--attached");
    if !attached {
        super::style_tmux();
        super::rename_open_tabs();
    }
    let my_pane = std::env::var("TMUX_PANE").unwrap_or_default();
    // one fork at birth, then "am I on screen?" is a file read: the hooks
    // write the active window index (see main::note_active); tmux is only
    // asked again every 20 s to reconcile a hook that never fired
    let my_idx = if my_pane.is_empty() {
        String::new()
    } else {
        super::tmux_out(&["display-message", "-p", "-t", &my_pane, "#{window_index}"]).trim().to_string()
    };
    let active_file = super::active_file();
    let window_active_tmux = |pane: &str| -> bool {
        pane.is_empty()
            || super::tmux_out(&["display-message", "-p", "-t", pane, "#{window_active}"])
                .trim()
                == "1"
    };
    let mut file_seen: Option<std::time::SystemTime> = None;

    let mut stdout = std::io::stdout();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    );
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let Ok(mut terminal) = ratatui::Terminal::new(backend) else { return };

    let mut app = App::new();
    app.reload(true);
    let mut last_reload = Instant::now();
    let mut last_rename = Instant::now();
    let mut last_active_check = Instant::now();
    let mut last_reconcile = Instant::now();
    let mut active = true;

    loop {
        if app.dirty {
            let _ = terminal.draw(|f| app.draw(f));
            app.dirty = false;
        }
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) => app.on_key(k),
                Ok(Event::Mouse(m)) => app.on_mouse(m),
                Ok(Event::Resize(..)) => app.dirty = true,
                _ => {}
            }
        }
        let now = Instant::now();
        // only the VISIBLE tree polls the world — with one instance per
        // window, N instances all polling would multiply the cost
        if now.duration_since(last_active_check) >= Duration::from_millis(500) {
            last_active_check = now;
            let was = active;
            let mtime = std::fs::metadata(&active_file).and_then(|m| m.modified()).ok();
            let reconcile = now.duration_since(last_reconcile) >= Duration::from_secs(20);
            if my_idx.is_empty() || mtime.is_none() || reconcile {
                last_reconcile = now;
                active = window_active_tmux(&my_pane);
            } else if mtime != file_seen {
                file_seen = mtime;
                active = std::fs::read_to_string(&active_file)
                    .map(|s| s.trim() == my_idx)
                    .unwrap_or(active);
            }
            if active && !was {
                app.reload(false); // catch up the moment we come on screen
                last_reload = now;
            }
        }
        if active && now.duration_since(last_reload) >= Duration::from_secs(6) {
            last_reload = now;
            app.reload(false);
        }
        // Claude names a conversation a little after it starts (30 s);
        // one renamer is plenty — the active instance owns the pass
        if active && now.duration_since(last_rename) >= Duration::from_secs(30) {
            last_rename = now;
            super::rename_open_tabs();
        }
        if app.notify.as_ref().is_some_and(|(_, t, _)| now >= *t) {
            app.notify = None;
            app.dirty = true;
        }
        if app.exit {
            break;
        }
    }
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(
        stdout,
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    );
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = stdout.flush();
}
