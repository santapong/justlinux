//! The claudesessions subset the office needs, ported from
//! lib/hyprdesk/claudesessions.py. Same /proc discipline: comm and
//! cmdline are read directly — the python version replaced `pgrep -x`
//! with this after measuring an 18 ms fork per call, twice a second.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn projects_dir() -> PathBuf {
    crate::home().join(".claude/projects")
}

// Claude Code's own plumbing — listing it painted a phantom everlasting
// background job in every picker (python comment, behaviour kept)
const CLAUDE_INFRA: [&str; 2] = ["daemon", "bg-pty-host"];
const CLAUDE_SUBCOMMANDS: [&str; 16] = [
    "agents", "auth", "auto-mode", "daemon", "doctor", "gateway", "install", "mcp", "plugin",
    "plugins", "project", "setup-token", "ultrareview", "update", "upgrade", "config",
];

fn argv(pid: &str) -> Vec<String> {
    fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            b.split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn pids_named(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir("/proc") else {
        return out;
    };
    for e in dir.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !n.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(comm) = fs::read_to_string(format!("/proc/{n}/comm")) {
            if comm.trim_end() == name {
                out.push(n);
            }
        }
    }
    out
}

fn subcommand(a: &[String]) -> Option<&str> {
    // first non-flag token after argv[0] (python _subcommand, simplified
    // to the same effect: it only has to catch one-shot CLI calls)
    a.iter()
        .skip(1)
        .find(|t| !t.starts_with('-'))
        .map(|s| s.as_str())
}

#[derive(Clone, Debug)]
pub struct ClaudeProc {
    pub pid: i32,
    pub cwd: String,
    pub interactive: bool,
    pub tty: String,
    /// session id when argv names it (`--resume <sid>` / `--session-id <sid>`)
    /// — a fact, which beats every heuristic
    pub argv_sid: String,
}

pub fn claude_procs() -> Vec<ClaudeProc> {
    let mut out = Vec::new();
    for p in pids_named("claude") {
        let a = argv(&p);
        if a.iter()
            .skip(1)
            .take(2)
            .any(|t| CLAUDE_INFRA.contains(&t.as_str()))
            || a.iter().any(|t| t == "--bg-spare")
        {
            continue;
        }
        if let Some(sc) = subcommand(&a) {
            if CLAUDE_SUBCOMMANDS.contains(&sc) || sc == "bg-pty-host" || sc == "migrate-installer"
            {
                continue;
            }
        }
        let (Ok(cwd), Ok(tty)) = (
            fs::read_link(format!("/proc/{p}/cwd")),
            fs::read_link(format!("/proc/{p}/fd/0")),
        ) else {
            continue;
        };
        let tty = tty.display().to_string();
        let sid = a
            .windows(2)
            .find(|w| w[0] == "--resume" || w[0] == "--session-id")
            .map(|w| w[1].clone())
            .filter(|s| s.len() == 36)
            .unwrap_or_default();
        out.push(ClaudeProc {
            pid: p.parse().unwrap_or(0),
            cwd: cwd.display().to_string(),
            interactive: tty.starts_with("/dev/pts"),
            tty,
            argv_sid: sid,
        });
    }
    out
}

/// [(mtime, path)] newest first; subagent/workflow transcripts are not
/// resumable conversations and are skipped.
pub fn recent_transcripts(limit: usize) -> Vec<(f64, PathBuf)> {
    let mut files: Vec<(f64, PathBuf)> = Vec::new();
    if let Ok(dir) = fs::read_dir(projects_dir()) {
        for proj in dir.flatten() {
            let name = proj.file_name().to_string_lossy().to_string();
            if !proj.path().is_dir() || name.contains("-subagents-") {
                continue;
            }
            if let Ok(inner) = fs::read_dir(proj.path()) {
                for f in inner.flatten() {
                    let p = f.path();
                    if p.extension().is_some_and(|e| e == "jsonl") {
                        if let Ok(md) = f.metadata() {
                            let mt = md
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs_f64())
                                .unwrap_or(0.0);
                            files.push((mt, p));
                        }
                    }
                }
            }
        }
    }
    files.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    files.truncate(limit);
    files
}

/// (cwd, preview) from a transcript's first lines — python session_meta.
pub fn session_meta(path: &Path) -> (String, String) {
    let (mut cwd, mut preview) = (String::new(), String::new());
    // python stops at 40 LINES; reading the whole file would pull entire
    // multi-MB transcripts into memory on every poll. 128 KB covers 40
    // lines of any real transcript.
    use std::io::Read;
    let Ok(f) = fs::File::open(path) else {
        return (cwd, preview);
    };
    let mut raw = Vec::new();
    let _ = f.take(131072).read_to_end(&mut raw); // short reads handled
    let text = String::from_utf8_lossy(&raw).to_string();
    for (i, line) in text.lines().enumerate() {
        if i > 40 || (!cwd.is_empty() && !preview.is_empty()) {
            break;
        }
        let Ok(d) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if cwd.is_empty() {
            if let Some(c) = d.get("cwd").and_then(|v| v.as_str()) {
                cwd = c.to_string();
            }
        }
        if preview.is_empty() && d.get("type").and_then(|v| v.as_str()) == Some("user") {
            let m = d.get("message").and_then(|m| m.get("content"));
            let text = match m {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() && !t.starts_with('<') {
                preview = t.chars().take(48).collect();
            }
        }
    }
    (cwd, preview)
}

/// Claude's own name for a conversation — the NEWEST aiTitle wins, and
/// the tail is scanned first because that is where a fresh one lands.
pub fn session_title(path: &Path) -> String {
    const TAIL: u64 = 65536;
    let Ok(md) = fs::metadata(path) else {
        return String::new();
    };
    let read_titles = |text: &str, newest_first: bool| -> String {
        let lines: Vec<&str> = text.lines().collect();
        let iter: Box<dyn Iterator<Item = &&str>> = if newest_first {
            Box::new(lines.iter().rev())
        } else {
            Box::new(lines.iter())
        };
        for line in iter {
            if !line.contains("\"aiTitle\"") {
                continue;
            }
            if let Ok(d) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(t) = d.get("aiTitle").and_then(|v| v.as_str()) {
                    return t.trim().to_string();
                }
            }
        }
        String::new()
    };
    // tail first
    use std::io::{Read, Seek, SeekFrom};
    if let Ok(mut f) = fs::File::open(path) {
        let mut buf = String::new();
        if md.len() > TAIL {
            let _ = f.seek(SeekFrom::End(-(TAIL as i64)));
            let mut raw = Vec::new();
            if f.read_to_end(&mut raw).is_ok() {
                buf = String::from_utf8_lossy(&raw).to_string();
                // drop the partial first line
                if let Some(i) = buf.find('\n') {
                    buf = buf[i + 1..].to_string();
                }
            }
        } else if f.read_to_string(&mut buf).is_err() {
            return String::new();
        }
        let t = read_titles(&buf, true);
        if !t.is_empty() || md.len() <= TAIL {
            return t;
        }
    }
    // long session whose only title was written near the start. BOUNDED:
    // an unbounded read_to_string here once pulled a 48 MB transcript into
    // the heap and glibc kept the arena — measured as the office2d widget
    // sitting at 52 MB RSS, python-sized, for one read it did at startup.
    if let Ok(f) = fs::File::open(path) {
        let mut raw = Vec::new();
        let _ = f.take(262144).read_to_end(&mut raw);
        let head = String::from_utf8_lossy(&raw);
        // drop the possibly-truncated final line
        let head = head.rsplit_once('\n').map(|(h, _)| h).unwrap_or(&head);
        return read_titles(head, false);
    }
    String::new()
}

/// Last 32 hex chars — uuid-sans-dashes, layout-independent.
pub fn sid_key(text: &str) -> String {
    let hexed: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    hexed.chars().rev().take(32).collect::<String>().chars().rev().collect()
}

/// {sid_key(parent): active agent count} — an agent transcript modified
/// in the last max_age seconds counts as actively working. Both layouts:
/// nested (<proj>/<sid>/subagents/[workflows/<wf>/]agent-*.jsonl) and
/// flat encoded (-subagents- top-level dirs, older runs).
pub fn active_subagents(max_age: f64) -> HashMap<String, usize> {
    let t = now();
    let mut out: HashMap<String, usize> = HashMap::new();
    let mut bump = |sid: &str, p: &Path| {
        if let Ok(md) = fs::metadata(p) {
            if let Ok(age) = md.modified().map(|m| {
                t - m
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0)
            }) {
                if age < max_age {
                    *out.entry(sid_key(sid)).or_insert(0) += 1;
                }
            }
        }
    };
    let is_agent_jsonl = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("agent-") && n.ends_with(".jsonl"))
    };
    let Ok(dir) = fs::read_dir(projects_dir()) else {
        return out;
    };
    for proj in dir.flatten() {
        if !proj.path().is_dir() {
            continue;
        }
        let name = proj.file_name().to_string_lossy().to_string();
        if let Some(sid) = name.split("-subagents-").next().filter(|_| name.contains("-subagents-"))
        {
            if let Ok(inner) = fs::read_dir(proj.path()) {
                for f in inner.flatten() {
                    if is_agent_jsonl(&f.path()) {
                        bump(sid, &f.path());
                    }
                }
            }
            continue;
        }
        // nested layout: <proj>/<sid>/subagents/{,workflows/<wf>/}agent-*.jsonl
        let Ok(inner) = fs::read_dir(proj.path()) else {
            continue;
        };
        for siddir in inner.flatten() {
            let sub = siddir.path().join("subagents");
            if !sub.is_dir() {
                continue;
            }
            let sid = siddir.file_name().to_string_lossy().to_string();
            let mut stack = vec![sub];
            let mut depth = 0;
            while let Some(d) = stack.pop() {
                depth += 1;
                if depth > 200 {
                    break; // runaway guard, not expected
                }
                if let Ok(entries) = fs::read_dir(&d) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if is_agent_jsonl(&p) {
                            bump(&sid, &p);
                        }
                    }
                }
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct JobState {
    pub state: String,
    pub detail: String,
    pub name: String,
}

/// ~/.claude/jobs/<sid8>/state.json — an undocumented internal whose
/// shape has already drifted across cliVersions; every access best-effort.
pub fn job_state(sid: &str) -> JobState {
    let short: String = sid.chars().take(8).collect();
    let path = crate::home().join(format!(".claude/jobs/{short}/state.json"));
    let Ok(text) = fs::read_to_string(path) else {
        return JobState::default();
    };
    let Ok(d) = serde_json::from_str::<serde_json::Value>(&text) else {
        return JobState::default();
    };
    let s = |k: &str| {
        d.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    JobState {
        state: s("state"),
        detail: s("detail"),
        name: s("name"),
    }
}

pub fn ago(ts: f64) -> String {
    let d = (now() - ts).max(0.0) as u64;
    if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86400)
    }
}
