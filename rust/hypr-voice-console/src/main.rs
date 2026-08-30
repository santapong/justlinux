//! hypr-voice-console — what Draven heard, and the knobs.
//!
//! A floating kitty (class `hyprvoiceconsole`, ALT+CTRL+V) running one
//! ratatui screen: live mic level + wake score against the threshold tick,
//! the transcript history (`~/.local/state/hyprdesk/voice-log.jsonl`) with
//! what each utterance did, and the controls — every one of them a single
//! line written to `$XDG_RUNTIME_DIR/hypr-voice.cmd`, which the daemon
//! consumes on its next 160 ms chunk. While this window is open the daemon
//! publishes the meter (flag file `hypr-voice.console-open`); closing the
//! console removes the flag, so the meter costs nothing the rest of the day.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde_json::Value;

const KITTY_CLASS: &str = "hyprvoiceconsole";

fn col(c: hyprdesk::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn runtime() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

fn hypr_json(args: &[&str]) -> Value {
    Command::new("hyprctl")
        .args(args)
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or(Value::Null)
}

/// Focus the console if it is already open, else spawn it in kitty.

/// Two binds (or two fast presses) must never open two windows: the first
/// launcher holds this lock while it spawns; the second sees it and exits.
fn launch_lock() -> Option<std::fs::File> {
    use fs2::FileExt;
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let f = std::fs::OpenOptions::new().create(true).write(true).open(format!("{dir}/hypr-voice-console.launch")).ok()?;
    if f.try_lock_exclusive().is_err() {
        return None;
    }
    Some(f)
}

fn launch() {
    let Some(_lock) = launch_lock() else { return }; // another launch is in flight
    if let Some(clients) = hypr_json(&["clients", "-j"]).as_array() {
        for c in clients {
            if c.get("class").and_then(|v| v.as_str()) == Some(KITTY_CLASS) {
                let addr = format!("address:{}", c.get("address").and_then(|v| v.as_str()).unwrap_or(""));
                let ws = hypr_json(&["-j", "activeworkspace"]).get("id").and_then(|v| v.as_i64()).unwrap_or(1);
                let _ = Command::new("hyprctl").args(["dispatch", "movetoworkspace", &format!("{ws},{addr}")]).status();
                let _ = Command::new("hyprctl").args(["dispatch", "focuswindow", &addr]).status();
                return;
            }
        }
    }
    let me = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "hypr-voice-console".into());
    let _ = Command::new("setsid")
        .args(["kitty", "--class", KITTY_CLASS, "--title", "DravenIQ voice", "-e", &me, "--tui"])
        .spawn();
    // the hyprland.conf rule floats it; a rule added live by `hyprctl
    // keyword` was seen NOT to apply — so make sure ourselves once mapped
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        if let Some(clients) = hypr_json(&["clients", "-j"]).as_array() {
            if let Some(c) = clients.iter().find(|c| c.get("class").and_then(|v| v.as_str()) == Some(KITTY_CLASS)) {
                let addr = format!("address:{}", c.get("address").and_then(|v| v.as_str()).unwrap_or(""));
                if c.get("floating").and_then(|v| v.as_bool()) != Some(true) {
                    let _ = Command::new("hyprctl").args(["dispatch", "setfloating", &addr]).status();
                    let _ = Command::new("hyprctl").args(["dispatch", "resizewindowpixel", &format!("exact 960 600,{addr}")]).status();
                    let _ = Command::new("hyprctl").args(["dispatch", "centerwindow"]).status();
                }
                return;
            }
        }
    }
}

fn send_cmd(line: &str) {
    let p = runtime().join("hypr-voice.cmd");
    let tmp = runtime().join("hypr-voice.cmd.tmp");
    if std::fs::write(&tmp, format!("{line}\n")).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

fn service_active() -> bool {
    Command::new("systemctl").args(["--user", "is-active", "--quiet", "hypr-voice.service"]).status().map(|s| s.success()).unwrap_or(false)
}

#[derive(Clone)]
struct Entry {
    ts: f64,
    kind: String,
    text: String,
    actions: Vec<Value>,
    score: f64,
    secs: f64,
    forced: bool,
}

fn read_log(max: usize) -> Vec<Entry> {
    let p = hyprdesk::home().join(".local/state/hyprdesk/voice-log.jsonl");
    let text = std::fs::read_to_string(p).unwrap_or_default();
    let mut out: Vec<Entry> = text
        .lines()
        .rev()
        .take(max)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| Entry {
            ts: v.get("ts").and_then(|x| x.as_f64()).unwrap_or(0.0),
            kind: v.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            actions: v.get("actions").and_then(|x| x.as_array()).cloned().unwrap_or_default(),
            score: v.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0),
            secs: v.get("secs").and_then(|x| x.as_f64()).unwrap_or(0.0),
            forced: v.get("forced").and_then(|x| x.as_bool()).unwrap_or(false),
        })
        .collect();
    out.reverse();
    out
}

fn describe(actions: &[Value]) -> String {
    if actions.is_empty() {
        return "✗ nothing matched".into();
    }
    actions
        .iter()
        .map(|a| {
            let verb = a.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let payload = a.get(1).cloned().unwrap_or(Value::Null);
            match verb {
                "new_tab" => format!(
                    "→ {} {} tab(s)",
                    payload.get("n").and_then(|n| n.as_u64()).unwrap_or(1),
                    payload.get("agent").and_then(|s| s.as_str()).unwrap_or("claude")
                ),
                "dictate" => format!("→ typed “{}”", payload.as_str().unwrap_or("")),
                "unknown" => "✗ didn't match".into(),
                "select_tab" => format!("→ {} tab", payload.as_str().unwrap_or("")),
                other => format!("→ {other}"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn hms(ts: f64) -> String {
    let secs = ts as i64;
    let local = secs + 7 * 3600; // Asia/Bangkok, the machine's zone
    format!("{:02}:{:02}:{:02}", (local / 3600) % 24, (local / 60) % 60, local % 60)
}

struct State {
    state: String,
    text: String,
}

fn read_state() -> State {
    let v: Value = std::fs::read_to_string(runtime().join("hypr-voice.state"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    State {
        state: v.get("state").and_then(|x| x.as_str()).unwrap_or("idle").into(),
        text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").into(),
    }
}

fn read_meter() -> (f64, f64, f64) {
    let s = std::fs::read_to_string(runtime().join("hypr-voice.meter")).unwrap_or_default();
    let mut it = s.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
    (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.6))
}

fn bar(frac: f64, width: usize, tick: Option<f64>) -> String {
    let n = ((frac.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let mut s: Vec<char> = (0..width).map(|i| if i < n { '█' } else { '·' }).collect();
    if let Some(t) = tick {
        let ti = ((t.clamp(0.0, 1.0)) * (width - 1) as f64).round() as usize;
        if ti < width {
            s[ti] = if ti < n { '┃' } else { '│' };
        }
    }
    s.into_iter().collect()
}

struct App {
    pal: hyprdesk::Palette,
    entries: Vec<Entry>,
    filter: String,
    filter_mode: bool,
    say_mode: bool,
    say: String,
    threshold: f64,
    peak: f64,
    peak_at: Instant,
    notify: Option<(String, Instant)>,
    exit: bool,
}

impl App {
    fn new() -> App {
        App {
            pal: hyprdesk::colors(),
            entries: read_log(200),
            filter: String::new(),
            filter_mode: false,
            say_mode: false,
            say: String::new(),
            threshold: hyprdesk::conf_get("voice_threshold", "0.6").parse().unwrap_or(0.6),
            peak: 0.0,
            peak_at: Instant::now(),
            notify: None,
            exit: false,
        }
    }

    fn say(&mut self, msg: &str) {
        self.notify = Some((msg.to_string(), Instant::now() + Duration::from_secs(3)));
    }

    fn set_threshold(&mut self, delta: f64) {
        self.threshold = ((self.threshold + delta) * 100.0).round() / 100.0;
        self.threshold = self.threshold.clamp(0.1, 0.95);
        hyprdesk::conf_set("voice_threshold", &format!("{:.2}", self.threshold));
        send_cmd(&format!("threshold {:.2}", self.threshold));
        let t = self.threshold;
        self.say(&format!("threshold {t:.2} (saved)"));
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.say_mode {
            match k.code {
                KeyCode::Esc => {
                    self.say_mode = false;
                    self.say.clear();
                }
                KeyCode::Enter => {
                    if !self.say.trim().is_empty() {
                        send_cmd(&format!("say {}", self.say.trim()));
                        self.say(&format!("sent: {}", self.say.trim()));
                    }
                    self.say_mode = false;
                    self.say.clear();
                }
                KeyCode::Backspace => {
                    self.say.pop();
                }
                KeyCode::Char(c) => self.say.push(c),
                _ => {}
            }
            return;
        }
        if self.filter_mode {
            match k.code {
                KeyCode::Esc => {
                    self.filter_mode = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.filter_mode = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.exit = true,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.exit = true,
            KeyCode::Char('l') | KeyCode::Char(' ') => {
                send_cmd("listen");
                self.say("listening — say the command");
            }
            KeyCode::Char('p') => {
                let st = read_state();
                send_cmd(if st.state == "paused" { "resume" } else { "pause" });
            }
            KeyCode::Char('t') => {
                let _ = Command::new(hyprdesk::home().join(".local/bin/hypr-voice")).arg("--toggle").status();
                self.say(if service_active() { "voice OFF" } else { "voice ON" });
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.set_threshold(0.05),
            KeyCode::Char('-') => self.set_threshold(-0.05),
            KeyCode::Char('r') => {
                send_cmd("retry");
                self.say("retrying the last command");
            }
            KeyCode::Char('s') => self.say_mode = true,
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Char('e') => {
                let _ = Command::new("setsid")
                    .args([
                        "kitty", "--class", "floatterm", "--title", "voice grammar", "nvim",
                        &hyprdesk::home().join(".local/lib/hyprdesk/voice_intents.py").display().to_string(),
                    ])
                    .spawn();
            }
            KeyCode::Char('x') => {
                let p = hyprdesk::home().join(".local/state/hyprdesk/voice-log.jsonl");
                let _ = std::fs::write(&p, "");
                self.entries.clear();
                self.say("history cleared");
            }
            _ => {}
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let pal = self.pal.clone();
        let area = f.area();
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Length(4), Constraint::Min(3), Constraint::Length(2)]).split(area);
        let sub = Style::default().fg(col(pal.sub));
        let fg = Style::default().fg(col(pal.fg));
        let acc = Style::default().fg(col(pal.accent)).add_modifier(Modifier::BOLD);

        // header
        let st = read_state();
        let active = service_active();
        let (state_txt, state_ink) = match (active, st.state.as_str()) {
            (false, _) => ("OFF".to_string(), pal.sub),
            (_, "listening") => ("● LISTENING — say the command".into(), pal.accent),
            (_, "asking") => ("● YES? — say the command".into(), pal.accent),
            (_, "thinking") => ("… thinking".into(), pal.sub),
            (_, "heard") => (format!("heard: “{}”", st.text), pal.good),
            (_, "paused") => ("paused".into(), pal.bad),
            _ => ("listening for the wake word".into(), pal.good),
        };
        let header = vec![
            Line::from(vec![Span::styled(" 󰍬 DravenIQ voice", acc), Span::styled(format!("   {state_txt}"), Style::default().fg(col(state_ink)))]),
            Line::from(vec![Span::styled(
                format!(
                    "   wake word: {}   threshold {:.2}   service {}",
                    if hyprdesk::home().join(".local/share/hyprdesk/voice").read_dir().map(|d| d.flatten().any(|e| e.path().extension().is_some_and(|x| x == "onnx"))).unwrap_or(false) { "custom model" } else { "“hey jarvis” (until hey_draven.onnx exists)" },
                    self.threshold,
                    if active { "on" } else { "off" }
                ),
                sub,
            )]),
        ];
        f.render_widget(Paragraph::new(header), chunks[0]);

        // meters
        let (level, score, thr) = read_meter();
        if score > self.peak || self.peak_at.elapsed() > Duration::from_secs(3) {
            self.peak = score;
            self.peak_at = Instant::now();
        }
        let w = (chunks[1].width as usize).saturating_sub(34).max(10);
        let lvl_frac = (level / 3000.0).min(1.0);
        let meters = vec![
            Line::from(vec![Span::styled(format!("   mic   {:>5}  ", level as i64), sub), Span::styled(bar(lvl_frac, w, None), Style::default().fg(col(if level > 600.0 { pal.good } else { pal.sub })))]),
            Line::from(vec![
                Span::styled(format!("   wake  {score:>5.2}  "), sub),
                Span::styled(bar(score, w, Some(thr)), Style::default().fg(col(if score >= thr { pal.good } else { pal.accent }))),
                Span::styled(format!("  peak {:.2}", self.peak), sub),
            ]),
            Line::from(Span::styled("         the │ tick is the threshold — “hey draven” must push the bar past it (l or ALT+CTRL+M skips the wake word)", sub)),
        ];
        f.render_widget(Paragraph::new(meters), chunks[1]);

        // history
        let q = self.filter.to_lowercase();
        let rows: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.kind == "heard" || e.kind == "actions" || e.kind == "ignored" || (e.kind == "wake" && e.forced))
            .filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q) || describe(&e.actions).to_lowercase().contains(&q))
            .collect();
        let h = chunks[2].height.saturating_sub(2) as usize;
        let start = rows.len().saturating_sub(h);
        let mut lines: Vec<Line> = Vec::new();
        for e in &rows[start..] {
            if e.kind == "wake" {
                lines.push(Line::from(vec![Span::styled(format!(" {}  ", hms(e.ts)), sub), Span::styled("push-to-talk", sub)]));
                continue;
            }
            if e.kind == "ignored" {
                lines.push(Line::from(vec![Span::styled(format!(" {}  ", hms(e.ts)), sub), Span::styled(format!("“{}”", e.text), sub), Span::styled("  ignored — no “Draven” in it", sub)]));
                continue;
            }
            let heard = if e.text.is_empty() { "(nothing heard)".to_string() } else { format!("“{}”", e.text) };
            let did = describe(&e.actions);
            let did_ink = if did.starts_with('✗') { pal.bad } else { pal.good };
            lines.push(Line::from(vec![
                Span::styled(format!(" {}  ", hms(e.ts)), sub),
                Span::styled(heard, fg),
                Span::styled(format!("  {did}"), Style::default().fg(col(did_ink))),
                Span::styled(if e.secs > 0.0 { format!("  {:.1}s", e.secs) } else { String::new() }, sub),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled("   nothing heard yet — say the wake word, or press l to listen now", sub)));
        }
        let title = if self.filter.is_empty() { " heard ".to_string() } else { format!(" heard · /{} ", self.filter) };
        let block = Block::default().borders(Borders::TOP).border_type(BorderType::Plain).border_style(sub).title(Span::styled(title, sub));
        f.render_widget(Paragraph::new(lines).block(block), chunks[2]);

        // footer
        let keys = [("l/space", "listen now"), ("p", "pause"), ("t", "on/off"), ("+/-", "threshold"), ("r", "retry"), ("s", "say…"), ("/", "filter"), ("e", "grammar"), ("x", "clear"), ("q", "quit")];
        let mut spans: Vec<Span> = Vec::new();
        for (k, v) in keys {
            spans.push(Span::styled(format!(" {k}"), acc));
            spans.push(Span::styled(format!(" {v} "), sub));
        }
        let mut foot = vec![Line::from(spans)];
        if self.say_mode {
            foot.push(Line::from(vec![Span::styled(" say> ", acc), Span::styled(self.say.clone(), fg), Span::styled("▏", acc)]));
        } else if self.filter_mode {
            foot.push(Line::from(vec![Span::styled(" /", acc), Span::styled(self.filter.clone(), fg), Span::styled("▏", acc)]));
        } else if let Some((msg, until)) = &self.notify {
            if Instant::now() < *until {
                foot.push(Line::from(Span::styled(format!(" {msg}"), Style::default().fg(col(pal.good)))));
            }
        }
        f.render_widget(Paragraph::new(foot), chunks[3]);
        let _ = Clear;
        let _ = Rect::default();
    }
}

fn tui() {
    let flag = runtime().join("hypr-voice.console-open");
    let _ = std::fs::write(&flag, "");
    let mut stdout = std::io::stdout();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen);
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let Ok(mut terminal) = ratatui::Terminal::new(backend) else { return };
    let mut app = App::new();
    let mut last_log = SystemTime::UNIX_EPOCH;
    let log_path = hyprdesk::home().join(".local/state/hyprdesk/voice-log.jsonl");
    let mut last_beat = Instant::now();
    loop {
        if last_beat.elapsed() >= Duration::from_secs(1) {
            last_beat = Instant::now();
            let _ = std::fs::write(&flag, ""); // heartbeat: a killed console stops it within 3 s
        }
        let _ = terminal.draw(|f| app.draw(f));
        if event::poll(Duration::from_millis(120)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                app.on_key(k);
            }
        }
        let mt = std::fs::metadata(&log_path).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        if mt != last_log {
            last_log = mt;
            app.entries = read_log(200);
        }
        if app.exit {
            break;
        }
    }
    let _ = std::fs::remove_file(&flag);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = std::io::stdout().flush();
    let _ = UNIX_EPOCH.elapsed();
}

fn main() {
    if std::env::args().any(|a| a == "--tui") {
        tui();
    } else {
        launch();
    }
}
