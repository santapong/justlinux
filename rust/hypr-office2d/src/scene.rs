//! The floor, design-handoff edition: a FULLSCREEN 1600×900 scene on the
//! bottom layer. Desk plates at fixed slots, a walking role-sprite worker
//! per live Claude session, a meeting room for multi-agent workflows,
//! ghosts for resumable conversations. The reconcile/animate split and
//! the static/dynamic pass split survive unchanged — they are the CPU
//! budget (<2%, measured law).
//!
//! Ink discipline (handoff): a worker is NEVER accent; status colours
//! only on marks, words, bodies and screens; muted is furniture. The
//! only cross-floor motion is the walk-in; needs-you moves NOTHING.

use std::collections::HashMap;
use std::path::PathBuf;

use hyprdesk::{ClaudeProc, Palette, Rgb};
use tiny_skia::Pixmap;

use crate::sprites::{self, R};
use hyprdesk::draw::Text;

pub const HEADER_H: f32 = 44.0;
pub const STRIP_H: f32 = 44.0;
const PLATE_W: f32 = 224.0;
const PLATE_H: f32 = 192.0;
const SPRITE: f32 = 4.0; // px per sprite pixel
const WALK_PX: f32 = 14.0; // per 160 ms tick — door to desk ≈ 1 s
const GHOST_MIN_IDLE: f64 = 120.0;
const MEET_W: f32 = 300.0;
const MEET_H: f32 = 398.0;

/// Plate origins on the 1600×900 floor (handoff coords × 4/3).
const DESK_SLOTS: [(f32, f32); 6] = [
    (357.0, 104.0),
    (357.0, 371.0),
    (357.0, 627.0),
    (693.0, 104.0),
    (693.0, 371.0),
    (693.0, 627.0),
];

#[derive(Clone, PartialEq)]
pub enum WorkState {
    Working,
    Reading,
    NeedsYou,
    Idle,
    Asleep,
}

#[derive(Clone)]
pub struct SessionRow {
    pub key: String,
    pub pid: i32,
    pub sid: String,
    pub title: String,
    pub cwd: String,
    pub state: WorkState,
    pub subagents: usize,
}

#[derive(Clone)]
pub enum Phase {
    Arriving,
    AtDesk,
    Leaving,
}

pub struct Actor {
    pub desk: usize,
    pub pos: (f32, f32),
    pub path: Vec<(f32, f32)>,
    pub phase: Phase,
    pub row: SessionRow,
}

pub struct Ghost {
    pub desk: usize,
    pub sid: String,
    pub cwd: String,
    pub title: String,
    pub age: String,
}

pub struct Scene {
    pub w: f32,
    pub h: f32,
    pub hover: Option<usize>,
    pub actors: Vec<Actor>,
    pub ghosts: Vec<Ghost>,
    pub meeting: usize,
    pub meeting_title: String,
    pub overflow: usize,
    pub motion: bool, // office_motion=off substitutes stillness, not absence
    frame: u64,
    meta_cache: HashMap<(PathBuf, u64), (String, String)>,
    title_cache: HashMap<(PathBuf, u64), String>,
}

fn door_pos(h: f32) -> (f32, f32) {
    (28.0, h - STRIP_H - 140.0)
}

fn seat_of(desk: usize) -> (f32, f32) {
    let (px, py) = DESK_SLOTS[desk];
    (px + 52.0, py + 40.0)
}

fn path_to_desk(desk: usize, h: f32) -> Vec<(f32, f32)> {
    let (sx, sy) = seat_of(desk);
    let (_, dy) = door_pos(h);
    vec![(180.0, dy), (180.0, sy + 30.0), (sx, sy + 30.0), (sx, sy)]
}

impl Scene {
    pub fn new() -> Scene {
        Scene {
            w: 1600.0,
            h: 900.0,
            hover: None,
            actors: Vec::new(),
            ghosts: Vec::new(),
            meeting: 0,
            meeting_title: String::new(),
            overflow: 0,
            motion: true,
            frame: 0,
            meta_cache: HashMap::new(),
            title_cache: HashMap::new(),
        }
    }

    fn meta(&mut self, mt: f64, p: &PathBuf) -> (String, String) {
        // 30 s buckets: a LIVE transcript changes mtime every poll, and
        // re-reading it each reconcile was ~20 ms of the CPU bill. cwd and
        // preview never change mid-session; the title moves rarely.
        let key = (p.clone(), (mt / 30.0) as u64);
        if let Some(v) = self.meta_cache.get(&key) {
            return v.clone();
        }
        let v = hyprdesk::session_meta(p);
        if self.meta_cache.len() > 256 {
            self.meta_cache.clear();
        }
        self.meta_cache.insert(key, v.clone());
        v
    }

    fn title(&mut self, mt: f64, p: &PathBuf) -> String {
        let key = (p.clone(), (mt / 30.0) as u64);
        if let Some(v) = self.title_cache.get(&key) {
            return v.clone();
        }
        let v = hyprdesk::session_title(p);
        if self.title_cache.len() > 256 {
            self.title_cache.clear();
        }
        self.title_cache.insert(key, v.clone());
        v
    }

    /// sids whose studio window carries a bell — Claude finished its turn
    /// and wants the user. The one honest needs-you signal on this box.
    fn bell_sids() -> Vec<String> {
        let out = std::process::Command::new("tmux")
            .args([
                "-L",
                "claude-studio",
                "list-panes",
                "-s",
                "-F",
                "#{window_bell_flag}|#{pane_start_command}",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        out.lines()
            .filter_map(|l| {
                let (bell, cmd) = l.split_once('|')?;
                if bell != "1" {
                    return None;
                }
                let i = cmd.find("--resume ").map(|i| i + 9)
                    .or_else(|| cmd.find("--session-id ").map(|i| i + 13))?;
                cmd.get(i..i + 36).map(String::from)
            })
            .collect()
    }

    pub fn reconcile(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let tp = std::time::Instant::now();
        let procs: Vec<ClaudeProc> = hyprdesk::claude_procs()
            .into_iter()
            .filter(|p| p.interactive)
            .collect();
        let t_procs = tp.elapsed();
        let tx0 = std::time::Instant::now();
        let txs = hyprdesk::recent_transcripts(30);
        let t_txs = tx0.elapsed();
        let ta = std::time::Instant::now();
        let agents = hyprdesk::active_subagents(20.0);
        let t_agents = ta.elapsed();
        let tb = std::time::Instant::now();
        let bells = Self::bell_sids();
        if std::env::var("OFFICE2D_PROFILE").is_ok() {
            eprintln!("procs {t_procs:?} txs {t_txs:?} agents {t_agents:?} bells {:?}", tb.elapsed());
        }

        let by_sid: HashMap<String, (f64, PathBuf)> = txs
            .iter()
            .map(|(mt, p)| {
                let sid = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                (sid, (*mt, p.clone()))
            })
            .collect();

        let mut rows: Vec<SessionRow> = Vec::new();
        let mut bound: Vec<PathBuf> = Vec::new();
        for p in &procs {
            let (sid, tx) = if !p.argv_sid.is_empty() {
                (
                    p.argv_sid.clone(),
                    by_sid.get(&p.argv_sid).map(|(m, f)| (*m, f.clone())),
                )
            } else {
                let mut found = (String::new(), None);
                let metas: Vec<(f64, PathBuf, String)> = txs
                    .iter()
                    .filter(|(_, f)| !bound.contains(f))
                    .map(|(mt, f)| {
                        let (cwd, _) = self.meta(*mt, f);
                        (*mt, f.clone(), cwd)
                    })
                    .collect();
                for (mt, f, cwd) in metas {
                    if cwd == p.cwd {
                        let sid = f
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        found = (sid, Some((mt, f)));
                        break;
                    }
                }
                found
            };
            let (title, mt) = match &tx {
                Some((mt, f)) => {
                    bound.push(f.clone());
                    (self.title(*mt, f), *mt)
                }
                None => (String::new(), 0.0),
            };
            let base = p.cwd.rsplit('/').next().unwrap_or("~").to_string();
            let age = now - mt;
            // state table (handoff): working <15 s of transcript writes,
            // reading 15–60, idle 60–120, asleep beyond. needs-you comes
            // from the studio bell and OUTRANKS everything but working.
            let mut state = if mt > 0.0 && age < 15.0 {
                WorkState::Working
            } else if mt > 0.0 && age < 60.0 {
                WorkState::Reading
            } else if mt > 0.0 && age < 120.0 {
                WorkState::Idle
            } else {
                WorkState::Asleep
            };
            if bells.contains(&sid) && !matches!(state, WorkState::Working) {
                state = WorkState::NeedsYou;
            }
            let subagents = agents.get(&hyprdesk::sid_key(&sid)).copied().unwrap_or(0);
            rows.push(SessionRow {
                key: if sid.is_empty() { format!("pid:{}", p.pid) } else { sid.clone() },
                pid: p.pid,
                sid,
                title: if title.is_empty() { base } else { title },
                cwd: p.cwd.clone(),
                state,
                subagents,
            });
        }
        self.overflow = rows.len().saturating_sub(DESK_SLOTS.len());
        rows.truncate(DESK_SLOTS.len());

        self.meeting = agents.values().sum();
        // name the workflow by its busiest parent, when that parent is on
        // the floor — real facts only, no invented progress
        self.meeting_title = agents
            .iter()
            .max_by_key(|(_, n)| **n)
            .and_then(|(k, _)| {
                rows.iter()
                    .find(|r| hyprdesk::sid_key(&r.sid) == *k)
                    .map(|r| r.title.clone())
            })
            .unwrap_or_default();

        let desired: HashMap<String, SessionRow> =
            rows.iter().map(|r| (r.key.clone(), r.clone())).collect();
        let h = self.h;
        for a in self.actors.iter_mut() {
            if !desired.contains_key(&a.row.key) && !matches!(a.phase, Phase::Leaving) {
                a.phase = Phase::Leaving;
                a.path = vec![(a.pos.0, door_pos(h).1), door_pos(h)];
            }
        }
        for (key, row) in &desired {
            if let Some(a) = self.actors.iter_mut().find(|a| &a.row.key == key) {
                a.row = row.clone();
                if matches!(a.phase, Phase::Leaving) {
                    a.phase = Phase::Arriving;
                    a.path = path_to_desk(a.desk, h);
                }
                continue;
            }
            let taken: Vec<usize> = self.actors.iter().map(|a| a.desk).collect();
            let Some(desk) = (0..DESK_SLOTS.len()).find(|d| !taken.contains(d)) else {
                continue;
            };
            let (path, pos) = if self.motion {
                (path_to_desk(desk, h), door_pos(h))
            } else {
                // reduced motion: substitute stillness — appear seated
                (Vec::new(), seat_of(desk))
            };
            self.actors.push(Actor {
                desk,
                pos,
                path,
                phase: if self.motion { Phase::Arriving } else { Phase::AtDesk },
                row: row.clone(),
            });
        }

        self.ghosts.clear();
        let taken: Vec<usize> = self.actors.iter().map(|a| a.desk).collect();
        let mut free: Vec<usize> = (0..DESK_SLOTS.len()).filter(|d| !taken.contains(d)).collect();
        free.reverse(); // fill top-down
        let ghost_rows: Vec<(f64, PathBuf)> = txs
            .iter()
            .filter(|(mt, f)| {
                !bound.contains(f)
                    && now - *mt > GHOST_MIN_IDLE
                    && !desired.keys().any(|k| f.to_string_lossy().contains(k))
            })
            .take(4)
            .map(|(mt, f)| (*mt, f.clone()))
            .collect();
        for (mt, f) in ghost_rows {
            let Some(desk) = free.pop() else { break };
            let sid = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let (cwd, preview) = self.meta(mt, &f);
            let title = {
                let t = self.title(mt, &f);
                if t.is_empty() { preview } else { t }
            };
            self.ghosts.push(Ghost {
                desk,
                sid,
                cwd,
                title,
                age: hyprdesk::ago(mt),
            });
        }
    }

    pub fn animate(&mut self) -> (bool, bool) {
        self.frame += 1;
        let mut moved = false;
        let mut static_dirty = false;
        for a in self.actors.iter_mut() {
            let Some(&target) = a.path.first() else {
                if matches!(a.phase, Phase::Arriving) {
                    a.phase = Phase::AtDesk;
                    static_dirty = true;
                    moved = true;
                }
                continue;
            };
            let (dx, dy) = (target.0 - a.pos.0, target.1 - a.pos.1);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= WALK_PX {
                a.pos = target;
                a.path.remove(0);
            } else {
                a.pos.0 += dx / dist * WALK_PX;
                a.pos.1 += dy / dist * WALK_PX;
            }
            moved = true;
        }
        let before = self.actors.len();
        self.actors
            .retain(|a| !(matches!(a.phase, Phase::Leaving) && a.path.is_empty()));
        if self.actors.len() != before {
            moved = true;
            static_dirty = true;
        }
        let animating = self.motion
            && (self.meeting > 0
                || self
                    .actors
                    .iter()
                    .any(|a| matches!(a.row.state, WorkState::Working)));
        (moved || animating, static_dirty)
    }

    // counts for the header (ghosts counted separately — a ghost is not
    // running and must not be reported as such)
    pub fn count(&self, f: impl Fn(&WorkState) -> bool) -> usize {
        self.actors
            .iter()
            .filter(|a| matches!(a.phase, Phase::AtDesk) && f(&a.row.state))
            .count()
    }

    fn plate_rect(&self, di: usize) -> (f32, f32, f32, f32) {
        let (x, y) = DESK_SLOTS[di];
        (x, y, PLATE_W, PLATE_H)
    }

    fn meet_rect(&self) -> (f32, f32, f32, f32) {
        (self.w - MEET_W - 32.0, 93.0, MEET_W, MEET_H)
    }

    /// The interactive pixels — main.rs turns this into the input region
    /// so the desktop outside the plates stays click-through (layer law).
    pub fn interactive_rects(&self) -> Vec<(i32, i32, i32, i32)> {
        let mut out = Vec::new();
        for a in &self.actors {
            let (x, y, w, h) = self.plate_rect(a.desk);
            out.push((x as i32, y as i32, w as i32, h as i32));
        }
        for g in &self.ghosts {
            let (x, y, w, h) = self.plate_rect(g.desk);
            out.push((x as i32, y as i32, w as i32, h as i32));
        }
        if self.meeting > 0 {
            let (x, y, w, h) = self.meet_rect();
            out.push((x as i32, y as i32, w as i32, h as i32));
        }
        out
    }

    /// Where the dynamic pass may touch pixels this frame — the damage
    /// and copy budget. Fullscreen made "swap the whole surface every
    /// tick" a 6% CPU bill; these rects are what actually moves.
    pub fn dynamic_rects(&self) -> Vec<(i32, i32, i32, i32)> {
        let mut out = Vec::new();
        if self.meeting > 0 {
            let (mx, my, mw, mh) = self.meet_rect();
            out.push((mx as i32, my as i32 + 90, mw as i32, (mh as i32 - 90).max(0)));
        }
        for a in &self.actors {
            match a.phase {
                Phase::AtDesk => {
                    if matches!(a.row.state, WorkState::Working) && self.motion {
                        let (x, y, w, h) = self.plate_rect(a.desk);
                        out.push((x as i32 - 2, y as i32 - 2, w as i32 + 4, h as i32 + 4));
                    }
                }
                _ => {
                    // walker bbox with one tick of travel margin
                    out.push((a.pos.0 as i32 - 20, a.pos.1 as i32 - 20, 88, 100));
                }
            }
        }
        out
    }

    pub fn desk_at(&self, x: f32, y: f32) -> Option<usize> {
        DESK_SLOTS.iter().position(|&(dx, dy)| {
            x >= dx && x <= dx + PLATE_W && y >= dy && y <= dy + PLATE_H
        })
    }

    pub fn hit(&self, x: f32, y: f32) -> Click {
        if let Some(di) = self.desk_at(x, y) {
            if let Some(a) = self
                .actors
                .iter()
                .find(|a| a.desk == di && matches!(a.phase, Phase::AtDesk))
            {
                return Click::Session(a.row.clone());
            }
            if let Some(g) = self.ghosts.iter().find(|g| g.desk == di) {
                return Click::Ghost {
                    sid: g.sid.clone(),
                    cwd: g.cwd.clone(),
                };
            }
        }
        let (mx, my, mw, mh) = self.meet_rect();
        if self.meeting > 0 && x >= mx && x <= mx + mw && y >= my && y <= my + mh {
            return Click::Meeting;
        }
        Click::Background
    }

    fn state_bits<'a>(&self, st: &WorkState, pal: &'a Palette) -> (&'static str, Rgb, Rgb, Rgb, &'static str) {
        // (mark, word ink, worker body, monitor screen, word)
        match st {
            WorkState::Working => ("●", pal.good, pal.good, pal.good, "working"),
            WorkState::Reading => ("◌", pal.warn, pal.sub, pal.warn, "reading"),
            WorkState::NeedsYou => ("●", pal.warn, pal.warn, pal.warn, "needs you"),
            WorkState::Idle => ("○", pal.sub, pal.sub, pal.muted, "idle"),
            WorkState::Asleep => ("○", pal.sub, pal.sub, pal.muted, "asleep"),
        }
    }

    pub fn render(&self, pix: &mut Pixmap, pal: &Palette, text: &Text, pass: Pass) {
        let blink = self.motion && self.frame % 2 == 0;
        if pass == Pass::Dynamic {
            self.render_dynamic(pix, pal, text, blink);
            return;
        }
        let (w, h) = (self.w, self.h);
        let fill = |pix: &mut Pixmap, x: f32, y: f32, ww: f32, hh: f32, c: Rgb, a: u8| {
            let mut p = tiny_skia::Paint::default();
            p.set_color(tiny_skia::Color::from_rgba8(c.0, c.1, c.2, a));
            if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, ww, hh) {
                pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
            }
        };
        let dark = |pix: &mut Pixmap, x: f32, y: f32, ww: f32, hh: f32, a: u8| {
            let mut p = tiny_skia::Paint::default();
            p.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, a));
            if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, ww, hh) {
                pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
            }
        };

        // floor lattice: 48 px grid, muted at 22%
        let mut gx = 0.0;
        while gx < w {
            fill(pix, gx, HEADER_H, 1.0, h - HEADER_H, pal.muted, 56);
            gx += 48.0;
        }
        let mut gy = HEADER_H;
        while gy < h {
            fill(pix, 0.0, gy, w, 1.0, pal.muted, 56);
            gy += 48.0;
        }

        // header strip
        dark(pix, 0.0, 0.0, w, HEADER_H, 107); // rgba(0,0,0,.42)
        fill(pix, 0.0, HEADER_H - 1.0, w, 1.0, pal.muted, 255);
        text.draw_weight(pix, 20.0, 28.0, 15.0, pal.accent2, "󰚩 CLAUDE OFFICE", true);
        let needs = self.count(|s| matches!(s, WorkState::NeedsYou));
        let working = self.count(|s| matches!(s, WorkState::Working | WorkState::Reading));
        let asleep = self.count(|s| matches!(s, WorkState::Idle | WorkState::Asleep));
        let mut hx = w - 24.0;
        let mut put_right = |pix: &mut Pixmap, s: &str, ink: Rgb| {
            let adv = text.advance(12.0, false, s);
            hx -= adv;
            text.draw(pix, hx, 27.0, 12.0, ink, s);
            hx -= 22.0;
        };
        if !self.ghosts.is_empty() {
            put_right(pix, &format!("{} to pick up", self.ghosts.len()), pal.sub);
        }
        if asleep > 0 {
            put_right(pix, &format!("{asleep} asleep"), pal.sub);
        }
        if working > 0 {
            put_right(pix, &format!("{working} working"), pal.good);
        }
        if needs > 0 {
            put_right(pix, &format!("{needs} needs you"), pal.warn);
        }

        // door + dashed walk-in path
        let (dx0, dy0) = door_pos(h);
        sprites::blit_role(pix, sprites::DOOR, dx0 - 20.0, dy0 - 44.0, SPRITE, pal, pal.muted, 255);
        text.draw(pix, dx0 - 16.0, dy0 + 60.0, 11.0, pal.sub, "DOOR");
        let mut px = dx0 + 40.0;
        while px < 340.0 {
            fill(pix, px, dy0 + 4.0, 8.0, 2.0, pal.muted, 140);
            px += 16.0;
        }

        // desk plates
        for (di, &(x, y)) in DESK_SLOTS.iter().enumerate() {
            let occupant = self
                .actors
                .iter()
                .find(|a| a.desk == di && matches!(a.phase, Phase::AtDesk));
            let ghost = self.ghosts.iter().find(|g| g.desk == di);
            if occupant.is_none() && ghost.is_none() {
                continue; // bare slots stay bare floor
            }
            // a Working occupant's whole plate belongs to the dynamic pass
            if occupant.is_some_and(|a| matches!(a.row.state, WorkState::Working)) && self.motion {
                continue;
            }
            self.draw_plate(pix, pal, text, di, x, y, occupant, ghost, blink);
        }

        // meeting room
        if self.meeting > 0 {
            let (mx, my, mw, mh) = self.meet_rect();
            dark(pix, mx, my, mw, mh, 56); // rgba(0,0,0,.22)
            for (bx, by, bw, bh) in [
                (mx, my, mw, 1.0),
                (mx, my + mh, mw, 1.0),
                (mx, my, 1.0, mh),
                (mx + mw, my, 1.0, mh + 1.0),
            ] {
                fill(pix, bx, by, bw, bh, pal.muted, 255);
            }
            fill(pix, mx, my + 34.0, mw, 1.0, pal.muted, 255);
            text.draw(pix, mx + 12.0, my + 23.0, 11.0, pal.sub, "MEETING ROOM");
            let run = "● running";
            let adv = text.advance(11.0, false, run);
            text.draw(pix, mx + mw - 12.0 - adv, my + 23.0, 11.0, pal.good, run);
            let name = if self.meeting_title.is_empty() {
                "multi-agent workflow".to_string()
            } else {
                self.meeting_title.chars().take(34).collect()
            };
            text.draw(pix, mx + 16.0, my + 58.0, 13.0, pal.fg, &name);
            text.draw(
                pix,
                mx + 16.0,
                my + 78.0,
                12.0,
                pal.sub,
                &format!(
                    "{} subagent{} · live",
                    self.meeting,
                    if self.meeting == 1 { "" } else { "s" }
                ),
            );
            // minis animate — dynamic pass
        }

        // bottom hover strip: the accent top rule is the ONE accent thing
        if let Some(di) = self.hover {
            let (occ, gho) = (
                self.actors
                    .iter()
                    .find(|a| a.desk == di && matches!(a.phase, Phase::AtDesk)),
                self.ghosts.iter().find(|g| g.desk == di),
            );
            if occ.is_some() || gho.is_some() {
                dark(pix, 0.0, h - STRIP_H, w, STRIP_H, 128); // rgba .5
                fill(pix, 0.0, h - STRIP_H, w, 2.0, pal.accent, 255);
                if let Some(a) = occ {
                    text.draw_weight(pix, 20.0, h - 26.0, 13.0, pal.fg, &a.row.title, true);
                    let cwd = format!("󰉋 {}", a.row.cwd.replace(&hyprdesk::home().display().to_string(), "~"));
                    text.draw(pix, 20.0, h - 9.0, 11.0, pal.sub, &cwd);
                    let (_, ink, _, _, word) = self.state_bits(&a.row.state, pal);
                    let adv = text.advance(12.0, false, word);
                    text.draw(pix, w - 24.0 - adv, h - 18.0, 12.0, ink, word);
                } else if let Some(g) = gho {
                    text.draw_weight(pix, 20.0, h - 26.0, 13.0, pal.fg, &g.title, true);
                    text.draw(
                        pix,
                        20.0,
                        h - 9.0,
                        11.0,
                        pal.sub,
                        &format!("󰥔 {} · click to reopen", g.age),
                    );
                }
            }
        }
        if self.overflow > 0 {
            text.draw(
                pix,
                24.0,
                HEADER_H + 24.0,
                12.0,
                pal.sub,
                &format!("+{} more — open the studio", self.overflow),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_plate(
        &self,
        pix: &mut Pixmap,
        pal: &Palette,
        text: &Text,
        di: usize,
        x: f32,
        y: f32,
        occupant: Option<&Actor>,
        ghost: Option<&Ghost>,
        blink: bool,
    ) {
        let hovered = self.hover == Some(di);
        let is_ghost = occupant.is_none() && ghost.is_some();
        let alpha = if is_ghost {
            if hovered { 217 } else { 115 } // .85 / .45
        } else {
            255
        };
        let a8 = |v: u8| ((v as u32 * alpha as u32) / 255) as u8;
        let fill = |pix: &mut Pixmap, fx: f32, fy: f32, ww: f32, hh: f32, c: Rgb, a: u8| {
            let mut p = tiny_skia::Paint::default();
            p.set_color(tiny_skia::Color::from_rgba8(c.0, c.1, c.2, a));
            if let Some(rc) = tiny_skia::Rect::from_xywh(fx, fy, ww, hh) {
                pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
            }
        };
        let needs = occupant.is_some_and(|a| matches!(a.row.state, WorkState::NeedsYou));
        // plate fill: dark glass, warn-tinted at 10% for needs-you
        if needs {
            fill(pix, x, y, PLATE_W, PLATE_H, pal.warn, 26);
        } else {
            let mut p = tiny_skia::Paint::default();
            p.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, a8(41))); // .16
            if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, PLATE_W, PLATE_H) {
                pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
            }
        }
        // border: accent ONLY while hovered (exactly one desk can be)
        let bc = if hovered { pal.accent } else { pal.muted };
        for (bx, by, bw, bh) in [
            (x, y, PLATE_W, 1.0),
            (x, y + PLATE_H - 1.0, PLATE_W, 1.0),
            (x, y, 1.0, PLATE_H),
            (x + PLATE_W - 1.0, y, 1.0, PLATE_H),
        ] {
            fill(pix, bx, by, bw, bh, bc, a8(255));
        }
        // scene: desk (screen = state), chair, worker, subagent
        let state_rgb = occupant
            .map(|a| self.state_bits(&a.row.state, pal).3)
            .unwrap_or(pal.muted);
        sprites::blit_role(pix, sprites::DESK2, x + 46.0, y + 8.0, SPRITE, pal, state_rgb, a8(255));
        sprites::blit_role(pix, sprites::CHAIR, x + 18.0, y + 52.0, SPRITE, pal, pal.muted, a8(255));
        if let Some(a) = occupant {
            let body = self.state_bits(&a.row.state, pal).2;
            let (wx, art): (f32, &[sprites::SpriteRect]) = match a.row.state {
                WorkState::Asleep => (52.0, sprites::WORKER_ASLEEP),
                WorkState::Working => (60.0, if blink { sprites::WORKER_B } else { sprites::WORKER }),
                _ => (52.0, sprites::WORKER),
            };
            sprites::blit_role(pix, art, x + wx, y + 44.0, SPRITE, pal, body, 255);
            if a.row.subagents > 0 {
                sprites::blit_role(
                    pix,
                    sprites::SUBAGENT,
                    x + 172.0,
                    y + 56.0,
                    3.5,
                    pal,
                    pal.good,
                    255,
                );
                text.draw(
                    pix,
                    x + 172.0 + 30.0,
                    y + 84.0,
                    11.0,
                    pal.sub,
                    &format!("×{}", a.row.subagents),
                );
            }
        }
        // glass label: name over two lines, then mark + state word
        let ly = y + 104.0;
        let mut p = tiny_skia::Paint::default();
        p.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, a8(87))); // .34
        if let Some(rc) = tiny_skia::Rect::from_xywh(x, ly, PLATE_W, PLATE_H - 104.0) {
            pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
        }
        fill(pix, x, ly, PLATE_W, 1.0, pal.muted, a8(255));
        let (name, name_ink, bold) = match (occupant, ghost) {
            (Some(a), _) => (
                a.row.title.clone(),
                if needs { pal.accent2 } else { pal.fg },
                needs,
            ),
            (None, Some(g)) => (g.title.clone(), pal.sub, false),
            _ => (String::new(), pal.sub, false),
        };
        // two lines of ~26 chars at 13 px, wrapped at word boundaries
        let (l1, l2) = wrap2(&name, 26);
        text.draw_weight(pix, x + 10.0, ly + 20.0, 13.0, name_ink, &l1, bold);
        if !l2.is_empty() {
            text.draw_weight(pix, x + 10.0, ly + 36.0, 13.0, name_ink, &l2, bold);
        }
        let state_line = match (occupant, ghost) {
            (Some(a), _) => {
                let (mark, ink, _, _, word) = self.state_bits(&a.row.state, pal);
                (format!("{mark} {word}"), ink)
            }
            (None, Some(g)) => (
                if hovered {
                    "󰥔 click to reopen".to_string()
                } else {
                    format!("󰥔 {}", g.age)
                },
                pal.sub,
            ),
            _ => (String::new(), pal.sub),
        };
        text.draw(pix, x + 10.0, ly + 58.0, 12.0, state_line.1, &state_line.0);
    }

    fn render_dynamic(&self, pix: &mut Pixmap, pal: &Palette, text: &Text, blink: bool) {
        // meeting minis (2-frame shuffle)
        if self.meeting > 0 {
            let (mx, my, ..) = self.meet_rect();
            for i in 0..self.meeting.min(6) {
                let sx = mx + 24.0 + (i % 3) as f32 * 90.0;
                let sy = my + 110.0 + (i / 3) as f32 * 120.0;
                let hop = if self.motion && (self.frame as usize + i) % 2 == 0 { 2.0 } else { 0.0 };
                sprites::blit_role(pix, sprites::SUBAGENT, sx, sy - hop, SPRITE, pal, pal.good, 255);
            }
            if self.meeting > 6 {
                text.draw(
                    pix,
                    mx + 24.0,
                    my + MEET_H - 20.0,
                    12.0,
                    pal.sub,
                    &format!("+{} more", self.meeting - 6),
                );
            }
        }
        // working desks: the full plate redraws here (bob + live screen)
        for (di, &(x, y)) in DESK_SLOTS.iter().enumerate() {
            let Some(a) = self.actors.iter().find(|a| {
                a.desk == di
                    && matches!(a.phase, Phase::AtDesk)
                    && matches!(a.row.state, WorkState::Working)
            }) else {
                continue;
            };
            if !self.motion {
                continue; // static pass drew it seated
            }
            self.draw_plate(pix, pal, text, di, x, y, Some(a), None, blink);
        }
        // walkers above everything
        for a in &self.actors {
            if matches!(a.phase, Phase::AtDesk) {
                continue;
            }
            let art = if blink { sprites::WORKER_B } else { sprites::WORKER };
            sprites::blit_role(pix, art, a.pos.0, a.pos.1, SPRITE, pal, pal.sub, 255);
        }
    }
}

/// Two lines wrapped at word boundaries, each capped at `w` chars.
fn wrap2(name: &str, w: usize) -> (String, String) {
    let mut l1 = String::new();
    let mut l2 = String::new();
    for word in name.split_whitespace() {
        if l2.is_empty() && (l1.is_empty() || l1.chars().count() + 1 + word.chars().count() <= w) {
            if !l1.is_empty() {
                l1.push(' ');
            }
            l1.push_str(word);
        } else {
            if !l2.is_empty() {
                l2.push(' ');
            }
            l2.push_str(word);
        }
    }
    if l2.chars().count() > w {
        l2 = l2.chars().take(w - 1).collect::<String>() + "…";
    }
    // a single unbroken over-long token still hard-splits
    if l1.chars().count() > w {
        let c: Vec<char> = l1.chars().collect();
        let head: String = c.iter().take(w).collect();
        let tail: String = c.iter().skip(w).take(w - 1).collect();
        return (head, if tail.is_empty() { l2 } else { tail + "…" });
    }
    (l1, l2)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Pass {
    Static,
    Dynamic,
}

#[derive(Clone)]
pub enum Click {
    Session(SessionRow),
    Ghost { sid: String, cwd: String },
    Meeting,
    Background,
}
