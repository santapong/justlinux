//! hypr-docker — Docker + Kubernetes control for this desktop,
//! docker-desktop-shaped: compose projects as expandable groups with
//! up/stop/restart/down and merged logs, containers (start/stop/restart/
//! remove/connect), per-container logs following live, images + pull,
//! and a Kubernetes pane (pods across namespaces, logs, exec, delete,
//! context switching) that degrades to a message — never a hang — when
//! the cluster is down. A ratatui terminal panel like the studio
//! sidebar; terminal panels get the palette, never the pixel icons.
//!
//! Ink discipline: ● good = running, ○ sub = stopped, ◌ warn =
//! transitional — status colors never decorate non-status content.

mod docker;
mod kube;

use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

const KITTY_CLASS: &str = "hyprdocker";
const POLL: Duration = Duration::from_secs(3);

fn col(c: hyprdesk::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn hypr_json(args: &[&str]) -> serde_json::Value {
    Command::new("hyprctl")
        .args(args)
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Focus-or-spawn, summoning to the current workspace (studio UX).
fn launch() {
    if let Some(clients) = hypr_json(&["clients", "-j"]).as_array() {
        for c in clients {
            if c.get("class").and_then(|v| v.as_str()) == Some(KITTY_CLASS) {
                let addr = format!(
                    "address:{}",
                    c.get("address").and_then(|v| v.as_str()).unwrap_or("")
                );
                let ws = hypr_json(&["-j", "activeworkspace"])
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "movetoworkspace", &format!("{ws},{addr}")])
                    .status();
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "focuswindow", &addr])
                    .status();
                return;
            }
        }
    }
    let me = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "hypr-docker".into());
    let _ = Command::new("setsid")
        .args([
            "kitty",
            "--class",
            KITTY_CLASS,
            "--title",
            "Docker",
            "-e",
            &me,
            "--tui",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Waybar feeder: one JSON line for the bar's custom/docker module.
fn waybar_status() {
    let cs = docker::ps_all();
    if cs.is_empty() && docker::daemon_version().is_empty() {
        println!(
            "{{\"text\":\"󰡨\",\"tooltip\":\"docker daemon unreachable\",\"class\":\"down\"}}"
        );
        return;
    }
    let up = cs.iter().filter(|c| c.state == "running").count();
    let class = if up > 0 { "on" } else { "off" };
    println!(
        "{{\"text\":\"󰡨 {up}\",\"tooltip\":\"{up} running / {} containers — click for the panel\",\"class\":\"{class}\"}}",
        cs.len()
    );
}

#[derive(PartialEq, Clone, Copy)]
enum Pane {
    Docker,
    Kube,
}

#[derive(Clone, PartialEq)]
enum RowRef {
    Header(&'static str), // COMPOSE / STANDALONE / IMAGES
    Proj(usize),
    Cont(usize),
    Repo(usize),          // image repository group
    Img(usize, usize),    // (repo group, member index into images)
    Blank,
}

struct Project {
    name: String,
    dir: String,
    members: Vec<usize>, // indexes into containers
    running: usize,
    expanded: bool,
    exited_bad: usize, // members with a nonzero exit code
}

struct Repo {
    key: String,
    members: Vec<usize>, // indexes into images
    bytes: f64,
    expanded: bool,
}

enum ConfirmAction {
    Docker(&'static str, String), // rm/rmi + target
    ComposeDown(String, String),  // dir, project
    PodDelete(String, String),    // ns, pod
}

enum Modal {
    None,
    Confirm(ConfirmAction, String, bool), // display name, armed
    Pull(String),
}

struct App {
    pal: hyprdesk::Palette,
    version: String,
    containers: Vec<docker::Container>,
    images: Vec<docker::Image>,
    projects: Vec<Project>,
    repos: Vec<Repo>,
    collapsed: HashSet<String>,      // project names the user folded
    expanded_repos: HashSet<String>, // image repos the user opened (default folded)
    filter: String,                  // live substring filter ("" = off)
    filter_input: bool,              // typing in the footer filter
    paused_at: usize,                // ring length when follow was paused
    rows: Vec<RowRef>,
    sel: usize,
    pane: Pane,
    kube_on: bool,
    kube_ctx: String,
    pods: Vec<kube::Pod>,
    kube_err: String,
    sel_pod: usize,
    logs: docker::LogSink,
    logs_for: String, // container id the pane follows ("" = none)
    logs_title: String,
    log_scroll: usize,
    modal: Modal,
    list_area: Rect,
    log_area: Rect,
    dirty: bool,
    exit: bool,
}

impl App {
    fn refresh(&mut self) {
        let mut cs = docker::ps_all();
        let st = docker::stats();
        for c in cs.iter_mut() {
            if let Some((cpu, mem)) = st.get(&c.id) {
                c.cpu = cpu.clone();
                c.mem = mem.clone();
            }
        }
        let is = docker::images();
        if cs != self.containers || is != self.images {
            self.containers = cs;
            self.images = is;
            self.regroup();
        }
        if self.pane == Pane::Kube && self.kube_on {
            let (pods, err) = kube::pods();
            if pods != self.pods || err != self.kube_err {
                self.pods = pods;
                self.kube_err = err;
                self.sel_pod = self.sel_pod.min(self.pods.len().saturating_sub(1));
                self.dirty = true;
            }
        }
    }

    /// containers → compose groups + standalone, then the flat row list.
    fn regroup(&mut self) {
        let mut projects: Vec<Project> = Vec::new();
        for (i, c) in self.containers.iter().enumerate() {
            if c.project.is_empty() {
                continue;
            }
            match projects.iter_mut().find(|p| p.name == c.project) {
                Some(p) => {
                    p.members.push(i);
                    if c.state == "running" {
                        p.running += 1;
                    }
                    if c.status.contains("Exited") && !c.status.contains("Exited (0)") {
                        p.exited_bad += 1;
                    }
                    if p.dir.is_empty() {
                        p.dir = c.compose_file.clone();
                    }
                }
                None => projects.push(Project {
                    name: c.project.clone(),
                    dir: c.compose_file.clone(),
                    members: vec![i],
                    running: (c.state == "running") as usize,
                    expanded: true,
                    exited_bad: (c.status.contains("Exited")
                        && !c.status.contains("Exited (0)")) as usize,
                }),
            }
        }
        projects.sort_by(|a, b| a.name.cmp(&b.name));
        for p in projects.iter_mut() {
            p.expanded = !self.collapsed.contains(&p.name);
        }
        self.projects = projects;
        // image repositories: registry path -> first segment; local names ->
        // the hyphen prefix when >=2 share it, else the whole repo
        let mut repos: Vec<Repo> = Vec::new();
        let key_of = |repo: &str| -> String {
            if let Some((head, _)) = repo.split_once('/') {
                head.to_string()
            } else if let Some((head, _)) = repo.split_once('-') {
                head.to_string()
            } else {
                repo.to_string()
            }
        };
        // only group under a shared prefix when it is actually shared
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for im in &self.images {
            *counts.entry(key_of(&im.repo)).or_insert(0) += 1;
        }
        for (i, im) in self.images.iter().enumerate() {
            let k = key_of(&im.repo);
            let key = if counts.get(&k).copied().unwrap_or(0) >= 2 { k } else { im.repo.clone() };
            match repos.iter_mut().find(|r| r.key == key) {
                Some(r) => {
                    r.members.push(i);
                    r.bytes += docker::size_bytes(&im.size);
                }
                None => repos.push(Repo {
                    key,
                    members: vec![i],
                    bytes: docker::size_bytes(&im.size),
                    expanded: false,
                }),
            }
        }
        repos.sort_by(|a, b| b.bytes.partial_cmp(&a.bytes).unwrap_or(std::cmp::Ordering::Equal));
        for r in repos.iter_mut() {
            r.expanded = self.expanded_repos.contains(&r.key);
        }
        self.repos = repos;
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let sel_key = self.rows.get(self.sel).cloned();
        let f = self.filter.to_lowercase();
        let hit = |t: &str| f.is_empty() || t.to_lowercase().contains(&f);
        let mut rows = Vec::new();
        let mut grouped: HashSet<usize> = HashSet::new();
        if !self.projects.is_empty() {
            rows.push(RowRef::Header("COMPOSE"));
            for (pi, p) in self.projects.iter().enumerate() {
                for &m in &p.members {
                    grouped.insert(m);
                }
                let member_hit = p
                    .members
                    .iter()
                    .any(|&m| hit(&self.containers[m].name));
                if !hit(&p.name) && !member_hit {
                    continue;
                }
                rows.push(RowRef::Proj(pi));
                if p.expanded || (!f.is_empty() && member_hit) {
                    for &m in &p.members {
                        if f.is_empty() || hit(&self.containers[m].name) || hit(&p.name) {
                            rows.push(RowRef::Cont(m));
                        }
                    }
                }
            }
        }
        let standalone: Vec<usize> = (0..self.containers.len())
            .filter(|i| !grouped.contains(i) && hit(&self.containers[*i].name))
            .collect();
        if !standalone.is_empty() {
            rows.push(RowRef::Blank);
            rows.push(RowRef::Header("STANDALONE"));
            for i in standalone {
                rows.push(RowRef::Cont(i));
            }
        }
        rows.push(RowRef::Blank);
        rows.push(RowRef::Header("IMAGES"));
        for (ri, r) in self.repos.iter().enumerate() {
            let member_hit = r.members.iter().any(|&m| {
                hit(&format!("{}:{}", self.images[m].repo, self.images[m].tag))
            });
            if !hit(&r.key) && !member_hit {
                continue;
            }
            rows.push(RowRef::Repo(ri));
            if r.expanded || (!f.is_empty() && member_hit) {
                for &m in &r.members {
                    rows.push(RowRef::Img(ri, m));
                }
            }
        }
        self.rows = rows;
        if let Some(k) = sel_key {
            if let Some(i) = self.rows.iter().position(|r| *r == k) {
                self.sel = i;
            }
        }
        self.sel = self.sel.min(self.rows.len().saturating_sub(1));
        if !self.selectable(self.sel) {
            self.step_sel(1);
        }
        self.dirty = true;
    }

    fn selectable(&self, i: usize) -> bool {
        matches!(
            self.rows.get(i),
            Some(RowRef::Proj(_) | RowRef::Cont(_) | RowRef::Repo(_) | RowRef::Img(_, _))
        )
    }

    fn step_sel(&mut self, dir: i64) {
        let mut i = self.sel as i64;
        loop {
            i += dir;
            if i < 0 || i as usize >= self.rows.len() {
                return; // stay put at the edges
            }
            if self.selectable(i as usize) {
                self.sel = i as usize;
                self.dirty = true;
                return;
            }
        }
    }

    fn retitle(&mut self, title: String) {
        self.logs_title = title;
        self.log_scroll = 0;
        self.dirty = true;
    }

    // ----- actions -----
    fn open_logs(&mut self) {
        match self.rows.get(self.sel).cloned() {
            Some(RowRef::Cont(i)) => {
                let c = self.containers[i].clone();
                self.logs_for = c.id.clone();
                let sink = self.logs.rebind();
                docker::follow_logs(&c.id, sink);
                self.retitle(format!("logs · {}", c.name));
            }
            Some(RowRef::Proj(pi)) => {
                let (dir, name) = (self.projects[pi].dir.clone(), self.projects[pi].name.clone());
                if dir.is_empty() {
                    self.logs.push_status(format!(
                        "✘ {name}: compose file missing — per-container logs only"
                    ));
                    self.dirty = true;
                } else {
                    self.logs_for.clear();
                    let sink = self.logs.rebind();
                    docker::compose_logs(&dir, &name, sink);
                    self.retitle(format!("compose logs · {name}"));
                }
            }
            _ => {}
        }
    }

    fn start_stop(&mut self) {
        match self.rows.get(self.sel).cloned() {
            Some(RowRef::Cont(i)) => {
                let c = &self.containers[i];
                let verb = if c.state == "running" { "stop" } else { "start" };
                docker::action(verb, c.id.clone(), self.logs.clone());
            }
            Some(RowRef::Proj(pi)) => {
                let p = &self.projects[pi];
                if p.dir.is_empty() {
                    // dir gone: degrade to per-container verbs
                    let v: &'static str = if p.running > 0 { "stop" } else { "start" };
                    for &m in &p.members {
                        docker::action(v, self.containers[m].id.clone(), self.logs.clone());
                    }
                } else {
                    let verb: &'static str = if p.running > 0 { "stop" } else { "up" };
                    docker::compose_action(verb, p.dir.clone(), p.name.clone(), self.logs.clone());
                }
            }
            _ => {}
        }
    }

    fn restart(&mut self) {
        match self.rows.get(self.sel).cloned() {
            Some(RowRef::Cont(i)) => {
                docker::action("restart", self.containers[i].id.clone(), self.logs.clone());
            }
            Some(RowRef::Proj(pi)) => {
                let p = &self.projects[pi];
                if !p.dir.is_empty() {
                    docker::compose_action(
                        "restart",
                        p.dir.clone(),
                        p.name.clone(),
                        self.logs.clone(),
                    );
                }
            }
            _ => {}
        }
    }

    fn remove(&mut self) {
        match self.rows.get(self.sel).cloned() {
            Some(RowRef::Cont(i)) => {
                let c = &self.containers[i];
                self.modal = Modal::Confirm(
                    ConfirmAction::Docker("rm", c.id.clone()),
                    format!("container {}", c.name),
                    false,
                );
            }
            Some(RowRef::Proj(pi)) => {
                let p = &self.projects[pi];
                if !p.dir.is_empty() {
                    self.modal = Modal::Confirm(
                        ConfirmAction::ComposeDown(p.dir.clone(), p.name.clone()),
                        format!("compose down {} ({} containers)", p.name, p.members.len()),
                        false,
                    );
                }
            }
            Some(RowRef::Img(_, i)) => {
                let im = &self.images[i];
                self.modal = Modal::Confirm(
                    ConfirmAction::Docker("rmi", format!("{}:{}", im.repo, im.tag)),
                    format!("image {}:{}", im.repo, im.tag),
                    false,
                );
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn toggle_repo(&mut self, ri: usize) {
        let key = self.repos[ri].key.clone();
        if !self.expanded_repos.remove(&key) {
            self.expanded_repos.insert(key);
        }
        self.repos[ri].expanded = !self.repos[ri].expanded;
        self.rebuild_rows();
    }

    fn toggle_proj(&mut self, pi: usize) {
        let name = self.projects[pi].name.clone();
        if !self.collapsed.remove(&name) {
            self.collapsed.insert(name);
        }
        self.projects[pi].expanded = !self.projects[pi].expanded;
        self.rebuild_rows();
    }

    fn kube_follow_selected(&mut self) {
        if let Some(p) = self.pods.get(self.sel_pod).cloned() {
            let sink = self.logs.rebind();
            kube::follow_logs(&p.ns, &p.name, sink);
            self.retitle(format!("{}/{} — logs", p.ns, p.name));
        }
    }

    // ----- input -----
    fn on_key(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        if self.filter_input {
            match k.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filter_input = false;
                }
                KeyCode::Enter => self.filter_input = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.rebuild_rows();
            return;
        }
        if k.code == KeyCode::Esc && !self.filter.is_empty() && matches!(self.modal, Modal::None) {
            self.filter.clear();
            self.rebuild_rows();
            return;
        }
        match &mut self.modal {
            Modal::Confirm(_, _, armed) => {
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.modal = Modal::None,
                    KeyCode::Tab | KeyCode::Left | KeyCode::Right => *armed = !*armed,
                    KeyCode::Enter => {
                        let m = std::mem::replace(&mut self.modal, Modal::None);
                        if let Modal::Confirm(action, _, true) = m {
                            match action {
                                ConfirmAction::Docker(verb, target) => {
                                    docker::action(verb, target, self.logs.clone())
                                }
                                ConfirmAction::ComposeDown(dir, name) => {
                                    docker::compose_action("down", dir, name, self.logs.clone())
                                }
                                ConfirmAction::PodDelete(ns, pod) => {
                                    kube::delete_pod(ns, pod, self.logs.clone())
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.dirty = true;
                return;
            }
            Modal::Pull(text) => {
                match k.code {
                    KeyCode::Esc => self.modal = Modal::None,
                    KeyCode::Enter => {
                        let image = text.trim().to_string();
                        if !image.is_empty() {
                            docker::pull(&image, self.logs.rebind());
                            self.retitle(format!("pull {image}"));
                        }
                        self.modal = Modal::None;
                    }
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char('u')
                        if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        text.clear()
                    }
                    KeyCode::Char(c) => text.push(c),
                    _ => {}
                }
                self.dirty = true;
                return;
            }
            Modal::None => {}
        }
        // pane-independent
        match k.code {
            KeyCode::Tab => {
                if self.kube_on {
                    self.pane = if self.pane == Pane::Docker { Pane::Kube } else { Pane::Docker };
                    if self.pane == Pane::Kube {
                        self.kube_ctx = kube::current_context();
                        let (pods, err) = kube::pods();
                        self.pods = pods;
                        self.kube_err = err;
                    }
                    self.dirty = true;
                }
                return;
            }
            KeyCode::Char('q') => {
                self.exit = true;
                return;
            }
            KeyCode::PageUp => {
                if self.log_scroll == 0 {
                    self.paused_at = self.logs.lines.lock().unwrap().len();
                }
                self.log_scroll += 20;
                self.dirty = true;
                return;
            }
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_sub(20);
                self.dirty = true;
                return;
            }
            KeyCode::End => {
                self.log_scroll = 0;
                self.dirty = true;
                return;
            }
            KeyCode::Char('R') => {
                self.refresh();
                return;
            }
            _ => {}
        }
        match self.pane {
            Pane::Docker => match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.step_sel(-1),
                KeyCode::Down | KeyCode::Char('j') => self.step_sel(1),
                KeyCode::Left => match self.rows.get(self.sel).cloned() {
                    Some(RowRef::Proj(pi)) if self.projects[pi].expanded => self.toggle_proj(pi),
                    Some(RowRef::Repo(ri)) if self.repos[ri].expanded => self.toggle_repo(ri),
                    _ => {}
                },
                KeyCode::Right => match self.rows.get(self.sel).cloned() {
                    Some(RowRef::Proj(pi)) if !self.projects[pi].expanded => self.toggle_proj(pi),
                    Some(RowRef::Repo(ri)) if !self.repos[ri].expanded => self.toggle_repo(ri),
                    _ => {}
                },
                KeyCode::Enter | KeyCode::Char('l') => {
                    if let Some(RowRef::Repo(ri)) = self.rows.get(self.sel).cloned() {
                        self.toggle_repo(ri);
                    } else {
                        self.open_logs();
                    }
                }
                KeyCode::Char('/') => {
                    self.filter_input = true;
                    self.dirty = true;
                }
                KeyCode::Char('s') => self.start_stop(),
                KeyCode::Char('r') => self.restart(),
                KeyCode::Char('x') => self.remove(),
                KeyCode::Char('c') => {
                    if let Some(RowRef::Cont(i)) = self.rows.get(self.sel).cloned() {
                        let c = &self.containers[i];
                        if c.state == "running" {
                            docker::connect(&c.id, &c.name);
                        } else {
                            self.logs
                                .push_status(format!("○ {} is not running — s starts it", c.name));
                            self.dirty = true;
                        }
                    }
                }
                KeyCode::Char('p') => {
                    // on a compose project: pull ALL its images
                    if let Some(RowRef::Proj(pi)) = self.rows.get(self.sel) {
                        let p = &self.projects[*pi];
                        if p.dir.is_empty() {
                            self.logs.push_status(format!(
                                "✘ {}: compose file missing — pull images individually",
                                p.name
                            ));
                            self.dirty = true;
                        } else {
                            let (file, name) = (p.dir.clone(), p.name.clone());
                            docker::compose_pull(&file, &name, self.logs.rebind());
                            self.retitle(format!("compose pull {name}"));
                        }
                        return;
                    }
                    let prefill = match self.rows.get(self.sel) {
                        Some(RowRef::Img(_, i)) => {
                            let im = &self.images[*i];
                            format!("{}:{}", im.repo, im.tag)
                        }
                        _ => String::new(),
                    };
                    self.modal = Modal::Pull(prefill);
                    self.dirty = true;
                }
                _ => {}
            },
            Pane::Kube => match k.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.sel_pod = self.sel_pod.saturating_sub(1);
                    self.dirty = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.sel_pod + 1 < self.pods.len() {
                        self.sel_pod += 1;
                    }
                    self.dirty = true;
                }
                KeyCode::Enter | KeyCode::Char('l') => self.kube_follow_selected(),
                KeyCode::Char('d') => {
                    if let Some(p) = self.pods.get(self.sel_pod).cloned() {
                        let sink = self.logs.rebind();
                        kube::describe(&p.ns, &p.name, sink);
                        self.retitle(format!("{}/{} — describe", p.ns, p.name));
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(p) = self.pods.get(self.sel_pod) {
                        kube::exec_shell(&p.ns, &p.name);
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(p) = self.pods.get(self.sel_pod).cloned() {
                        self.modal = Modal::Confirm(
                            ConfirmAction::PodDelete(p.ns.clone(), p.name.clone()),
                            format!("pod {}/{}", p.ns, p.name),
                            false,
                        );
                        self.dirty = true;
                    }
                }
                KeyCode::Char('C') => {
                    let all = kube::contexts();
                    if all.len() > 1 {
                        let cur = all.iter().position(|c| *c == self.kube_ctx).unwrap_or(0);
                        let next = all[(cur + 1) % all.len()].clone();
                        kube::use_context(&next, self.logs.clone());
                        self.kube_ctx = next;
                        self.pods.clear();
                        self.kube_err.clear();
                        self.dirty = true;
                    }
                }
                _ => {}
            },
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.modal, Modal::None) {
            return;
        }
        let in_rect = |r: Rect, x: u16, y: u16| {
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !in_rect(self.list_area, m.column, m.row) {
                    return;
                }
                let row = (m.row - self.list_area.y) as usize;
                match self.pane {
                    Pane::Docker => {
                        if row >= self.rows.len() || !self.selectable(row) {
                            return;
                        }
                        let again = row == self.sel;
                        self.sel = row;
                        match self.rows[row].clone() {
                            RowRef::Proj(pi) => self.toggle_proj(pi), // free to undo
                            RowRef::Repo(ri) => self.toggle_repo(ri),
                            _ if again => self.open_logs(),           // click-again opens
                            _ => {}
                        }
                        self.dirty = true;
                    }
                    Pane::Kube => {
                        // row 0 = context header, pods start at row 2
                        if row >= 2 && row - 2 < self.pods.len() {
                            let again = row - 2 == self.sel_pod;
                            self.sel_pod = row - 2;
                            if again {
                                self.kube_follow_selected();
                            }
                            self.dirty = true;
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if in_rect(self.log_area, m.column, m.row) {
                    self.log_scroll += 3;
                } else if self.pane == Pane::Docker {
                    self.step_sel(-1);
                } else {
                    self.sel_pod = self.sel_pod.saturating_sub(1);
                }
                self.dirty = true;
            }
            MouseEventKind::ScrollDown => {
                if in_rect(self.log_area, m.column, m.row) {
                    self.log_scroll = self.log_scroll.saturating_sub(3);
                } else if self.pane == Pane::Docker {
                    self.step_sel(1);
                } else if self.sel_pod + 1 < self.pods.len() {
                    self.sel_pod += 1;
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    // ----- drawing -----
    fn draw(&mut self, f: &mut Frame) {
        let pal = self.pal;
        let outer = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());
        let running = self.containers.iter().filter(|c| c.state == "running").count();
        let mut head = vec![Span::styled(
            " 󰡨 Docker ",
            if self.pane == Pane::Docker {
                Style::default().bg(col(pal.accent)).fg(col(pal.bg)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(col(pal.sub))
            },
        )];
        if self.kube_on {
            head.push(Span::raw(" "));
            head.push(Span::styled(
                " 󱃾 Kubernetes ",
                if self.pane == Pane::Kube {
                    Style::default().bg(col(pal.accent)).fg(col(pal.bg)).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(col(pal.sub))
                },
            ));
            head.push(Span::styled(" ⇥", Style::default().fg(col(pal.muted))));
        }
        head.push(Span::raw("            "));
        head.push(Span::styled("● ", Style::default().fg(col(pal.good))));
        let sep = || Span::styled(" · ", Style::default().fg(col(pal.muted)));
        let num = |n: String| Span::styled(n, Style::default().fg(col(pal.fg)));
        let noun = |t: &str| Span::styled(t.to_string(), Style::default().fg(col(pal.sub)));
        head.push(num(format!("{running}")));
        head.push(noun(" running"));
        head.push(sep());
        head.push(num(format!("{}", self.containers.len())));
        head.push(noun(" containers"));
        head.push(sep());
        head.push(num(format!("{}", self.projects.len())));
        head.push(noun(" compose"));
        head.push(sep());
        head.push(num(format!("{}", self.images.len())));
        head.push(noun(" images"));
        head.push(sep());
        head.push(noun(&format!("v{}", self.version)));
        f.render_widget(Paragraph::new(Line::from(head)), outer[0]);

        let cols = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(outer[1]);
        self.list_area = cols[0];
        self.log_area = cols[1];

        match self.pane {
            Pane::Docker => self.draw_docker_list(f, cols[0]),
            Pane::Kube => self.draw_kube_list(f, cols[0]),
        }
        self.draw_logs(f, cols[1]);
        self.draw_footer(f, outer[2]);

        match &self.modal {
            Modal::Confirm(action, name, armed) => {
                let what = match action {
                    ConfirmAction::Docker("rmi", _) => "Remove this image?",
                    ConfirmAction::ComposeDown(..) => "Compose DOWN this project?",
                    ConfirmAction::PodDelete(..) => "Delete this pod?",
                    _ => "Remove this container?",
                };
                self.draw_confirm(f, what, name, *armed);
            }
            Modal::Pull(text) => self.draw_pull(f, text),
            Modal::None => {}
        }
    }

    fn state_mark(&self, state: &str) -> (&'static str, Color) {
        match state {
            "running" | "Running" | "Completed" => ("●", col(self.pal.good)),
            "restarting" | "paused" | "created" | "Pending" | "ContainerCreating" => {
                ("◌", col(self.pal.warn))
            }
            s if s.contains("Err") || s.contains("Crash") || s.contains("BackOff") => {
                ("✘", col(self.pal.bad))
            }
            _ => ("○", col(self.pal.sub)),
        }
    }

    fn draw_docker_list(&self, f: &mut Frame, area: Rect) {
        let pal = self.pal;
        let width = area.width as usize;
        // one right-aligned status column: right text ends 2 cells in
        let pad_to = |left_len: usize, right_len: usize| -> String {
            " ".repeat(width.saturating_sub(left_len + right_len + 2).max(1))
        };
        let mut lines: Vec<Line> = Vec::new();
        let up_total = self.containers.iter().filter(|c| c.state == "running").count();
        for (ri, row) in self.rows.iter().enumerate().take(area.height as usize) {
            let mut line = match row {
                RowRef::Header(h) => {
                    let (left, right) = match *h {
                        "COMPOSE" => (
                            format!(" COMPOSE  · {} projects", self.projects.len()),
                            format!("{}/{} up", up_total, self.containers.len()),
                        ),
                        "STANDALONE" => {
                            let n = self
                                .containers
                                .iter()
                                .filter(|c| c.project.is_empty())
                                .count();
                            let up = self
                                .containers
                                .iter()
                                .filter(|c| c.project.is_empty() && c.state == "running")
                                .count();
                            (format!(" STANDALONE  · {n}"), format!("{up}/{n} up"))
                        }
                        _ => (
                            format!(
                                " 󰋊 IMAGES  · {} · {} repos",
                                self.images.len(),
                                self.repos.len()
                            ),
                            docker::human_gb(self.repos.iter().map(|r| r.bytes).sum()),
                        ),
                    };
                    Line::from(vec![
                        Span::styled(left.clone(), Style::default().fg(col(pal.fg)).add_modifier(Modifier::BOLD)),
                        Span::raw(pad_to(left.chars().count(), right.chars().count())),
                        Span::styled(right, Style::default().fg(col(pal.sub))),
                    ])
                }
                RowRef::Blank => Line::from(""),
                RowRef::Proj(pi) => {
                    let p = &self.projects[*pi];
                    let arrow = if p.expanded { "▾" } else { "▸" };
                    let full = p.running == p.members.len() && !p.members.is_empty();
                    let right = format!("{}/{}", p.running, p.members.len());
                    let mut spans = vec![
                        Span::styled(format!(" {arrow} "), Style::default().fg(col(pal.sub))),
                        Span::styled("󰡨 ", Style::default().fg(col(pal.sub))),
                        Span::styled(p.name.clone(), Style::default().fg(col(pal.fg))),
                    ];
                    let mut left_len = 3 + 2 + p.name.chars().count();
                    // a collapsed project states its failure (design)
                    if !p.expanded && p.exited_bad > 0 {
                        let t = format!("  {} exited (≠0)", p.exited_bad);
                        left_len += t.chars().count();
                        spans.push(Span::styled(t, Style::default().fg(col(pal.bad))));
                    }
                    spans.push(Span::raw(pad_to(left_len, right.chars().count())));
                    // N/M up: sub while anything is down, good at full strength,
                    // NEVER warn — partial is normal, not transitional
                    spans.push(Span::styled(
                        right,
                        Style::default().fg(if full { col(pal.good) } else { col(pal.sub) }),
                    ));
                    Line::from(spans)
                }
                RowRef::Cont(i) => {
                    let c = &self.containers[*i];
                    let (glyph, ink) = self.state_mark(&c.state);
                    let failed = c.status.contains("Exited") && !c.status.contains("Exited (0)");
                    let (glyph, ink) = if failed { ("✘", col(pal.bad)) } else { (glyph, ink) };
                    let name = if c.service.is_empty() { c.name.clone() } else { c.service.clone() };
                    let indent = if c.project.is_empty() { "   " } else { "      " };
                    let (mid, right) = if c.state == "running" {
                        (
                            if c.cpu.is_empty() { String::new() } else { format!("{} cpu", c.cpu) },
                            c.mem.clone(),
                        )
                    } else {
                        docker::short_status(&c.status)
                    };
                    let name_w = 16usize;
                    let name_pad: String = format!("{name:<name_w$}").chars().take(name_w.max(name.chars().count())).collect();
                    let mut spans = vec![
                        Span::raw(indent.to_string()),
                        Span::styled(format!("{glyph} "), Style::default().fg(ink)),
                        Span::styled(name_pad.clone(), Style::default().fg(col(pal.fg))),
                        Span::raw(" "),
                    ];
                    let mid_ink = if failed {
                        col(pal.bad)
                    } else if c.state == "running" {
                        col(pal.fg)
                    } else {
                        col(pal.sub)
                    };
                    spans.push(Span::styled(mid.clone(), Style::default().fg(mid_ink)));
                    let left_len =
                        indent.len() + 2 + name_pad.chars().count() + 1 + mid.chars().count();
                    spans.push(Span::raw(pad_to(left_len, right.chars().count())));
                    spans.push(Span::styled(
                        right,
                        Style::default().fg(if c.state == "running" { col(pal.fg) } else { col(pal.sub) }),
                    ));
                    Line::from(spans)
                }
                RowRef::Repo(ri2) => {
                    let r = &self.repos[*ri2];
                    let arrow = if r.expanded { "▾" } else { "▸" };
                    let tags = format!(
                        "{} tag{}",
                        r.members.len(),
                        if r.members.len() == 1 { "" } else { "s" }
                    );
                    let right = docker::human_gb(r.bytes);
                    let left = format!(" {arrow} {}", r.key);
                    let mut spans = vec![
                        Span::styled(format!(" {arrow} "), Style::default().fg(col(pal.sub))),
                        Span::styled(r.key.clone(), Style::default().fg(col(pal.sub))),
                    ];
                    let mid_pad = pad_to(
                        left.chars().count() + tags.chars().count() + right.chars().count() + 4,
                        0,
                    );
                    spans.push(Span::raw(mid_pad));
                    spans.push(Span::styled(format!("{tags}    "), Style::default().fg(col(pal.sub))));
                    spans.push(Span::styled(right, Style::default().fg(col(pal.sub))));
                    Line::from(spans)
                }
                RowRef::Img(ri2, i) => {
                    let r = &self.repos[*ri2];
                    let im = &self.images[*i];
                    // strip the group key prefix: aegis-backend:latest -> backend:latest
                    let full = format!("{}:{}", im.repo, im.tag);
                    let short = full
                        .strip_prefix(&format!("{}-", r.key))
                        .or_else(|| full.strip_prefix(&format!("{}/", r.key)))
                        .unwrap_or(&full)
                        .to_string();
                    let right = im.size.clone();
                    let left = format!("     {short}");
                    Line::from(vec![
                        Span::styled(left.clone(), Style::default().fg(col(pal.sub))),
                        Span::raw(pad_to(left.chars().count(), right.chars().count())),
                        Span::styled(right, Style::default().fg(col(pal.sub))),
                    ])
                }
            };
            if ri == self.sel && self.selectable(ri) {
                line = line.style(
                    Style::default()
                        .bg(col(pal.accent))
                        .fg(col(pal.bg))
                        .add_modifier(Modifier::BOLD),
                );
            }
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), area);
    }

    fn draw_kube_list(&self, f: &mut Frame, area: Rect) {
        let pal = self.pal;
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("context ", Style::default().fg(col(pal.sub))),
                Span::styled(
                    self.kube_ctx.clone(),
                    Style::default().fg(col(pal.accent2)).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  (C cycles)", Style::default().fg(col(pal.sub))),
            ]),
            Line::from(""),
        ];
        if !self.kube_err.is_empty() {
            // an unreachable cluster is a STATE, not a blank pane
            lines.push(Line::from(Span::styled(
                format!("✘ {}", self.kube_err),
                Style::default().fg(col(pal.bad)),
            )));
            lines.push(Line::from(Span::styled(
                "cluster unreachable — is minikube/k3s running?",
                Style::default().fg(col(pal.sub)),
            )));
        } else if self.pods.is_empty() {
            lines.push(Line::from(Span::styled(
                "no pods",
                Style::default().fg(col(pal.sub)),
            )));
        }
        for (i, p) in self.pods.iter().enumerate() {
            let (glyph, ink) = self.state_mark(&p.status);
            let mut line = Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{glyph} "), Style::default().fg(ink)),
                Span::raw(p.name.clone()),
                Span::styled(
                    format!("  {} · {} · ↻{} · {}", p.ns, p.ready, p.restarts, p.age),
                    Style::default().fg(col(pal.sub)),
                ),
            ]);
            if i == self.sel_pod {
                line = line.style(
                    Style::default()
                        .bg(col(pal.accent))
                        .fg(col(pal.bg))
                        .add_modifier(Modifier::BOLD),
                );
            }
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), area);
    }

    fn draw_logs(&self, f: &mut Frame, area: Rect) {
        let pal = self.pal;
        let lock = self.logs.lines.lock().unwrap();
        let total = lock.len();
        let follow_state = if self.log_scroll == 0 {
            "following".to_string()
        } else {
            format!("paused · {} new", total.saturating_sub(self.paused_at))
        };
        let title = if self.logs_title.is_empty() {
            " logs — ↵ follows the selection ".to_string()
        } else {
            format!(" {} · {follow_state} ", self.logs_title)
        };
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(col(pal.muted)))
            .title(Span::styled(title, Style::default().fg(col(pal.sub))));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let h = inner.height as usize;
        // the followed container's stopped state earns a closing rule
        let stopped = self
            .containers
            .iter()
            .find(|c| c.id == self.logs_for)
            .filter(|c| c.state != "running")
            .map(|c| docker::short_status(&c.status));
        let extra = usize::from(stopped.is_some());
        let end = total.saturating_sub(self.log_scroll);
        let start = end.saturating_sub(h.saturating_sub(extra));
        let mut log_lines: Vec<Line> = lock
            .iter()
            .skip(start)
            .take(end - start)
            .map(|l| {
                if l.starts_with('✘') {
                    return Line::from(Span::styled(l.clone(), Style::default().fg(col(pal.bad))));
                }
                if l.starts_with('✔') || l.starts_with('⇣') {
                    return Line::from(Span::styled(l.clone(), Style::default().fg(col(pal.good))));
                }
                // level inks: timestamp sub · INFO sub · WARN warn · ERROR bad
                let mut spans: Vec<Span> = Vec::new();
                let mut rest = l.as_str();
                let ts_len = rest
                    .find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.' || c == '-' || c == 'T' || c == 'Z'))
                    .unwrap_or(rest.len());
                if ts_len >= 8 {
                    spans.push(Span::styled(
                        rest[..ts_len].to_string(),
                        Style::default().fg(col(pal.sub)),
                    ));
                    rest = &rest[ts_len..];
                }
                let ink = if rest.contains("ERROR") || rest.contains("FATAL") {
                    col(pal.bad)
                } else if rest.contains("WARN") {
                    col(pal.warn)
                } else if rest.trim_start().starts_with("File ")
                    || rest.starts_with("    ")
                {
                    col(pal.sub) // traceback continuation
                } else {
                    col(pal.fg)
                };
                spans.push(Span::styled(rest.to_string(), Style::default().fg(ink)));
                Line::from(spans)
            })
            .collect();
        drop(lock);
        if let Some((_, age)) = stopped {
            if self.log_scroll == 0 && !self.logs_title.is_empty() {
                let label = if age.is_empty() {
                    " ─── container stopped ─── ".to_string()
                } else {
                    format!(" ─── container stopped · {age} ago ─── ")
                };
                log_lines.push(Line::from(vec![
                    Span::styled("─── ", Style::default().fg(col(pal.muted))),
                    Span::styled(label, Style::default().fg(col(pal.sub))),
                    Span::styled(" ───", Style::default().fg(col(pal.muted))),
                ]));
            }
        }
        f.render_widget(Paragraph::new(log_lines), inner);
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let pal = self.pal;
        if self.filter_input || !self.filter.is_empty() {
            let mut spans = vec![
                Span::styled(" / ", Style::default().fg(col(pal.accent2)).add_modifier(Modifier::BOLD)),
                Span::styled("filter: ", Style::default().fg(col(pal.sub))),
                Span::styled(self.filter.clone(), Style::default().fg(col(pal.fg))),
            ];
            if self.filter_input {
                spans.push(Span::styled("▌", Style::default().fg(col(pal.accent))));
                spans.push(Span::styled(
                    "   ↵ keep · Esc clear",
                    Style::default().fg(col(pal.sub)),
                ));
            } else {
                spans.push(Span::styled(
                    "   Esc clears",
                    Style::default().fg(col(pal.sub)),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        let keys: &[(&str, &str)] = match (self.pane, self.rows.get(self.sel)) {
            (Pane::Kube, _) => &[
                ("↵", "logs"),
                ("d", "describe"),
                ("c", "shell"),
                ("x", "delete"),
                ("C", "context"),
                ("⇥", "docker"),
                ("q", "quit"),
            ],
            (_, Some(RowRef::Proj(_))) => &[
                ("↵", "logs"),
                ("s", "up/stop"),
                ("r", "restart"),
                ("p", "pull"),
                ("x", "down"),
                ("↔", "fold"),
                ("/", "filter"),
                ("q", "quit"),
            ],
            (_, Some(RowRef::Repo(_))) => &[
                ("↵", "fold"),
                ("/", "filter"),
                ("⇥", "k8s"),
                ("q", "quit"),
            ],
            (_, Some(RowRef::Img(_, _))) => &[
                ("p", "pull"),
                ("x", "remove"),
                ("/", "filter"),
                ("q", "quit"),
            ],
            _ => &[
                ("↵", "logs"),
                ("s", "start/stop"),
                ("r", "restart"),
                ("c", "connect"),
                ("x", "remove"),
                ("p", "pull"),
                ("/", "filter"),
                ("⇥", "k8s"),
                ("q", "quit"),
            ],
        };
        let spans: Vec<Span> = keys
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
            .collect();
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_confirm(&self, f: &mut Frame, what: &str, name: &str, armed: bool) {
        let pal = self.pal;
        let area = f.area();
        let w = 56.min(area.width.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(7)) / 2;
        let r = Rect::new(x, y, w, 7);
        f.render_widget(Clear, r);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col(pal.muted)))
            .style(Style::default().bg(col(pal.bg)));
        f.render_widget(block, r);
        let inner = Rect::new(r.x + 2, r.y + 1, r.width - 4, r.height - 2);
        let (cs, qs) = if armed {
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
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    what.to_string(),
                    Style::default().fg(col(pal.fg)).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(name.to_string(), Style::default().fg(col(pal.sub)))),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" Cancel ", cs),
                    Span::raw("  "),
                    Span::styled(" Confirm ", qs),
                ]),
            ]),
            inner,
        );
    }

    fn draw_pull(&self, f: &mut Frame, text: &str) {
        let pal = self.pal;
        let area = f.area();
        let w = 56.min(area.width.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(5)) / 2;
        let r = Rect::new(x, y, w, 5);
        f.render_widget(Clear, r);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col(pal.muted)))
            .style(Style::default().bg(col(pal.bg)))
            .title(Span::styled(
                " docker pull ",
                Style::default().fg(col(pal.accent2)),
            ));
        f.render_widget(block, r);
        let inner = Rect::new(r.x + 2, r.y + 1, r.width - 4, 3);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("image: ", Style::default().fg(col(pal.sub))),
                    Span::raw(text.to_string()),
                    Span::styled("▏", Style::default().fg(col(pal.accent))),
                ]),
                Line::from(Span::styled(
                    "Enter pulls · Esc cancels · ctrl-u clears",
                    Style::default().fg(col(pal.sub)),
                )),
            ]),
            inner,
        );
    }
}

fn tui() {
    let mut stdout = std::io::stdout();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    );
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let Ok(mut terminal) = ratatui::Terminal::new(backend) else { return };

    let mut app = App {
        pal: hyprdesk::colors(),
        version: docker::daemon_version(),
        containers: Vec::new(),
        images: Vec::new(),
        projects: Vec::new(),
        repos: Vec::new(),
        collapsed: HashSet::new(),
        expanded_repos: HashSet::new(),
        filter: String::new(),
        filter_input: false,
        paused_at: 0,
        rows: Vec::new(),
        sel: 1,
        pane: Pane::Docker,
        kube_on: kube::available(),
        kube_ctx: String::new(),
        pods: Vec::new(),
        kube_err: String::new(),
        sel_pod: 0,
        logs: docker::LogSink::new(),
        logs_for: String::new(),
        logs_title: String::new(),
        log_scroll: 0,
        modal: Modal::None,
        list_area: Rect::default(),
        log_area: Rect::default(),
        dirty: true,
        exit: false,
    };
    if app.version.is_empty() {
        app.logs
            .push_status("✘ docker daemon unreachable — is the service running?".into());
    }
    app.refresh();
    let mut last_poll = Instant::now();
    let mut last_log_len = 0usize;

    loop {
        let log_len = app.logs.lines.lock().unwrap().len();
        if log_len != last_log_len {
            last_log_len = log_len;
            app.dirty = true;
        }
        if app.dirty {
            let _ = terminal.draw(|f| app.draw(f));
            app.dirty = false;
        }
        if event::poll(Duration::from_millis(150)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) => app.on_key(k),
                Ok(Event::Mouse(m)) => app.on_mouse(m),
                Ok(Event::Resize(..)) => app.dirty = true,
                _ => {}
            }
        }
        if last_poll.elapsed() >= POLL {
            last_poll = Instant::now();
            app.refresh();
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
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--tui") {
        tui();
    } else if args.iter().any(|a| a == "--waybar") {
        waybar_status();
    } else {
        launch();
    }
}
