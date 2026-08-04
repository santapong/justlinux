//! Docker plumbing — the `docker` CLI is the API surface, matching the
//! fleet's subprocess style. Streams (logs, pull) run in threads and
//! deliver lines into a shared ring with a generation token, so a
//! superseded stream can never write into the pane it lost.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const LOG_CAP: usize = 2000;

#[derive(Clone, Default, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,  // running | exited | restarting | paused | created
    pub status: String, // human line ("Up 2 hours", "Exited (0) 3 days ago")
    pub cpu: String,    // from docker stats, running only
    pub mem: String,
    pub project: String, // com.docker.compose.project ("" = standalone)
    pub project_dir: String, // …project.working_dir, for compose verbs
    pub compose_file: String, // …project.config_files (first), if it still exists
    pub service: String, // com.docker.compose.service — the short in-project name
}

#[derive(Clone, Default, PartialEq)]
pub struct Image {
    pub repo: String,
    pub tag: String,
    pub size: String,
    pub id: String,
}

fn docker_lines(args: &[&str]) -> Vec<String> {
    Command::new("docker")
        .args(args)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn ps_all() -> Vec<Container> {
    let mut out = Vec::new();
    for line in docker_lines(&["ps", "-a", "--format", "{{json .}}"]) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let labels = s("Labels");
        let label = |key: &str| -> String {
            labels
                .split(',')
                .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
                .unwrap_or("")
                .to_string()
        };
        out.push(Container {
            id: s("ID"),
            name: s("Names"),
            image: s("Image"),
            state: s("State"),
            status: s("Status"),
            cpu: String::new(),
            mem: String::new(),
            project: label("com.docker.compose.project"),
            project_dir: label("com.docker.compose.project.working_dir"),
            compose_file: label("com.docker.compose.project.config_files")
                .split(',')
                .next()
                .filter(|f| std::path::Path::new(f).is_file())
                .unwrap_or("")
                .to_string(),
            service: label("com.docker.compose.service"),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One `docker stats --no-stream` for ALL running containers.
pub fn stats() -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for line in docker_lines(&["stats", "--no-stream", "--format", "{{json .}}"]) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mem = s("MemUsage").split('/').next().unwrap_or("").trim().to_string();
        out.insert(s("ID"), (s("CPUPerc"), mem));
    }
    out
}

pub fn images() -> Vec<Image> {
    let mut out = Vec::new();
    for line in docker_lines(&["images", "--format", "{{json .}}"]) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        if s("Repository") == "<none>" {
            continue; // dangling layers are noise in a control panel
        }
        out.push(Image {
            repo: s("Repository"),
            tag: s("Tag"),
            size: s("Size"),
            id: s("ID"),
        });
    }
    out.sort_by(|a, b| a.repo.cmp(&b.repo));
    out
}

/// start/stop/restart/rm/rmi — fire and report. Blocking calls run in a
/// thread; the result line lands in the log ring like any other stream.
pub fn action(verb: &'static str, target: String, sink: LogSink) {
    std::thread::spawn(move || {
        let out = Command::new("docker").args([verb, &target]).output();
        let msg = match out {
            Ok(o) if o.status.success() => format!("✔ docker {verb} {target}"),
            Ok(o) => format!(
                "✘ docker {verb} {target}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => format!("✘ docker {verb} {target}: {e}"),
        };
        sink.push_status(msg);
    });
}

/// The shared line ring the logs pane renders. `gen` is bumped whenever
/// the selection changes; late lines from a dead stream are dropped.
#[derive(Clone)]
pub struct LogSink {
    pub lines: Arc<Mutex<VecDeque<String>>>,
    pub gen: Arc<AtomicU64>,
    my_gen: u64,
}

impl LogSink {
    pub fn new() -> LogSink {
        LogSink {
            lines: Arc::new(Mutex::new(VecDeque::new())),
            gen: Arc::new(AtomicU64::new(0)),
            my_gen: 0,
        }
    }
    /// A sink bound to the CURRENT generation — clears the ring.
    pub fn rebind(&mut self) -> LogSink {
        let g = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.lines.lock().unwrap().clear();
        LogSink {
            lines: self.lines.clone(),
            gen: self.gen.clone(),
            my_gen: g,
        }
    }
    pub fn push(&self, line: String) {
        if self.gen.load(Ordering::SeqCst) != self.my_gen {
            return; // superseded stream: never write into the pane it lost
        }
        let mut l = self.lines.lock().unwrap();
        if l.len() >= LOG_CAP {
            l.pop_front();
        }
        l.push_back(line);
    }
    /// Status lines (action results) always land, whatever the gen.
    pub fn push_status(&self, line: String) {
        let mut l = self.lines.lock().unwrap();
        if l.len() >= LOG_CAP {
            l.pop_front();
        }
        l.push_back(line);
    }
}

pub fn stream_into(mut cmd: Command, sink: LogSink) {
    std::thread::spawn(move || {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let Ok(mut child) = cmd.spawn() else {
            sink.push("✘ failed to spawn docker".into());
            return;
        };
        let so = child.stdout.take().unwrap();
        let se = child.stderr.take().unwrap();
        let s2 = sink.clone();
        let t = std::thread::spawn(move || {
            for line in BufReader::new(se).lines().map_while(Result::ok) {
                s2.push(line);
            }
        });
        for line in BufReader::new(so).lines().map_while(Result::ok) {
            if sink.gen.load(Ordering::SeqCst) != sink.my_gen() {
                let _ = child.kill(); // pane moved on: stop tailing
                break;
            }
            sink.push(line);
        }
        let _ = t.join();
        let _ = child.wait();
    });
}

impl LogSink {
    fn my_gen(&self) -> u64 {
        self.my_gen
    }
}

/// Follow a container's logs into the ring.
pub fn follow_logs(id: &str, sink: LogSink) {
    let mut cmd = Command::new("docker");
    cmd.args(["logs", "-f", "--tail", "300", id]);
    stream_into(cmd, sink);
}

/// Pull an image, progress lines into the ring.
pub fn pull(image: &str, sink: LogSink) {
    sink.push_status(format!("⇣ docker pull {image} …"));
    let mut cmd = Command::new("docker");
    cmd.args(["pull", image]);
    stream_into(cmd, sink);
}

/// Connect: a shell inside the container, in its own kitty window —
/// docker exec needs a real tty and the panel must not give up its own.
pub fn connect(id: &str, name: &str) {
    let _ = Command::new("setsid")
        .args([
            "kitty",
            "--class",
            "hyprdockerexec",
            "--title",
            &format!("{name} — shell"),
            "-e",
            "sh",
            "-c",
            &format!("docker exec -it {id} bash 2>/dev/null || docker exec -it {id} sh"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn daemon_version() -> String {
    docker_lines(&["version", "--format", "{{.Server.Version}}"])
        .first()
        .cloned()
        .unwrap_or_default()
}

/// compose verbs run against the project's recorded working_dir; a
/// project whose dir vanished degrades to per-container start/stop.
pub fn compose_action(verb: &'static str, file: String, project: String, sink: LogSink) {
    std::thread::spawn(move || {
        let mut args: Vec<&str> = vec!["compose", "-f", &file, "-p", &project];
        let extra: Vec<&str> = match verb {
            "up" => vec!["up", "-d"],
            v => vec![v],
        };
        args.extend(extra);
        let out = Command::new("docker").args(&args).output();
        let msg = match out {
            Ok(o) if o.status.success() => format!("✔ compose {verb} {project}"),
            Ok(o) => format!(
                "✘ compose {verb} {project}: {}",
                String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").trim()
            ),
            Err(e) => format!("✘ compose {verb} {project}: {e}"),
        };
        sink.push_status(msg);
    });
}

/// Pull every image a compose project references, progress streamed.
pub fn compose_pull(file: &str, project: &str, sink: LogSink) {
    sink.push_status(format!("⇣ compose pull {project} …"));
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f", file, "-p", project, "pull"]);
    stream_into(cmd, sink);
}

/// Merged, service-prefixed logs for a whole compose project.
pub fn compose_logs(file: &str, project: &str, sink: LogSink) {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-f", file, "-p", project, "logs", "-f", "--tail", "200"]);
    stream_into(cmd, sink);
}

/// public alias for sibling modules (kube) — same stream semantics.
pub fn stream_into_pub(cmd: Command, sink: LogSink) {
    stream_into(cmd, sink);
}

/// "Exited (0) 2 hours ago" → ("exited (0)", "2h"); "Up 3 seconds" →
/// ("up 3s", ""). The design's status column wants short, aligned facts.
pub fn short_status(status: &str) -> (String, String) {
    let s = status.trim();
    let age = |txt: &str| -> String {
        // last "N unit ago" run, compressed
        let words: Vec<&str> = txt.split_whitespace().collect();
        for w in words.windows(3) {
            if w[2] == "ago" {
                let n = w[0];
                let u = match w[1].trim_end_matches('s') {
                    "second" => "s",
                    "minute" => "m",
                    "hour" => "h",
                    "day" => "d",
                    "week" => "w",
                    "month" => "mo",
                    "year" => "y",
                    other => other,
                };
                return format!("{n}{u}");
            }
        }
        String::new()
    };
    if let Some(rest) = s.strip_prefix("Exited ") {
        let code = rest.split(')').next().map(|c| format!("exited {c})")).unwrap_or_default();
        return (code, age(s));
    }
    if s.starts_with("Up ") {
        let mut it = s.split_whitespace();
        let (_, n, unit) = (it.next(), it.next().unwrap_or(""), it.next().unwrap_or(""));
        let u = match unit.trim_end_matches('s') {
            "second" => "s",
            "minute" => "m",
            "hour" => "h",
            "day" => "d",
            "week" => "w",
            "month" => "mo",
            other => other,
        };
        return (format!("up {n}{u}"), String::new());
    }
    if s.starts_with("Created") {
        return ("created".into(), "—".into());
    }
    (s.to_lowercase().chars().take(16).collect(), age(s))
}

/// "500MB" / "1.2GB" → bytes, best effort, for repo-group sums.
pub fn size_bytes(sz: &str) -> f64 {
    let t = sz.trim();
    let num: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let v: f64 = num.parse().unwrap_or(0.0);
    let unit = t[num.len()..].trim();
    match unit.to_uppercase().as_str() {
        "KB" | "KIB" => v * 1e3,
        "MB" | "MIB" => v * 1e6,
        "GB" | "GIB" => v * 1e9,
        "TB" => v * 1e12,
        _ => v,
    }
}

pub fn human_gb(bytes: f64) -> String {
    if bytes >= 1e9 {
        format!("{:.1} GB", bytes / 1e9)
    } else {
        format!("{:.0} MB", bytes / 1e6)
    }
}
