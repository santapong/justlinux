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
        out.push(Container {
            id: s("ID"),
            name: s("Names"),
            image: s("Image"),
            state: s("State"),
            status: s("Status"),
            cpu: String::new(),
            mem: String::new(),
        });
    }
    // running first, then by name — the fleet's "live things lead" order
    out.sort_by(|a, b| {
        (a.state != "running")
            .cmp(&(b.state != "running"))
            .then(a.name.cmp(&b.name))
    });
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

fn stream_into(mut cmd: Command, sink: LogSink) {
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
