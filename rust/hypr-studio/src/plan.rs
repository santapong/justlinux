//! `hypr-claude-studio --plan <file>` — a readable, live-following view of a
//! plan-mode plan (`~/.claude/plans/*.md`) in a tmux split beside its tab.
//!
//! Markdown → pre-wrapped styled lines (pulldown-cmark, no external
//! renderer: glow/mdcat are not on this machine and the studio palette
//! must apply). The file is re-read whenever its mtime changes, so the
//! plan fills in as Claude builds it; the scroll offset survives.
//!
//! `--plan-dump <file> <width>` prints the same lines as plain text — the
//! headless check that every plan on disk renders without a panic.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{self, Event, KeyCode, MouseEventKind};
use pulldown_cmark::{Alignment, CodeBlockKind, Event as Md, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

fn col(c: hyprdesk::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

const CODE_INDENT: &str = "  ";

/// One styled run of text inside a block, before wrapping.
#[derive(Clone)]
struct Run {
    text: String,
    style: Style,
}

struct Table {
    rows: Vec<Vec<String>>,
    aligns: Vec<Alignment>,
    cur: Vec<String>,
    cell: String,
    header_rows: usize,
}

struct Renderer<'a> {
    pal: &'a hyprdesk::Palette,
    width: usize,
    lines: Vec<Line<'static>>,
    runs: Vec<Run>,
    // inline state
    bold: usize,
    italic: usize,
    code: bool,
    link: bool,
    // block state
    lists: Vec<Option<u64>>, // None = bullet, Some(next number)
    item_prefix: Option<String>,
    quote: usize,
    code_block: Option<String>,
    heading: Option<HeadingLevel>,
    table: Option<Table>,
    in_head: bool,
}

impl<'a> Renderer<'a> {
    fn new(pal: &'a hyprdesk::Palette, width: usize) -> Self {
        Renderer {
            pal,
            width: width.max(20),
            lines: Vec::new(),
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            code: false,
            link: false,
            lists: Vec::new(),
            item_prefix: None,
            quote: 0,
            code_block: None,
            heading: None,
            table: None,
            in_head: false,
        }
    }

    fn base(&self) -> Style {
        Style::default().fg(col(self.pal.fg))
    }

    fn inline_style(&self) -> Style {
        let mut s = self.base();
        if self.code {
            s = s.fg(col(self.pal.accent2));
        }
        if self.link {
            s = s.add_modifier(Modifier::UNDERLINED);
        }
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if let Some(h) = self.heading {
            s = s.add_modifier(Modifier::BOLD).fg(col(match h {
                HeadingLevel::H1 | HeadingLevel::H2 => self.pal.accent,
                _ => self.pal.accent2,
            }));
        }
        s
    }

    fn push_text(&mut self, text: &str) {
        if let Some(t) = self.table.as_mut() {
            t.cell.push_str(text);
            return;
        }
        if let Some(buf) = self.code_block.as_mut() {
            buf.push_str(text);
            return;
        }
        let style = self.inline_style();
        self.runs.push(Run { text: text.to_string(), style });
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    /// Prefix every wrapped line of the block: quote bars, list indent.
    fn prefixes(&mut self) -> (String, String) {
        let bar = "▎ ".repeat(self.quote);
        let indent = "  ".repeat(self.lists.len().saturating_sub(1));
        let first = match self.item_prefix.take() {
            Some(p) => format!("{bar}{indent}{p}"),
            None => {
                let hang = self.lists.last().map(|_| "  ").unwrap_or("");
                format!("{bar}{indent}{hang}")
            }
        };
        let hang = " ".repeat(first.chars().count() - bar.chars().count());
        (first, format!("{bar}{hang}"))
    }

    /// Greedy word-wrap of the pending runs into lines.
    fn flush(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.runs);
        let (first, rest) = self.prefixes();
        let sub = Style::default().fg(col(self.pal.sub));
        // words with their style; whitespace collapses to single spaces
        let mut words: Vec<(String, Style)> = Vec::new();
        let mut trailing_space = true;
        for r in &runs {
            for (i, piece) in r.text.split(char::is_whitespace).enumerate() {
                if i > 0 {
                    trailing_space = true;
                }
                if piece.is_empty() {
                    continue;
                }
                if !trailing_space {
                    // glue to previous word (style change mid-word)
                    if let Some(last) = words.last_mut() {
                        if last.1 == r.style {
                            last.0.push_str(piece);
                            continue;
                        }
                    }
                    words.push((format!("\u{0}{piece}"), r.style)); // \0 = no space before
                } else {
                    words.push((piece.to_string(), r.style));
                }
                trailing_space = false;
            }
            if r.text.ends_with(char::is_whitespace) {
                trailing_space = true;
            }
        }
        let mut cur: Vec<Span<'static>> = vec![Span::styled(first.clone(), sub)];
        let mut used = first.chars().count();
        let mut first_word = true;
        for (w, style) in words {
            let (glue, w) = match w.strip_prefix('\u{0}') {
                Some(rest) => (true, rest.to_string()),
                None => (false, w),
            };
            let wl = w.chars().count();
            let need = if first_word || glue { wl } else { wl + 1 };
            if used + need > self.width && !first_word {
                self.lines.push(Line::from(std::mem::take(&mut cur)));
                cur.push(Span::styled(rest.clone(), sub));
                used = rest.chars().count();
                cur.push(Span::styled(w, style));
                used += wl;
            } else {
                if !first_word && !glue {
                    cur.push(Span::raw(" "));
                    used += 1;
                }
                cur.push(Span::styled(w, style));
                used += wl;
            }
            first_word = false;
        }
        self.lines.push(Line::from(cur));
    }

    fn rule(&mut self) {
        let w = self.width;
        self.lines.push(Line::from(Span::styled("─".repeat(w), Style::default().fg(col(self.pal.sub)))));
    }

    fn end_code_block(&mut self) {
        let Some(buf) = self.code_block.take() else { return };
        let style = Style::default().fg(col(self.pal.fg)).bg(col(self.pal.muted));
        let w = self.width;
        for l in buf.lines() {
            let mut s = format!("{CODE_INDENT}{l}");
            let n = s.chars().count();
            if n < w {
                s.push_str(&" ".repeat(w - n));
            }
            self.lines.push(Line::from(Span::styled(s, style)));
        }
        self.blank();
    }

    fn end_table(&mut self) {
        let Some(t) = self.table.take() else { return };
        if t.rows.is_empty() {
            return;
        }
        let ncol = t.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![0usize; ncol];
        for r in &t.rows {
            for (i, c) in r.iter().enumerate() {
                widths[i] = widths[i].max(c.chars().count());
            }
        }
        // shrink the widest columns until the table fits
        let sep = 3;
        loop {
            let total: usize = widths.iter().sum::<usize>() + sep * ncol.saturating_sub(1);
            if total <= self.width || widths.iter().all(|w| *w <= 4) {
                break;
            }
            if let Some(m) = widths.iter_mut().max() {
                *m -= 1;
            }
        }
        let sub = Style::default().fg(col(self.pal.sub));
        let fit = |s: &str, w: usize, a: Alignment| -> String {
            let n = s.chars().count();
            let s: String = if n > w {
                let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
                t.push('…');
                t
            } else {
                s.to_string()
            };
            let pad = w.saturating_sub(s.chars().count());
            match a {
                Alignment::Right => format!("{}{s}", " ".repeat(pad)),
                Alignment::Center => format!("{}{s}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2)),
                _ => format!("{s}{}", " ".repeat(pad)),
            }
        };
        for (ri, r) in t.rows.iter().enumerate() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" │ ", sub));
                }
                let cell = r.get(i).map(|s| s.as_str()).unwrap_or("");
                let a = t.aligns.get(i).copied().unwrap_or(Alignment::None);
                let st = if ri < t.header_rows {
                    self.base().add_modifier(Modifier::BOLD)
                } else {
                    self.base()
                };
                spans.push(Span::styled(fit(cell, *w, a), st));
            }
            self.lines.push(Line::from(spans));
            if ri + 1 == t.header_rows {
                let mut s = String::new();
                for (i, w) in widths.iter().enumerate() {
                    if i > 0 {
                        s.push_str("─┼─");
                    }
                    s.push_str(&"─".repeat(*w));
                }
                self.lines.push(Line::from(Span::styled(s, sub)));
            }
        }
        self.blank();
    }

    fn event(&mut self, ev: Md) {
        match ev {
            Md::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { level, .. } => {
                    self.blank();
                    self.heading = Some(level);
                }
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.quote += 1;
                }
                Tag::CodeBlock(kind) => {
                    self.flush();
                    let _lang = match kind {
                        CodeBlockKind::Fenced(l) => l.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.code_block = Some(String::new());
                }
                Tag::List(start) => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                    self.lists.push(start);
                }
                Tag::Item => {
                    self.flush();
                    let marker = match self.lists.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}. ");
                            *n += 1;
                            m
                        }
                        _ => "• ".to_string(),
                    };
                    self.item_prefix = Some(marker);
                }
                Tag::Emphasis => self.italic += 1,
                Tag::Strong => self.bold += 1,
                Tag::Link { .. } => self.link = true,
                Tag::Table(aligns) => {
                    self.flush();
                    self.blank();
                    self.table = Some(Table {
                        rows: Vec::new(),
                        aligns,
                        cur: Vec::new(),
                        cell: String::new(),
                        header_rows: 0,
                    });
                }
                Tag::TableHead => self.in_head = true,
                Tag::TableRow => {}
                Tag::TableCell => {
                    if let Some(t) = self.table.as_mut() {
                        t.cell.clear();
                    }
                }
                _ => {}
            },
            Md::End(tag) => match tag {
                TagEnd::Paragraph => {
                    self.flush();
                    if self.lists.is_empty() && self.quote == 0 {
                        self.blank();
                    }
                }
                TagEnd::Heading(level) => {
                    self.flush();
                    self.heading = None;
                    if level == HeadingLevel::H1 {
                        self.rule();
                    }
                    self.blank();
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quote = self.quote.saturating_sub(1);
                    self.blank();
                }
                TagEnd::CodeBlock => self.end_code_block(),
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                }
                TagEnd::Item => {
                    self.flush();
                    if let Some(p) = self.item_prefix.take() {
                        // empty item: still show the marker
                        self.runs.push(Run { text: String::new(), style: self.base() });
                        self.item_prefix = Some(p);
                        self.flush();
                    }
                }
                TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
                TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
                TagEnd::Link => self.link = false,
                TagEnd::Table => self.end_table(),
                TagEnd::TableHead => {
                    self.in_head = false;
                    if let Some(t) = self.table.as_mut() {
                        let row = std::mem::take(&mut t.cur);
                        t.rows.push(row);
                        t.header_rows = 1;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = self.table.as_mut() {
                        let row = std::mem::take(&mut t.cur);
                        t.rows.push(row);
                    }
                }
                TagEnd::TableCell => {
                    if let Some(t) = self.table.as_mut() {
                        let c = std::mem::take(&mut t.cell);
                        t.cur.push(c.split_whitespace().collect::<Vec<_>>().join(" "));
                    }
                }
                _ => {}
            },
            Md::Text(t) => self.push_text(&t),
            Md::Code(t) => {
                if self.table.is_some() || self.code_block.is_some() {
                    self.push_text(&t);
                } else {
                    self.code = true;
                    self.push_text(&t);
                    self.code = false;
                }
            }
            Md::SoftBreak => self.push_text(" "),
            Md::HardBreak => {
                self.flush();
            }
            Md::Rule => {
                self.flush();
                self.blank();
                self.rule();
                self.blank();
            }
            Md::TaskListMarker(done) => {
                let mark = if done { "[x] " } else { "[ ] " };
                if let Some(p) = self.item_prefix.as_mut() {
                    p.push_str(mark);
                } else {
                    self.push_text(mark);
                }
            }
            Md::Html(t) | Md::InlineHtml(t) => self.push_text(&t),
            _ => {}
        }
    }
}

/// Markdown → pre-wrapped lines at `width`.
pub fn render(md: &str, width: usize, pal: &hyprdesk::Palette) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut r = Renderer::new(pal, width);
    for ev in Parser::new_ext(md, opts) {
        r.event(ev);
    }
    r.flush();
    r.end_code_block();
    r.end_table();
    while r.lines.last().is_some_and(|l| l.spans.is_empty()) {
        r.lines.pop();
    }
    r.lines
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Headless: print the rendered lines as plain text.
pub fn dump(path: &Path, width: usize) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let pal = hyprdesk::colors();
    for l in render(&text, width, &pal) {
        let s: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        println!("{}", s.trim_end());
    }
}

/// The interactive viewer.
pub fn run(path: &Path) {
    let pal = hyprdesk::colors();
    let mut stdout = std::io::stdout();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    );
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let Ok(mut terminal) = ratatui::Terminal::new(backend) else { return };

    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    let mut seen = mtime(path);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut width = 0usize;
    let mut scroll: usize = 0;
    let mut dirty = true;
    let mut last_check = Instant::now();
    let mut follow = true; // stick to the end while Claude is still writing

    loop {
        if dirty {
            let _ = terminal.draw(|f| {
                let area = f.area();
                let w = area.width as usize;
                if w != width {
                    width = w;
                    lines = render(&text, width.saturating_sub(1), &pal);
                }
                let body = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(2) };
                let max = lines.len().saturating_sub(body.height as usize);
                if follow {
                    scroll = max;
                }
                scroll = scroll.min(max);
                let pct = if max == 0 { 100 } else { scroll * 100 / max };
                let hdr = Line::from(vec![
                    Span::styled(format!("󰈙 {name}"), Style::default().fg(col(pal.accent)).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("   {} lines · {pct}%{}", lines.len(), if follow { " · following" } else { "" }),
                        Style::default().fg(col(pal.sub)),
                    ),
                ]);
                f.render_widget(Paragraph::new(hdr), Rect { height: 1, ..area });
                let view: Vec<Line<'static>> = lines.iter().skip(scroll).take(body.height as usize).cloned().collect();
                f.render_widget(Paragraph::new(view), body);
                let foot = Line::from(Span::styled(
                    " q close · j/k scroll · G end (follow) · e edit · y path · o zoom · r reload",
                    Style::default().fg(col(pal.sub)),
                ));
                f.render_widget(Paragraph::new(foot), Rect { y: area.y + area.height.saturating_sub(1), height: 1, ..area });
            });
            dirty = false;
        }
        if event::poll(Duration::from_millis(250)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) => {
                    let page = terminal.size().map(|s| s.height as usize / 2).unwrap_or(10).max(1);
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('j') | KeyCode::Down => {
                            scroll += 1;
                            follow = false;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            scroll = scroll.saturating_sub(1);
                            follow = false;
                        }
                        KeyCode::Char('d') | KeyCode::PageDown | KeyCode::Char(' ') => {
                            scroll += page;
                            follow = false;
                        }
                        KeyCode::Char('u') | KeyCode::PageUp => {
                            scroll = scroll.saturating_sub(page);
                            follow = false;
                        }
                        KeyCode::Char('g') | KeyCode::Home => {
                            scroll = 0;
                            follow = false;
                        }
                        KeyCode::Char('G') | KeyCode::End => follow = true,
                        KeyCode::Char('r') => {
                            text = std::fs::read_to_string(path).unwrap_or_default();
                            width = 0;
                        }
                        KeyCode::Char('e') => {
                            let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::terminal::LeaveAlternateScreen);
                            let _ = crossterm::terminal::disable_raw_mode();
                            let _ = std::process::Command::new("nvim").arg(path).status();
                            let _ = crossterm::terminal::enable_raw_mode();
                            let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::event::EnableMouseCapture);
                            let _ = terminal.clear();
                            text = std::fs::read_to_string(path).unwrap_or_default();
                            seen = mtime(path);
                            width = 0;
                        }
                        KeyCode::Char('y') => {
                            let _ = std::process::Command::new("wl-copy")
                                .arg(path.display().to_string())
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status();
                        }
                        KeyCode::Char('o') => {
                            super::tmux(&["resize-pane", "-Z"]);
                        }
                        _ => {}
                    }
                    dirty = true;
                }
                Ok(Event::Mouse(m)) => {
                    match m.kind {
                        MouseEventKind::ScrollDown => {
                            scroll += 3;
                            follow = false;
                        }
                        MouseEventKind::ScrollUp => {
                            scroll = scroll.saturating_sub(3);
                            follow = false;
                        }
                        _ => {}
                    }
                    dirty = true;
                }
                Ok(Event::Resize(..)) => dirty = true,
                _ => {}
            }
        }
        if last_check.elapsed() >= Duration::from_millis(500) {
            last_check = Instant::now();
            let now = mtime(path);
            if now != seen {
                seen = now;
                text = std::fs::read_to_string(path).unwrap_or_default();
                width = 0; // force re-render
                dirty = true;
            }
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
