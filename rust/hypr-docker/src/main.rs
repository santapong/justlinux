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
            "-o",
            "background_opacity=0.93",
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
    Header(&'static str),
    Proj(usize),
    Cont(usize),
    Img(usize),
    Blank,
}

struct Project {
    name: String,
    dir: String,
    members: Vec<usize>, // indexes into containers
    running: usize,
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
    collapsed: HashSet<String>, // project names the user folded
    rows: Vec<RowRef>,
    sel: usize,
    pane: Pane,
    kube_on: bool,
    kube_ctx: String,
    pods: Vec<kube::Pod>,
    kube_err: String,
    sel_pod: usize,
    logs: docker::LogSink,
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
                }),
            }
        }
        projects.sort_by(|a, b| a.name.cmp(&b.name));
        for p in projects.iter_mut() {
            p.expanded = !self.collapsed.contains(&p.name);
        }
        self.projects = projects;
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let sel_key = self.rows.get(self.sel).cloned();
        let mut rows = vec![RowRef::Header("Containers")];
        let mut grouped: HashSet<usize> = HashSet::new();
        for (pi, p) in self.projects.iter().enumerate() {
            rows.push(RowRef::Proj(pi));
            for &m in &p.members {
                grouped.insert(m);
                if p.expanded {
                    rows.push(RowRef::Cont(m));
                }
            }
        }
        for i in 0..self.containers.len() {
            if !grouped.contains(&i) {
                rows.push(RowRef::Cont(i));
            }
        }
        rows.push(RowRef::Blank);
        rows.push(RowRef::Header("Images"));
        for i in 0..self.images.len() {
            rows.push(RowRef::Img(i));
        }
        self.rows = rows;
        // keep the selection on the same row when it survived the rebuild
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
            Some(RowRef::Proj(_) | RowRef::Cont(_) | RowRef::Img(_))
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
                let sink = self.logs.rebind();
                docker::follow_logs(&c.id, sink);
                self.retitle(format!("{} — logs", c.name));
            }
            Some(RowRef::Proj(pi)) => {
                let (dir, name) = (self.projects[pi].dir.clone(), self.projects[pi].name.clone());
                if dir.is_empty() {
                    self.logs.push_status(format!(
                        "✘ {name}: compose file missing — per-container logs only"
                    ));
                    self.dirty = true;
                } else {
                    let sink = self.logs.rebind();
                    docker::compose_logs(&dir, &name, sink);
                    self.retitle(format!("{name} — compose logs"));
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
            Some(RowRef::Img(i)) => {
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
                KeyCode::Left => {
                    if let Some(RowRef::Proj(pi)) = self.rows.get(self.sel).cloned() {
                        if self.projects[pi].expanded {
                            self.toggle_proj(pi);
                        }
                    }
                }
                KeyCode::Right => {
                    if let Some(RowRef::Proj(pi)) = self.rows.get(self.sel).cloned() {
                        if !self.projects[pi].expanded {
                            self.toggle_proj(pi);
                        }
                    }
                }
                KeyCode::Enter | KeyCode::Char('l') => self.open_logs(),
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
                    let prefill = match self.rows.get(self.sel) {
                        Some(RowRef::Img(i)) => {
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
            Style::default()
                .fg(if self.pane == Pane::Docker { col(pal.accent) } else { col(pal.sub) })
                .add_modifier(Modifier::BOLD),
        )];
        if self.kube_on {
            head.push(Span::styled(
                "󱃾 Kubernetes ",
                Style::default()
                    .fg(if self.pane == Pane::Kube { col(pal.accent) } else { col(pal.sub) })
                    .add_modifier(Modifier::BOLD),
            ));
            head.push(Span::styled("(Tab)  ", Style::default().fg(col(pal.sub))));
        }
        head.push(Span::styled(
            format!(
                "v{} · {} running / {} containers · {} compose · {} images",
                self.version,
                running,
                self.containers.len(),
                self.projects.len(),
                self.images.len()
            ),
            Style::default().fg(col(pal.sub)),
        ));
        f.render_widget(Paragraph::new(Line::from(head)), outer[0]);

        let cols = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
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
        let mut lines: Vec<Line> = Vec::new();
        for (ri, row) in self.rows.iter().enumerate().take(area.height as usize) {
            let mut line = match row {
                RowRef::Header(h) => Line::from(Span::styled(
                    h.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                RowRef::Blank => Line::from(""),
                RowRef::Proj(pi) => {
                    let p = &self.projects[*pi];
                    let arrow = if p.expanded { "▾" } else { "▸" };
                    let ink = if p.running == p.members.len() {
                        col(pal.good)
                    } else if p.running > 0 {
                        col(pal.warn)
                    } else {
                        col(pal.sub)
                    };
                    Line::from(vec![
                        Span::styled(format!("  {arrow} "), Style::default().fg(col(pal.sub))),
                        Span::styled("󰡨 ", Style::default().fg(ink)),
                        Span::raw(p.name.clone()),
                        Span::styled(
                            format!("  {}/{} up", p.running, p.members.len()),
                            Style::default().fg(ink),
                        ),
                    ])
                }
                RowRef::Cont(i) => {
                    let c = &self.containers[*i];
                    let (glyph, ink) = self.state_mark(&c.state);
                    let indent = if c.project.is_empty() { "  " } else { "      " };
                    let mut spans = vec![
                        Span::raw(indent.to_string()),
                        Span::styled(format!("{glyph} "), Style::default().fg(ink)),
                        Span::raw(c.name.clone()),
                    ];
                    if c.state == "running" && !c.cpu.is_empty() {
                        spans.push(Span::styled(
                            format!("  {} {}", c.cpu, c.mem),
                            Style::default().fg(col(pal.sub)),
                        ));
                    } else {
                        spans.push(Span::styled(
                            format!("  {}", c.status),
                            Style::default().fg(col(pal.sub)),
                        ));
                    }
                    Line::from(spans)
                }
                RowRef::Img(i) => {
                    let im = &self.images[*i];
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("󰋊 ", Style::default().fg(col(pal.sub))),
                        Span::raw(format!("{}:{}", im.repo, im.tag)),
                        Span::styled(format!("  {}", im.size), Style::default().fg(col(pal.sub))),
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
        let title = if self.logs_title.is_empty() {
            " logs — Enter follows the selection ".to_string()
        } else {
            let tail = if self.log_scroll == 0 { "" } else { " (scrolled)" };
            format!(" {}{tail} ", self.logs_title)
        };
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(col(pal.sub)))
            .title(Span::styled(title, Style::default().fg(col(pal.accent2))));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let lock = self.logs.lines.lock().unwrap();
        let total = lock.len();
        let h = inner.height as usize;
        let end = total.saturating_sub(self.log_scroll);
        let start = end.saturating_sub(h);
        let log_lines: Vec<Line> = lock
            .iter()
            .skip(start)
            .take(end - start)
            .map(|l| {
                let ink = if l.starts_with('✘') {
                    col(pal.bad)
                } else if l.starts_with('✔') || l.starts_with('⇣') {
                    col(pal.good)
                } else {
                    col(pal.fg)
                };
                Line::from(Span::styled(l.clone(), Style::default().fg(ink)))
            })
            .collect();
        drop(lock);
        f.render_widget(Paragraph::new(log_lines), inner);
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let pal = self.pal;
        let keys: &[(&str, &str)] = match (self.pane, self.rows.get(self.sel)) {
            (Pane::Kube, _) => &[
                ("Enter", "Logs"),
                ("c", "Shell"),
                ("x", "Delete pod"),
                ("C", "Context"),
                ("Tab", "Docker"),
                ("q", "Quit"),
            ],
            (_, Some(RowRef::Proj(_))) => &[
                ("Enter", "Logs"),
                ("s", "Up/Stop"),
                ("r", "Restart"),
                ("x", "Down"),
                ("←→", "Fold"),
                ("Tab", "K8s"),
                ("q", "Quit"),
            ],
            (_, Some(RowRef::Img(_))) => &[
                ("p", "Pull"),
                ("x", "Remove"),
                ("Tab", "K8s"),
                ("q", "Quit"),
            ],
            _ => &[
                ("Enter", "Logs"),
                ("s", "Start/Stop"),
                ("r", "Restart"),
                ("c", "Connect"),
                ("x", "Remove"),
                ("p", "Pull"),
                ("Tab", "K8s"),
                ("q", "Quit"),
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
        collapsed: HashSet::new(),
        rows: Vec::new(),
        sel: 1,
        pane: Pane::Docker,
        kube_on: kube::available(),
        kube_ctx: String::new(),
        pods: Vec::new(),
        kube_err: String::new(),
        sel_pod: 0,
        logs: docker::LogSink::new(),
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
