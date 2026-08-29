//! hyprdesk-mine — turn Claude Code transcripts into candidate lessons.
//!
//! Deterministic, offline, cheap: no LLM reads the transcripts. It walks
//! `~/.claude/projects/**/*.jsonl` INCREMENTALLY (byte offset per file in
//! `~/.local/state/hyprdesk/miner.json`), pulls out the moments that tend
//! to hold reusable knowledge, redacts anything secret-shaped, and appends
//! them to `~/.local/state/hyprdesk/lessons/YYYY-MM-DD.jsonl`.
//!
//! What counts as a lesson candidate (kind):
//!   correction  the user pushed back ("no,", "don't", "wrong", "actually")
//!               — the correction plus what Claude had just said
//!   fix         a tool call errored and Claude's next words explain the fix
//!   trap        an assistant sentence that names a trap/gotcha/lesson
//!   routine     a command shape used in ≥ 3 distinct sessions (digest only)
//!
//! Usage:
//!   hyprdesk-mine                  mine new transcript bytes, append lessons
//!   hyprdesk-mine --since 7d       …only sessions active in the window
//!   hyprdesk-mine --full           ignore offsets, re-mine everything
//!   hyprdesk-mine --digest [7d]    markdown digest of the lessons files
//!   hyprdesk-mine --json [7d]      the same as JSON (for n8n / LINE)
//!   hyprdesk-mine --format hermes  digest in Hermes' skill-proposal shape

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNIPPET: usize = 320;
const CONTEXT: usize = 220;

fn home() -> PathBuf {
    hyprdesk::home()
}

fn state_dir() -> PathBuf {
    let d = home().join(".local/state/hyprdesk");
    let _ = fs::create_dir_all(d.join("lessons"));
    d
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn parse_since(s: &str) -> f64 {
    let n: f64 = s.trim_end_matches(|c: char| c.is_alphabetic()).parse().unwrap_or(7.0);
    let unit = s.chars().last().unwrap_or('d');
    let secs = match unit {
        'h' => 3600.0,
        'w' => 7.0 * 86400.0,
        _ => 86400.0,
    };
    now() - n * secs
}

fn iso_epoch(ts: &str) -> f64 {
    // 2026-08-29T13:37:54.000Z → epoch; date-only precision is enough here
    let d: Vec<u32> = ts
        .get(..10)
        .unwrap_or("")
        .split('-')
        .filter_map(|p| p.parse().ok())
        .collect();
    if d.len() != 3 {
        return 0.0;
    }
    let (y, m, day) = (d[0] as i64, d[1] as i64, d[2] as i64);
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let hms: Vec<f64> = ts
        .get(11..19)
        .unwrap_or("00:00:00")
        .split(':')
        .filter_map(|p| p.parse().ok())
        .collect();
    let secs = if hms.len() == 3 { hms[0] * 3600.0 + hms[1] * 60.0 + hms[2] } else { 0.0 };
    days as f64 * 86400.0 + secs
}

// ---------------------------------------------------------------- redaction
const SECRET_PREFIXES: &[&str] = &["sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "eyJhbGci"];

fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        let w = word.trim_end();
        let tail = &word[w.len()..];
        let secretish = SECRET_PREFIXES.iter().any(|p| w.starts_with(p) && w.len() > p.len() + 8)
            || (w.len() >= 32 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_alphabetic()) && !w.contains('/'))
            || w.starts_with("-----BEGIN");
        if secretish {
            out.push_str("[redacted]");
        } else {
            out.push_str(w);
        }
        out.push_str(tail);
    }
    // "Bearer <token>" and "password=..." / "token: ..." values
    let mut s = out;
    for key in ["Bearer ", "password=", "PASSWORD=", "token=", "TOKEN=", "api_key=", "API_KEY="] {
        while let Some(i) = s.find(key) {
            let start = i + key.len();
            let end = s[start..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'').map(|e| start + e).unwrap_or(s.len());
            if end > start {
                s.replace_range(start..end, "[redacted]");
            } else {
                break;
            }
        }
    }
    s
}

// ---------------------------------------------------------------- extraction
#[derive(Clone)]
struct Lesson {
    kind: &'static str,
    project: String,
    session: String,
    ts: f64,
    snippet: String,
    context: String,
}

fn clip(s: &str, n: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > n {
        let mut c: String = t.chars().take(n - 1).collect();
        c.push('…');
        c
    } else {
        t
    }
}

fn text_of(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

const CORRECTION_STARTS: &[&str] = &["no,", "no ", "nope", "don't", "do not", "wrong", "not that", "that's not", "thats not", "actually", "stop", "undo", "revert", "instead", "i said", "i meant", "why did you", "you should not", "never "];
const TRAP_WORDS: &[&str] = &["trap", "gotcha", "the fix is", "the fix was", "turns out", "lesson", "root cause", "the culprit", "must not", "the catch", "bit me", "bitten"];

/// `needle` as whole words — "trap" must not match "trapped", "never" not "whenever".
fn has_phrase(low: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(i) = low[start..].find(needle) {
        let a = start + i;
        let b = a + needle.len();
        let before = low[..a].chars().last().map_or(true, |c| !c.is_alphanumeric());
        let after = low[b..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        start = b;
    }
    false
}

fn is_correction(user_text: &str) -> bool {
    let t = user_text.trim().to_lowercase();
    if t.len() < 4 || t.starts_with('<') || t.len() > 600 {
        return false; // system-reminder blobs / pasted logs are not corrections
    }
    CORRECTION_STARTS.iter().any(|p| t.starts_with(p)) || t.contains("that's wrong") || t.contains("not what i")
}

fn trap_sentences(assistant_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for sent in assistant_text.split(|c| c == '.' || c == '\n') {
        let s = sent.trim();
        if s.len() < 30 || s.len() > 400 || s.starts_with('|') || s.starts_with("```") {
            continue;
        }
        let low = s.to_lowercase();
        if TRAP_WORDS.iter().any(|w| has_phrase(&low, w)) {
            out.push(s.to_string());
        }
    }
    out
}

fn cmd_shape(cmd: &str) -> String {
    // "cargo build --release -p x" → "cargo build"; "ssh pi-tailscale …" → "ssh pi-tailscale"
    let toks: Vec<&str> = cmd.split_whitespace().take(2).collect();
    if toks.first().is_some_and(|t| ["cd", "echo", "cat", "ls", "for", "python3", "git"].contains(t)) {
        return String::new();
    }
    toks.join(" ")
}

fn mine_file(path: &Path, from: u64, since: f64, project: &str, routines: &mut HashMap<String, HashSet<String>>) -> (Vec<Lesson>, u64) {
    let mut out = Vec::new();
    let Ok(mut f) = fs::File::open(path) else { return (out, from) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if from >= len {
        return (out, len);
    }
    let _ = f.seek(SeekFrom::Start(from));
    let mut reader = BufReader::new(f);
    let mut consumed = from;
    let mut last_assistant = String::new();
    let mut pending_error: Option<String> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if !line.ends_with('\n') {
            break; // partial line still being written — leave it for next run
        }
        consumed += n as u64;
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let ts = v.get("timestamp").and_then(|t| t.as_str()).map(iso_epoch).unwrap_or(0.0);
        if ts < since {
            continue;
        }
        let session = v.get("sessionId").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let Some(msg) = v.get("message") else { continue };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let text = text_of(msg);
                if let Some(err) = pending_error.take() {
                    if !text.is_empty() {
                        out.push(Lesson { kind: "fix", project: project.into(), session: session.clone(), ts, snippet: redact(&clip(&text, SNIPPET)), context: redact(&clip(&err, CONTEXT)) });
                    }
                }
                for s in trap_sentences(&text) {
                    out.push(Lesson { kind: "trap", project: project.into(), session: session.clone(), ts, snippet: redact(&clip(&s, SNIPPET)), context: String::new() });
                }
                if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") && b.get("name").and_then(|n| n.as_str()) == Some("Bash") {
                            if let Some(cmd) = b.get("input").and_then(|i| i.get("command")).and_then(|c| c.as_str()) {
                                let shape = cmd_shape(cmd);
                                if !shape.is_empty() {
                                    routines.entry(shape).or_default().insert(session.clone());
                                }
                            }
                        }
                    }
                }
                if !text.is_empty() {
                    last_assistant = text;
                }
            }
            Some("user") => {
                if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") && b.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
                            let body = match b.get("content") {
                                Some(Value::String(s)) => s.clone(),
                                Some(Value::Array(a)) => a.iter().filter_map(|x| x.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(" "),
                                _ => String::new(),
                            };
                            if !body.is_empty() {
                                pending_error = Some(body);
                            }
                        }
                    }
                }
                let text = text_of(msg);
                if is_correction(&text) {
                    out.push(Lesson { kind: "correction", project: project.into(), session: session.clone(), ts, snippet: redact(&clip(&text, SNIPPET)), context: redact(&clip(&last_assistant, CONTEXT)) });
                }
            }
            _ => {}
        }
    }
    (out, consumed)
}

// ---------------------------------------------------------------- state + output
fn load_state() -> HashMap<String, u64> {
    fs::read_to_string(state_dir().join("miner.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, u64>>(&s).ok())
        .unwrap_or_default()
}

fn save_state(st: &HashMap<String, u64>) {
    let p = state_dir().join("miner.json");
    let tmp = p.with_extension("tmp");
    if fs::write(&tmp, serde_json::to_string(st).unwrap_or_default()).is_ok() {
        let _ = fs::rename(tmp, p);
    }
}

fn append_lessons(lessons: &[Lesson]) -> PathBuf {
    let day = {
        let t = now() as i64;
        let days = t / 86400;
        // civil from days (Hinnant)
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
    };
    let p = state_dir().join("lessons").join(format!("{day}.jsonl"));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
        for l in lessons {
            let _ = writeln!(f, "{}", json!({"kind": l.kind, "project": l.project, "session": l.session, "ts": l.ts, "snippet": l.snippet, "context": l.context}));
        }
    }
    p
}

fn read_lessons(since: f64) -> Vec<Value> {
    let mut out = Vec::new();
    let dir = state_dir().join("lessons");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for f in files {
        if let Ok(text) = fs::read_to_string(&f) {
            for line in text.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if v.get("ts").and_then(|t| t.as_f64()).unwrap_or(0.0) >= since {
                        out.push(v);
                    }
                }
            }
        }
    }
    // de-duplicate identical snippets (re-mines, repeated reminders)
    let mut seen = HashSet::new();
    out.retain(|v| seen.insert(v.get("snippet").and_then(|s| s.as_str()).unwrap_or("").to_string()));
    out
}

fn digest(since: f64, format: &str) -> String {
    let lessons = read_lessons(since);
    let mut by_project: HashMap<String, Vec<&Value>> = HashMap::new();
    for l in &lessons {
        by_project.entry(l.get("project").and_then(|p| p.as_str()).unwrap_or("?").to_string()).or_default().push(l);
    }
    let mut projects: Vec<_> = by_project.into_iter().collect();
    projects.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    let mut md = String::new();
    if format == "hermes" {
        md.push_str("# Skill proposals for Hermes\n\nEach item is a candidate rule for a SKILL.md under ~/.hermes/skills/. Keep only what generalises beyond one session.\n\n");
    } else {
        md.push_str(&format!("# Lessons digest — {} candidates\n\n", lessons.len()));
    }
    for (proj, items) in projects {
        md.push_str(&format!("## {proj} ({})\n\n", items.len()));
        for kind in ["correction", "fix", "trap"] {
            let ks: Vec<&&Value> = items.iter().filter(|l| l.get("kind").and_then(|k| k.as_str()) == Some(kind)).collect();
            if ks.is_empty() {
                continue;
            }
            md.push_str(&format!("### {kind}\n"));
            for l in ks.iter().take(12) {
                let s = l.get("snippet").and_then(|x| x.as_str()).unwrap_or("");
                let c = l.get("context").and_then(|x| x.as_str()).unwrap_or("");
                if c.is_empty() {
                    md.push_str(&format!("- {s}\n"));
                } else {
                    md.push_str(&format!("- {s}\n  - after: {c}\n"));
                }
            }
            md.push('\n');
        }
    }
    md
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let format = arg_after("--format").unwrap_or_else(|| "md".into());
    if args.iter().any(|a| a == "--digest" || a == "--json") {
        let since = arg_after("--digest").or_else(|| arg_after("--json")).filter(|s| s.ends_with(['d', 'h', 'w'])).map(|s| parse_since(&s)).unwrap_or_else(|| parse_since("7d"));
        if args.iter().any(|a| a == "--json") {
            println!("{}", serde_json::to_string_pretty(&json!({"since": since, "lessons": read_lessons(since)})).unwrap_or_default());
        } else {
            print!("{}", digest(since, &format));
        }
        return;
    }
    let full = args.iter().any(|a| a == "--full");
    let since = arg_after("--since").map(|s| parse_since(&s)).unwrap_or(0.0);
    let mut state = if full { HashMap::new() } else { load_state() };
    let mut routines: HashMap<String, HashSet<String>> = HashMap::new();
    let mut all = Vec::new();
    let mut files = 0usize;
    let mut bytes = 0u64;
    let t0 = std::time::Instant::now();
    let projects = home().join(".claude/projects");
    if let Ok(rd) = fs::read_dir(&projects) {
        for proj in rd.flatten() {
            let pname = proj.file_name().to_string_lossy().to_string();
            let pretty = pname.trim_start_matches('-').replace('-', "/");
            let Ok(tx) = fs::read_dir(proj.path()) else { continue };
            for e in tx.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "jsonl") {
                    let key = p.display().to_string();
                    let from = *state.get(&key).unwrap_or(&0);
                    let (mut ls, to) = mine_file(&p, from, since, &pretty, &mut routines);
                    bytes += to.saturating_sub(from);
                    files += 1;
                    all.append(&mut ls);
                    state.insert(key, to);
                }
            }
        }
    }
    save_state(&state);
    let out = append_lessons(&all);
    let routine_n = routines.values().filter(|s| s.len() >= 3).count();
    eprintln!(
        "mined {files} transcripts, {:.1} MB new, {} lessons ({} corrections, {} fixes, {} traps), {routine_n} routine command shapes, {:.2}s → {}",
        bytes as f64 / 1e6,
        all.len(),
        all.iter().filter(|l| l.kind == "correction").count(),
        all.iter().filter(|l| l.kind == "fix").count(),
        all.iter().filter(|l| l.kind == "trap").count(),
        t0.elapsed().as_secs_f64(),
        out.display()
    );
    if args.iter().any(|a| a == "--routines") {
        let mut r: Vec<_> = routines.into_iter().filter(|(_, s)| s.len() >= 3).collect();
        r.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        for (shape, sessions) in r.iter().take(30) {
            println!("{:3} sessions  {shape}", sessions.len());
        }
    }
}
