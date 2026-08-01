//! The floor: desks at x,y, actors that WALK, a meeting room. The core is
//! the reconcile/animate split (docs/rust-migration.md rung 2): reconcile
//! (every ~2 s) decides DESIRED state from the filesystem; animate (every
//! tick) moves actors toward it. Neither ever blocks the other.

use std::collections::HashMap;
use std::path::PathBuf;

use hyprdesk::{ClaudeProc, Palette, Rgb};
use tiny_skia::Pixmap;

use crate::sprites;
use crate::text::Text;

pub const W: u32 = 640;
pub const H: u32 = 300;
const SCALE: f32 = 3.0;
const WALL_X: f32 = 445.0; // meeting room wall
const DOOR: (f32, f32) = (24.0, 258.0);
const CORRIDOR_Y: f32 = 258.0;
const WALK_PX: f32 = 6.0; // per tick (~160ms) — a desk is ~8s from the door
const GHOST_MIN_IDLE: f64 = 120.0;

/// Fixed slots for P1; a "+N more" line owns the overflow. Chosen so a
/// walker's corridor never crosses a desk footprint.
const DESK_SLOTS: [(f32, f32); 6] = [
    (40.0, 64.0),
    (180.0, 64.0),
    (320.0, 64.0),
    (40.0, 168.0),
    (180.0, 168.0),
    (320.0, 168.0),
];

#[derive(Clone, PartialEq)]
pub enum WorkState {
    Typing,
    Idle,
    Sleep,
}

#[derive(Clone)]
pub struct SessionRow {
    pub key: String, // sid when known, else "pid:<n>" — stable per poll
    pub pid: i32,
    pub sid: String,
    pub title: String,
    pub cwd: String,
    pub state: WorkState,
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
    pub path: Vec<(f32, f32)>, // remaining waypoints
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
    pub hover: Option<usize>, // desk index under the pointer
    pub actors: Vec<Actor>,
    pub ghosts: Vec<Ghost>,
    pub meeting: usize, // live subagents in the meeting room
    pub overflow: usize,
    frame: u64,
    meta_cache: HashMap<(PathBuf, u64), (String, String)>,
    title_cache: HashMap<(PathBuf, u64), String>,
}

fn path_to_desk(desk: usize) -> Vec<(f32, f32)> {
    let (dx, dy) = DESK_SLOTS[desk];
    vec![
        (dx + 24.0, CORRIDOR_Y),
        (dx + 24.0, dy + 8.0),
        (dx + 6.0, dy - 13.0), // the worker's seat behind the desk
    ]
}

impl Scene {
    pub fn new() -> Scene {
        Scene {
            hover: None,
            actors: Vec::new(),
            ghosts: Vec::new(),
            meeting: 0,
            overflow: 0,
            frame: 0,
            meta_cache: HashMap::new(),
            title_cache: HashMap::new(),
        }
    }

    fn meta(&mut self, mt: f64, p: &PathBuf) -> (String, String) {
        let key = (p.clone(), mt.to_bits());
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
        let key = (p.clone(), mt.to_bits());
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

    /// The 2 s half: read the world, diff it against the cast, and hand
    /// every change to the animator as an arrival or a departure.
    pub fn reconcile(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let procs: Vec<ClaudeProc> = hyprdesk::claude_procs()
            .into_iter()
            .filter(|p| p.interactive)
            .collect();
        let txs = hyprdesk::recent_transcripts(30);
        let agents = hyprdesk::active_subagents(20.0);

        // sid → transcript for binding and titles
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

        // ---- desired session rows ----
        let mut rows: Vec<SessionRow> = Vec::new();
        let mut bound: Vec<PathBuf> = Vec::new();
        for p in &procs {
            let (sid, tx) = if !p.argv_sid.is_empty() {
                (
                    p.argv_sid.clone(),
                    by_sid.get(&p.argv_sid).map(|(m, f)| (*m, f.clone())),
                )
            } else {
                // no argv fact: newest unclaimed transcript in this cwd
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
                    let t = self.title(*mt, f);
                    (t, *mt)
                }
                None => (String::new(), 0.0),
            };
            let base = p
                .cwd
                .rsplit('/')
                .next()
                .unwrap_or("~")
                .to_string();
            let age = now - mt;
            let state = if mt > 0.0 && age < 15.0 {
                WorkState::Typing
            } else if mt > 0.0 && age > 120.0 {
                WorkState::Sleep
            } else {
                WorkState::Idle
            };
            rows.push(SessionRow {
                key: if sid.is_empty() {
                    format!("pid:{}", p.pid)
                } else {
                    sid.clone()
                },
                pid: p.pid,
                sid,
                title: if title.is_empty() { base } else { title },
                cwd: p.cwd.clone(),
                state,
            });
        }
        self.overflow = rows.len().saturating_sub(DESK_SLOTS.len());
        rows.truncate(DESK_SLOTS.len());

        // ---- meeting room: every live subagent, whoever their parent is ----
        self.meeting = agents.values().sum();

        // ---- diff against the cast ----
        let desired: HashMap<String, SessionRow> =
            rows.iter().map(|r| (r.key.clone(), r.clone())).collect();
        // departures: at a desk (or arriving) but no longer in the world
        for a in self.actors.iter_mut() {
            if !desired.contains_key(&a.row.key) && !matches!(a.phase, Phase::Leaving) {
                a.phase = Phase::Leaving;
                a.path = vec![(a.pos.0, CORRIDOR_Y), DOOR];
            }
        }
        // arrivals + state refresh
        for (key, row) in &desired {
            if let Some(a) = self.actors.iter_mut().find(|a| &a.row.key == key) {
                a.row = row.clone(); // refresh title/state in place
                if matches!(a.phase, Phase::Leaving) {
                    // came back before reaching the door: turn around
                    a.phase = Phase::Arriving;
                    a.path = path_to_desk(a.desk);
                }
                continue;
            }
            let taken: Vec<usize> = self.actors.iter().map(|a| a.desk).collect();
            let Some(desk) = (0..DESK_SLOTS.len()).find(|d| !taken.contains(d)) else {
                continue; // overflow counter already told the truth
            };
            self.actors.push(Actor {
                desk,
                pos: DOOR,
                path: path_to_desk(desk),
                phase: Phase::Arriving,
                row: row.clone(),
            });
        }

        // ---- ghosts fill desks nobody occupies ----
        self.ghosts.clear();
        let taken: Vec<usize> = self.actors.iter().map(|a| a.desk).collect();
        let mut free: Vec<usize> = (0..DESK_SLOTS.len())
            .filter(|d| !taken.contains(d))
            .collect();
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
                if t.is_empty() {
                    preview
                } else {
                    t
                }
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

    /// The 160 ms half: pure motion, no filesystem. Returns whether the
    /// FRAME CHANGED VISIBLY — walkers moved, or something on screen
    /// animates (typing bob/code, meeting minis). A scene of seated idle
    /// workers is a still image, and a still image redrawn six times a
    /// second was the measured 4.5%-vs-2.9% CPU gap against the python.
    pub fn animate(&mut self) -> bool {
        self.frame += 1;
        let mut moved = false;
        for a in self.actors.iter_mut() {
            let Some(&target) = a.path.first() else {
                if matches!(a.phase, Phase::Arriving) {
                    a.phase = Phase::AtDesk;
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
        // a leaver who reached the door despawns
        let before = self.actors.len();
        self.actors
            .retain(|a| !(matches!(a.phase, Phase::Leaving) && a.path.is_empty()));
        moved |= self.actors.len() != before;
        // blink-driven animation only exists on screen for these states
        let animating = self.meeting > 0
            || self
                .actors
                .iter()
                .any(|a| matches!(a.row.state, WorkState::Typing));
        moved || animating
    }

    pub fn working(&self) -> usize {
        self.actors
            .iter()
            .filter(|a| matches!(a.phase, Phase::AtDesk))
            .count()
    }
    pub fn arriving(&self) -> usize {
        self.actors
            .iter()
            .filter(|a| matches!(a.phase, Phase::Arriving))
            .count()
    }

    /// The desk index under (x, y), if any — hover and click share it so
    /// what highlights is exactly what a click would act on.
    pub fn desk_at(&self, x: f32, y: f32) -> Option<usize> {
        DESK_SLOTS.iter().position(|&(dx, dy)| {
            x >= dx - 6.0 && x <= dx + 54.0 && y >= dy - 20.0 && y <= dy + 62.0
        })
    }

    /// What a click at (x, y) means. Desk rects cover art + labels.
    pub fn hit(&self, x: f32, y: f32) -> Click {
        for a in &self.actors {
            let (dx, dy) = DESK_SLOTS[a.desk];
            if x >= dx - 6.0 && x <= dx + 54.0 && y >= dy - 20.0 && y <= dy + 62.0 {
                return Click::Session(a.row.clone());
            }
        }
        for g in &self.ghosts {
            let (dx, dy) = DESK_SLOTS[g.desk];
            if x >= dx - 6.0 && x <= dx + 54.0 && y >= dy - 20.0 && y <= dy + 62.0 {
                return Click::Ghost {
                    sid: g.sid.clone(),
                    cwd: g.cwd.clone(),
                };
            }
        }
        Click::Background
    }

    pub fn render(&self, pix: &mut Pixmap, pal: &Palette, text: &Text) {
        let blink = self.frame % 2 == 0;
        let mut paint = tiny_skia::Paint::default();
        paint.anti_alias = true;

        // glass card
        let r = 14.0;
        let mut pb = tiny_skia::PathBuilder::new();
        let (w, h) = (W as f32, H as f32);
        pb.move_to(r, 0.0);
        pb.line_to(w - r, 0.0);
        pb.quad_to(w, 0.0, w, r);
        pb.line_to(w, h - r);
        pb.quad_to(w, h, w - r, h);
        pb.line_to(r, h);
        pb.quad_to(0.0, h, 0.0, h - r);
        pb.line_to(0.0, r);
        pb.quad_to(0.0, 0.0, r, 0.0);
        pb.close();
        let card = pb.finish().unwrap();
        let Rgb(br, bgc, bb) = pal.bg;
        paint.set_color(tiny_skia::Color::from_rgba8(br, bgc, bb, 204));
        pix.fill_path(
            &card,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );

        // header — the one-line answer to "does Claude need me?"
        let head = if self.meeting > 0 {
            format!(
                "· {} working · {} arriving · workflow ×{} in the meeting room",
                self.working(),
                self.arriving(),
                self.meeting
            )
        } else {
            format!("· {} working · {} arriving", self.working(), self.arriving())
        };
        text.draw(pix, 14.0, 20.0, 11.0, pal.accent, "󰚩 CLAUDE OFFICE ");
        text.draw(pix, 148.0, 20.0, 10.0, pal.sub, &head);
        let line = |pix: &mut Pixmap, x: f32, y: f32, ww: f32, hh: f32, c: Rgb, a: u8| {
            let mut p = tiny_skia::Paint::default();
            p.set_color(tiny_skia::Color::from_rgba8(c.0, c.1, c.2, a));
            if let Some(rc) = tiny_skia::Rect::from_xywh(x, y, ww, hh) {
                pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
            }
        };
        line(pix, 10.0, 30.0, w - 20.0, 1.0, pal.sub, 46);

        // meeting room: wall with a doorway, table, minis
        line(pix, WALL_X, 40.0, 1.0, h - 70.0, pal.sub, 90);
        line(pix, WALL_X, 150.0, 2.0, 44.0, pal.bg, 255); // doorway gap
        text.draw(pix, WALL_X + 18.0, 54.0, 8.0, pal.sub, "MEETING ROOM");
        line(pix, WALL_X + 34.0, 110.0, 110.0, 44.0, pal.muted, 255);
        line(pix, WALL_X + 34.0, 110.0, 110.0, 2.0, pal.sub, 100);
        if self.meeting > 0 {
            text.draw(
                pix,
                WALL_X + 18.0,
                66.0,
                8.0,
                pal.accent,
                &format!("workflow ×{}", self.meeting),
            );
            let seats = [
                (WALL_X + 52.0, 84.0),
                (WALL_X + 104.0, 84.0),
                (WALL_X + 52.0, 158.0),
                (WALL_X + 104.0, 158.0),
                (WALL_X + 20.0, 118.0),
                (WALL_X + 150.0, 118.0),
            ];
            for (i, &(sx, sy)) in seats.iter().enumerate().take(self.meeting.min(6)) {
                let art = if (self.frame as usize + i) % 2 == 0 {
                    sprites::CLAWD_A
                } else {
                    sprites::CLAWD_B
                };
                sprites::blit(pix, art, sx, sy, 2.0, pal, pal.muted, true, 255);
            }
            if self.meeting > 6 {
                text.draw(
                    pix,
                    WALL_X + 150.0,
                    170.0,
                    8.0,
                    pal.accent,
                    &format!("+{}", self.meeting - 6),
                );
            }
        }

        // door
        line(pix, 12.0, h - 38.0, 3.0, 28.0, pal.accent, 230);
        text.draw(pix, 10.0, h - 6.0, 7.0, pal.sub, "DOOR");

        // desks: occupied by an AtDesk actor, ghosted, or bare
        for (di, &(dx, dy)) in DESK_SLOTS.iter().enumerate() {
            if self.hover == Some(di) {
                // hover wash: feedback before the click lands (the office
                // ghosts and the studio tree obey the same rule)
                let mut p = tiny_skia::Paint::default();
                p.set_color(tiny_skia::Color::from_rgba8(
                    pal.sub.0, pal.sub.1, pal.sub.2, 26,
                ));
                if let Some(rc) = tiny_skia::Rect::from_xywh(dx - 6.0, dy - 20.0, 60.0, 82.0) {
                    pix.fill_rect(rc, &p, tiny_skia::Transform::identity(), None);
                }
            }
            let occupant = self
                .actors
                .iter()
                .find(|a| a.desk == di && matches!(a.phase, Phase::AtDesk));
            let ghost = self.ghosts.iter().find(|g| g.desk == di);
            let arriving = self
                .actors
                .iter()
                .find(|a| a.desk == di && !matches!(a.phase, Phase::AtDesk));

            let screen = match occupant.map(|a| &a.row.state) {
                Some(WorkState::Typing) => Rgb(0x2a, 0x24, 0x22),
                Some(_) => pal.muted,
                None => pal.muted,
            };
            let alpha = if ghost.is_some() && occupant.is_none() {
                115
            } else {
                255
            };
            if let Some(a) = occupant {
                match a.row.state {
                    WorkState::Sleep => {
                        sprites::blit(
                            pix,
                            sprites::CLAWD_SLEEP,
                            dx + 12.0,
                            dy - 4.0,
                            SCALE,
                            pal,
                            screen,
                            true,
                            255,
                        );
                    }
                    _ => {
                        let bob = if matches!(a.row.state, WorkState::Typing) && blink {
                            SCALE
                        } else {
                            0.0
                        };
                        let art = if blink { sprites::CLAWD_A } else { sprites::CLAWD_B };
                        sprites::blit(pix, art, dx + 6.0, dy - 13.0 - bob, SCALE, pal, screen, true, 255);
                    }
                }
            }
            sprites::blit(pix, sprites::DESK, dx, dy, SCALE, pal, screen, false, alpha);
            // typing: scrolling code pixels on the dark screen (python parity)
            if let Some(a) = occupant {
                if matches!(a.row.state, WorkState::Typing) {
                    for rr in 0..3u32 {
                        let cx = (self.frame as u32 + rr * 2) % 4;
                        let c = if rr % 2 == 1 { pal.accent2 } else { pal.accent };
                        line(
                            pix,
                            dx + (4 + cx) as f32 * SCALE,
                            dy + rr as f32 * SCALE + SCALE / 2.0,
                            2.0 * SCALE,
                            (SCALE / 2.0).max(1.0),
                            c,
                            255,
                        );
                    }
                }
            }
            // labels
            let ty = dy + 7.0 * SCALE + 12.0;
            if let Some(a) = occupant.or(arriving) {
                let name: String = a.row.title.chars().take(22).collect();
                text.draw(pix, dx, ty, 9.0, pal.fg, &name);
                let (word, ink) = match (&a.phase, &a.row.state) {
                    (Phase::Arriving, _) => ("arriving…", pal.accent),
                    (Phase::Leaving, _) => ("heading out", pal.sub),
                    (_, WorkState::Typing) => ("working", pal.accent),
                    (_, WorkState::Sleep) => ("asleep", pal.sub),
                    (_, WorkState::Idle) => ("idle", pal.sub),
                };
                text.draw(pix, dx, ty + 12.0, 8.0, ink, word);
            } else if let Some(g) = ghost {
                let name: String = g.title.chars().take(22).collect();
                text.draw(pix, dx, ty, 9.0, pal.sub, &name);
                text.draw(
                    pix,
                    dx,
                    ty + 12.0,
                    8.0,
                    pal.sub,
                    &format!("{} · click to reopen", g.age),
                );
            }
        }
        if self.overflow > 0 {
            text.draw(
                pix,
                WALL_X - 80.0,
                h - 10.0,
                9.0,
                pal.sub,
                &format!("+{} more", self.overflow),
            );
        }

        // walkers draw ABOVE desks so they are never hidden mid-corridor
        for a in &self.actors {
            if matches!(a.phase, Phase::AtDesk) {
                continue;
            }
            let art = if blink { sprites::CLAWD_A } else { sprites::CLAWD_B };
            sprites::blit(pix, art, a.pos.0, a.pos.1, SCALE, pal, pal.muted, true, 255);
        }
    }
}

#[derive(Clone)]
pub enum Click {
    Session(SessionRow),
    Ghost { sid: String, cwd: String },
    Background,
}
