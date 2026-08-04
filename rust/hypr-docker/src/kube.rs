//! Kubernetes plumbing — kubectl with a HARD request timeout everywhere:
//! a stopped cluster (minikube down) answers with connection-refused in
//! milliseconds, but a firewalled one hangs 30 s, and this panel must
//! never freeze its UI thread on a dead apiserver.

use std::process::{Command, Stdio};

use crate::docker::LogSink;

const TIMEOUT: &str = "--request-timeout=3s";

fn kubectl_lines(args: &[&str]) -> (Vec<String>, String) {
    match Command::new("kubectl").args(args).output() {
        Ok(o) => (
            String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect(),
            String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("").to_string(),
        ),
        Err(e) => (Vec::new(), e.to_string()),
    }
}

pub fn available() -> bool {
    Command::new("kubectl")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn current_context() -> String {
    kubectl_lines(&["config", "current-context"]).0.first().cloned().unwrap_or_default()
}

pub fn contexts() -> Vec<String> {
    kubectl_lines(&["config", "get-contexts", "-o", "name"]).0
}

pub fn use_context(name: &str, sink: LogSink) {
    let name = name.to_string();
    std::thread::spawn(move || {
        let ok = Command::new("kubectl")
            .args(["config", "use-context", &name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        sink.push_status(if ok {
            format!("✔ context → {name}")
        } else {
            format!("✘ could not switch context to {name}")
        });
    });
}

#[derive(Clone, Default, PartialEq)]
pub struct Pod {
    pub ns: String,
    pub name: String,
    pub ready: String,  // "1/1"
    pub status: String, // Running | Pending | CrashLoopBackOff | …
    pub restarts: String,
    pub age: String,
}

/// (pods, error-line). An unreachable cluster returns the error, and the
/// caller renders it — a blank pane is a bug (fleet rule).
pub fn pods() -> (Vec<Pod>, String) {
    let (lines, err) = kubectl_lines(&["get", "pods", "-A", "--no-headers", TIMEOUT]);
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 6 {
            continue;
        }
        out.push(Pod {
            ns: f[0].into(),
            name: f[1].into(),
            ready: f[2].into(),
            status: f[3].into(),
            restarts: f[4].into(),
            age: f[f.len() - 1].into(),
        });
    }
    (out, err)
}

pub fn follow_logs(ns: &str, pod: &str, sink: LogSink) {
    let mut cmd = Command::new("kubectl");
    cmd.args(["logs", "-f", "--tail", "200", "-n", ns, pod, TIMEOUT]);
    crate::docker::stream_into_pub(cmd, sink);
}

pub fn delete_pod(ns: String, pod: String, sink: LogSink) {
    std::thread::spawn(move || {
        let out = Command::new("kubectl")
            .args(["delete", "pod", "-n", &ns, &pod, TIMEOUT])
            .output();
        let msg = match out {
            Ok(o) if o.status.success() => format!("✔ deleted pod {ns}/{pod}"),
            Ok(o) => format!("✘ delete {ns}/{pod}: {}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => format!("✘ delete {ns}/{pod}: {e}"),
        };
        sink.push_status(msg);
    });
}

/// A shell inside the pod, in its own kitty window (docker::connect twin).
pub fn exec_shell(ns: &str, pod: &str) {
    let _ = Command::new("setsid")
        .args([
            "kitty",
            "--class",
            "hyprdockerexec",
            "--title",
            &format!("{pod} — shell"),
            "-e",
            "sh",
            "-c",
            &format!("kubectl exec -it -n {ns} {pod} -- bash 2>/dev/null || kubectl exec -it -n {ns} {pod} -- sh"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Everything inside a pod — containers, images, mounts, conditions,
/// events — streamed into the pane (the "see inside" view).
pub fn describe(ns: &str, pod: &str, sink: LogSink) {
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(["describe", "pod", "-n", ns, pod, TIMEOUT]);
    crate::docker::stream_into_pub(cmd, sink);
}
