//! hypr-docker — Docker control for this desktop, docker-desktop-shaped:
//! containers (start/stop/restart/remove/connect), per-container logs
//! following live, image list + pull. A ratatui terminal panel like the
//! studio sidebar — terminal panels get the palette and the ink
//! hierarchy, never the pixel icons (design doc's own exemption).
//!
//! Modes: (default) focus-or-spawn a kitty window · --tui (inside it).
//! Ink discipline: ● good = running, ○ sub = stopped, warn = transitional
//! (restarting/paused) — status colors never decorate non-status content.

mod docker;

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

#[derive(PartialEq, Clone, Copy)]
enum Pane {
    Containers,
    Images,
}

enum Modal {
    None,
    /// remove confirmation: (verb, target-id, display-name), Tab arms it
    Confirm(&'static str, String, String, bool),
    /// pull prompt with the typed image ref
    Pull(String),
}

struct App {
    pal: hyprdesk::Palette,
    version: String,
    containers: Vec<docker::Container>,
    images: Vec<docker::Image>,
    pane: Pane,
    sel_c: usize,
    sel_i: usize,
    logs: docker::LogSink,
    logs_for: String, // container id the pane is following ("" = none)
    log_scroll: usize, // 0 = follow tail
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
            self.sel_c = self.sel_c.min(self.containers.len().saturating_sub(1));
            self.sel_i = self.sel_i.min(self.images.len().saturating_sub(1));
            self.dirty = true;
        }
    }

    fn selected_container(&self) -> Option<&docker::Container> {
        self.containers.get(self.sel_c)
    }

    fn follow_selected(&mut self) {
        let Some(c) = self.selected_container().cloned() else { return };
        if self.logs_for == c.id {
            return;
        }
        self.logs_for = c.id.clone();
        self.log_scroll = 0;
        let sink = self.logs.rebind();
        docker::follow_logs(&c.id, sink);
        self.dirty = true;
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        match &mut self.modal {
            Modal::Confirm(verb, id, _, armed) => {
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.modal = Modal::None,
                    KeyCode::Tab | KeyCode::Left | KeyCode::Right => *armed = !*armed,
                    KeyCode::Enter => {
                        if *armed {
                            docker::action(verb, id.clone(), self.logs.clone());
                        }
                        self.modal = Modal::None;
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
                            self.logs_for.clear(); // the pane now shows the pull
                        }
                        self.modal = Modal::None;
                    }
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c) => text.push(c),
                    _ => {}
                }
                self.dirty = true;
                return;
            }
            Modal::None => {}
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                match self.pane {
                    Pane::Containers => self.sel_c = self.sel_c.saturating_sub(1),
                    Pane::Images => self.sel_i = self.sel_i.saturating_sub(1),
                }
                self.dirty = true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.pane {
                    Pane::Containers => {
                        if self.sel_c + 1 < self.containers.len() {
                            self.sel_c += 1;
                        }
                    }
                    Pane::Images => {
                        if self.sel_i + 1 < self.images.len() {
                            self.sel_i += 1;
                        }
                    }
                }
                self.dirty = true;
            }
            KeyCode::Tab => {
                self.pane = if self.pane == Pane::Containers { Pane::Images } else { Pane::Containers };
                self.dirty = true;
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if self.pane == Pane::Containers {
                    self.follow_selected();
                }
            }
            KeyCode::Char('s') => {
                if self.pane == Pane::Containers {
                    if let Some(c) = self.selected_container() {
                        let verb = if c.state == "running" { "stop" } else { "start" };
                        docker::action(verb, c.id.clone(), self.logs.clone());
                    }
                }
            }
            KeyCode::Char('r') => {
                if self.pane == Pane::Containers {
                    if let Some(c) = self.selected_container() {
                        docker::action("restart", c.id.clone(), self.logs.clone());
                    }
                }
            }
            KeyCode::Char('c') => {
                if self.pane == Pane::Containers {
                    if let Some(c) = self.selected_container() {
                        if c.state == "running" {
                            docker::connect(&c.id, &c.name);
                        } else {
                            self.logs.push_status(format!("○ {} is not running — s starts it", c.name));
                        }
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Char('x') => match self.pane {
                Pane::Containers => {
                    if let Some(c) = self.selected_container() {
                        self.modal = Modal::Confirm("rm", c.id.clone(), c.name.clone(), false);
                        self.dirty = true;
                    }
                }
                Pane::Images => {
                    if let Some(i) = self.images.get(self.sel_i) {
                        self.modal = Modal::Confirm(
                            "rmi",
                            format!("{}:{}", i.repo, i.tag),
                            format!("image {}:{}", i.repo, i.tag),
                            false,
                        );
                        self.dirty = true;
                    }
                }
            },
            KeyCode::Char('p') => {
                // prefill with the selected image ref — "pull again" is the
                // common case; a blank prompt is one ctrl-u away
                let prefill = match self.pane {
                    Pane::Images => self
                        .images
                        .get(self.sel_i)
                        .map(|i| format!("{}:{}", i.repo, i.tag))
                        .unwrap_or_default(),
                    Pane::Containers => String::new(),
                };
                self.modal = Modal::Pull(prefill);
                self.dirty = true;
            }
            KeyCode::PageUp => {
                self.log_scroll += 20;
                self.dirty = true;
            }
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_sub(20);
                self.dirty = true;
            }
            KeyCode::End => {
                self.log_scroll = 0;
                self.dirty = true;
            }
            KeyCode::Char('R') => {
                self.refresh();
            }
            KeyCode::Char('q') => self.exit = true,
            _ => {}
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
                if in_rect(self.list_area, m.column, m.row) {
                    // rows: containers header, containers, blank, images
                    // header, images — resolved in draw() via row map
                    let row = (m.row - self.list_area.y) as usize;
                    self.click_list_row(row);
                }
            }
            MouseEventKind::ScrollUp => {
                if in_rect(self.log_area, m.column, m.row) {
                    self.log_scroll += 3;
                } else {
                    match self.pane {
                        Pane::Containers => self.sel_c = self.sel_c.saturating_sub(1),
                        Pane::Images => self.sel_i = self.sel_i.saturating_sub(1),
                    }
                }
                self.dirty = true;
            }
            MouseEventKind::ScrollDown => {
                if in_rect(self.log_area, m.column, m.row) {
                    self.log_scroll = self.log_scroll.saturating_sub(3);
                } else {
                    match self.pane {
                        Pane::Containers => {
                            if self.sel_c + 1 < self.containers.len() {
                                self.sel_c += 1;
                            }
                        }
                        Pane::Images => {
                            if self.sel_i + 1 < self.images.len() {
                                self.sel_i += 1;
                            }
                        }
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn click_list_row(&mut self, row: usize) {
        // mirror of draw()'s layout: row 0 = "Containers" header
        if row == 0 {
            return;
        }
        let nc = self.containers.len();
        if row - 1 < nc {
            self.pane = Pane::Containers;
            let was = self.sel_c;
            self.sel_c = row - 1;
            if was == self.sel_c {
                self.follow_selected(); // click the selected row: open logs
            }
            self.dirty = true;
            return;
        }
        // blank + "Images" header
        let img_start = 1 + nc + 2;
        if row >= img_start && row - img_start < self.images.len() {
            self.pane = Pane::Images;
            self.sel_i = row - img_start;
            self.dirty = true;
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let pal = self.pal;
        let outer = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());
        // header
        let running = self.containers.iter().filter(|c| c.state == "running").count();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Docker ",
                    Style::default().fg(col(pal.accent2)).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "v{} · {} running / {} containers · {} images",
                        self.version,
                        running,
                        self.containers.len(),
                        self.images.len()
                    ),
                    Style::default().fg(col(pal.sub)),
                ),
            ])),
            outer[0],
        );

        let cols = Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(outer[1]);
        self.list_area = cols[0];
        self.log_area = cols[1];

        // ---- left: containers + images ----
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            "Containers",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (i, c) in self.containers.iter().enumerate() {
            let (glyph, ink) = match c.state.as_str() {
                "running" => ("●", col(pal.good)),
                "restarting" | "paused" | "created" => ("◌", col(pal.warn)),
                _ => ("○", col(pal.sub)),
            };
            let mut spans = vec![
                Span::raw("  "),
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
            let mut line = Line::from(spans);
            if self.pane == Pane::Containers && i == self.sel_c {
                line = line.style(
                    Style::default()
                        .bg(col(pal.accent))
                        .fg(col(pal.bg))
                        .add_modifier(Modifier::BOLD),
                );
            }
            lines.push(line);
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Images",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (i, im) in self.images.iter().enumerate() {
            let mut line = Line::from(vec![
                Span::raw("  "),
                Span::styled("󰋊 ", Style::default().fg(col(pal.sub))),
                Span::raw(format!("{}:{}", im.repo, im.tag)),
                Span::styled(format!("  {}", im.size), Style::default().fg(col(pal.sub))),
            ]);
            if self.pane == Pane::Images && i == self.sel_i {
                line = line.style(
                    Style::default()
                        .bg(col(pal.accent))
                        .fg(col(pal.bg))
                        .add_modifier(Modifier::BOLD),
                );
            }
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), cols[0]);

        // ---- right: log pane ----
        let title = if self.logs_for.is_empty() {
            " logs — Enter on a container to follow ".to_string()
        } else {
            let name = self
                .containers
                .iter()
                .find(|c| c.id == self.logs_for)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| self.logs_for.clone());
            let tail = if self.log_scroll == 0 { "" } else { " (scrolled)" };
            format!(" {name} — logs{tail} ")
        };
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(col(pal.sub)))
            .title(Span::styled(title, Style::default().fg(col(pal.accent2))));
        let inner = block.inner(cols[1]);
        f.render_widget(block, cols[1]);
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

        // ---- footer ----
        let keys: &[(&str, &str)] = match self.pane {
            Pane::Containers => &[
                ("Enter", "Logs"),
                ("s", "Start/Stop"),
                ("r", "Restart"),
                ("c", "Connect"),
                ("x", "Remove"),
                ("p", "Pull"),
                ("Tab", "Images"),
                ("q", "Quit"),
            ],
            Pane::Images => &[
                ("p", "Pull"),
                ("x", "Remove"),
                ("Tab", "Containers"),
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
        f.render_widget(Paragraph::new(Line::from(spans)), outer[2]);

        match &self.modal {
            Modal::Confirm(verb, _, name, armed) => {
                self.draw_confirm(f, verb, name, *armed);
            }
            Modal::Pull(text) => self.draw_pull(f, text),
            Modal::None => {}
        }
    }

    fn draw_confirm(&self, f: &mut Frame, verb: &str, name: &str, armed: bool) {
        let pal = self.pal;
        let area = f.area();
        let w = 52.min(area.width.saturating_sub(2));
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
        let what = if verb == "rmi" { "Remove this image?" } else { "Remove this container?" };
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
                    what,
                    Style::default().fg(col(pal.fg)).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(name.to_string(), Style::default().fg(col(pal.sub)))),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" Cancel ", cs),
                    Span::raw("  "),
                    Span::styled(" Remove ", qs),
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
                    "Enter pulls · Esc cancels",
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
        pane: Pane::Containers,
        sel_c: 0,
        sel_i: 0,
        logs: docker::LogSink::new(),
        logs_for: String::new(),
        log_scroll: 0,
        modal: Modal::None,
        list_area: Rect::default(),
        log_area: Rect::default(),
        dirty: true,
        exit: false,
    };
    if app.version.is_empty() {
        // graceful fallback beats a blank panel (fleet rule)
        app.logs.push_status("✘ docker daemon unreachable — is the service running?".into());
    }
    app.refresh();
    let mut last_poll = Instant::now();
    let mut last_log_len = 0usize;

    loop {
        // follow mode redraws when the stream grows
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
    if std::env::args().any(|a| a == "--tui") {
        tui();
    } else {
        launch();
    }
}
