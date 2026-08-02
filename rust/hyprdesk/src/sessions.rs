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

// ---------------- session_rows (python claudesessions.session_rows) ----

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub kind: String, // "run" | "bg" | "past"
    pub label: String,
    pub sid: String,
    pub cwd: String,
    pub dir: String,
    pub pid: i32,
    pub tty: String,
    pub addr: String,
    pub title: String,
    pub detail: String,
}

fn boot_time() -> Option<f64> {
    let text = fs::read_to_string("/proc/stat").ok()?;
    text.lines()
        .find(|l| l.starts_with("btime "))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

fn clk_tck() -> f64 {
    extern "C" {
        fn sysconf(name: i32) -> i64;
    }
    let hz = unsafe { sysconf(2) }; // _SC_CLK_TCK
    if hz > 0 {
        hz as f64
    } else {
        100.0
    }
}

/// Wall-clock start of a pid, or None (python _proc_start).
fn proc_start(pid: i32) -> Option<f64> {
    let boot = boot_time()?;
    let data = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &data[data.rfind(')')? + 2..];
    let ticks: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some(boot + ticks / clk_tck())
}

/// Epoch of an ISO-8601 "YYYY-MM-DDTHH:MM:SS(.fff)?(Z|±..)" — enough for
/// transcript timestamps, which Claude writes in UTC with a Z.
fn iso_epoch(ts: &str) -> Option<f64> {
    let b = ts.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let (y, mo, d) = (num(&ts[0..4])?, num(&ts[5..7])?, num(&ts[8..10])?);
    let (h, mi, s) = (num(&ts[11..13])?, num(&ts[14..16])?, num(&ts[17..19])?);
    // days since epoch (civil_from_days inverse, Howard Hinnant's algorithm)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + h * 3600 + mi * 60 + s) as f64)
}

/// When a transcript's conversation began (python _tx_start, uncached —
/// callers here poll at studio cadence, not per-frame).
fn tx_start(path: &Path) -> Option<f64> {
    use std::io::Read;
    let f = fs::File::open(path).ok()?;
    let mut raw = Vec::new();
    let _ = f.take(65536).read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw);
    for (i, line) in text.lines().enumerate() {
        if i > 12 {
            break;
        }
        let Ok(d) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = d.get("timestamp").and_then(|v| v.as_str()) {
            return iso_epoch(ts);
        }
    }
    None
}

/// Path of a known session id (python _tx_for_sid).
pub fn tx_for_sid(sid: &str, txs: &[(f64, PathBuf)]) -> String {
    for (_mt, f) in txs {
        if f.file_stem().and_then(|s| s.to_str()) == Some(sid) {
            return f.display().to_string();
        }
    }
    if let Ok(dir) = fs::read_dir(projects_dir()) {
        for proj in dir.flatten() {
            let p = proj.path().join(format!("{sid}.jsonl"));
            if p.is_file() {
                return p.display().to_string();
            }
        }
    }
    String::new()
}

/// Which conversation a process is really on (python transcript_for):
/// argv wins; else pair unclaimed cwd-matching transcripts by TIME with
/// the 120 s adoption bound — waiting beats adopting.
pub fn transcript_for(
    proc_: &ClaudeProc,
    taken: &std::collections::HashSet<String>,
    txs: &[(f64, PathBuf)],
) -> String {
    if !proc_.argv_sid.is_empty() {
        return tx_for_sid(&proc_.argv_sid, txs);
    }
    let started = proc_start(proc_.pid);
    let (mut best, mut best_gap): (String, Option<f64>) = (String::new(), None);
    for (mt, f) in txs {
        let key = f.display().to_string();
        if taken.contains(&key) {
            continue;
        }
        let (meta_cwd, _) = session_meta(f);
        if meta_cwd != proc_.cwd {
            continue;
        }
        let Some(st) = started else {
            return key; // no clock: newest-first, as before
        };
        if *mt < st - 5.0 {
            continue; // untouched since the process began
        }
        let gap = (tx_start(f).unwrap_or(*mt) - st).abs();
        if best_gap.is_none_or(|g| gap < g) {
            best = key;
            best_gap = Some(gap);
        }
    }
    match best_gap {
        Some(g) if g <= 120.0 => best,
        _ => String::new(),
    }
}

/// [(pid, sid, cwd)] daemon-hosted sessions (bg-pty-host workers) that
/// outlive their terminals (python daemon_hosted).
pub fn daemon_hosted() -> Vec<(i32, String, String)> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir("/proc") else {
        return out;
    };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(raw) = fs::read(format!("/proc/{name}/cmdline")) else {
            continue;
        };
        let argv: Vec<String> = raw
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        if !argv.iter().any(|a| a.contains("bg-pty-host")) {
            continue;
        }
        let Some(i) = argv.iter().position(|a| a == "--session-id") else {
            continue; // pre-warmed spare, not a session
        };
        let Some(sid) = argv.get(i + 1) else { continue };
        let Ok(cwd) = fs::read_link(format!("/proc/{name}/cwd")) else {
            continue;
        };
        out.push((
            name.parse().unwrap_or(0),
            sid.clone(),
            cwd.display().to_string(),
        ));
    }
    out
}

/// Hyprland window address owning a pid, walking ppid up (python
/// window_of_pid). Studio-internal sessions dead-end at the tmux server.
pub fn window_of_pid(pid: i32) -> Option<String> {
    let out = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    let clients: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut by_pid: HashMap<i64, String> = HashMap::new();
    for c in clients.as_array()? {
        if let (Some(p), Some(a)) = (
            c.get("pid").and_then(|v| v.as_i64()),
            c.get("address").and_then(|v| v.as_str()),
        ) {
            by_pid.insert(p, a.to_string());
        }
    }
    let mut cur = pid as i64;
    for _ in 0..15 {
        if let Some(addr) = by_pid.get(&cur) {
            return Some(addr.clone());
        }
        let stat = fs::read_to_string(format!("/proc/{cur}/stat")).ok()?;
        let rest = &stat[stat.rfind(')')? + 2..];
        cur = rest.split_whitespace().nth(1)?.parse().ok()?;
        if cur <= 1 {
            break;
        }
    }
    None
}

const BG_DETAIL_CAP: usize = 40;

/// What a background job is actually DOING (python _bg_detail).
fn bg_detail(sid: &str, where_: &str, fallback: &str) -> String {
    let js = job_state(sid);
    let state = js.state.trim().to_string();
    let mut detail = js.detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if detail.to_lowercase() == state.to_lowercase() {
        detail.clear(); // "stopped · stopped" says it twice
    }
    if detail.chars().count() > BG_DETAIL_CAP {
        detail = detail.chars().take(BG_DETAIL_CAP - 1).collect::<String>();
        detail = format!("{}…", detail.trim_end());
    }
    let head = if state.is_empty() { "background".to_string() } else { state };
    let tail = if detail.is_empty() { fallback.to_string() } else { detail };
    format!("󰑮 {head} · {tail} — in {where_}")
}

/// Flat entry list for pickers: running, bg jobs, then resumable
/// transcripts (python session_rows). Same ordering, same fields.
pub fn session_rows() -> Vec<Row> {
    let home = crate::home().display().to_string();
    let nice = |p: &str| -> String {
        let s = if p.is_empty() { "?" } else { p };
        let s = s.replace(&home, "~");
        if s.is_empty() { "~".into() } else { s }
    };
    let mut rows = Vec::new();
    let txs = recent_transcripts(25);
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in claude_procs() {
        let tx = transcript_for(&p, &taken, &txs);
        if !tx.is_empty() {
            taken.insert(tx.clone());
        }
        let title = if tx.is_empty() {
            String::new()
        } else {
            session_title(Path::new(&tx))
        };
        let sid = Path::new(&tx)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let where_ = nice(&p.cwd);
        if p.interactive {
            rows.push(Row {
                kind: "run".into(),
                label: if title.is_empty() { where_.clone() } else { title },
                addr: window_of_pid(p.pid).unwrap_or_default(),
                cwd: p.cwd.clone(),
                pid: p.pid,
                tty: p.tty.clone(),
                sid,
                dir: where_.clone(),
                detail: format!("🟢 running in {where_} — Enter focuses its terminal"),
                ..Default::default()
            });
        } else {
            rows.push(Row {
                kind: "bg".into(),
                label: if title.is_empty() { where_.clone() } else { title },
                cwd: p.cwd.clone(),
                pid: p.pid,
                sid: sid.clone(),
                dir: where_.clone(),
                detail: bg_detail(&sid, &where_, "view on claude.ai"),
                ..Default::default()
            });
        }
    }
    for (pid, sid, cwd) in daemon_hosted() {
        let where_ = nice(&cwd);
        let tx = tx_for_sid(&sid, &txs);
        if !tx.is_empty() {
            taken.insert(tx.clone());
        }
        let title = if tx.is_empty() {
            String::new()
        } else {
            session_title(Path::new(&tx))
        };
        rows.push(Row {
            kind: "bg".into(),
            label: if title.is_empty() { where_.clone() } else { title },
            cwd,
            pid,
            sid: sid.clone(),
            dir: where_.clone(),
            detail: bg_detail(&sid, &where_, "outlives its terminal"),
            ..Default::default()
        });
    }
    for (mt, f) in &txs {
        let (cwd, preview) = session_meta(f);
        if preview.is_empty() {
            continue; // empty/aborted session: skip
        }
        let label = nice(&cwd);
        rows.push(Row {
            kind: "past".into(),
            label: label.clone(),
            sid: f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            cwd: if cwd.is_empty() { home.clone() } else { cwd },
            dir: label,
            title: session_title(f),
            detail: format!("{} — {preview}", ago(*mt)),
            ..Default::default()
        });
    }
    rows
}
