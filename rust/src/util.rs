//! Shared helpers: paths, detached spawning, notifications, time.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

/// $XDG_RUNTIME_DIR with the same /tmp fallback the python tools used.
pub fn xdg_runtime() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn local_bin(name: &str) -> PathBuf {
    home().join(".local/bin").join(name)
}

/// HYPRSETTINGS_DRYRUN=1 — record side effects instead of executing them
/// (same contract the python apps used for their test harness).
pub fn dry() -> bool {
    std::env::var("HYPRSETTINGS_DRYRUN").map(|v| v == "1").unwrap_or(false)
}

static ACTIONS: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

/// Under DRYRUN, remember the command; if HYPR_ACTIONS_FILE is set, also
/// append one line per action so external test harnesses can observe it.
pub fn record_action(cmd: &[&str]) {
    let owned: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
    if let Ok(path) = std::env::var("HYPR_ACTIONS_FILE") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{}", owned.join("\x1f"));
        }
    }
    ACTIONS.lock().unwrap().push(owned);
}

#[cfg(test)]
pub fn recorded_actions() -> Vec<Vec<String>> {
    ACTIONS.lock().unwrap().clone()
}

/// Start a detached external program in its own session, stdio to /dev/null
/// (the launch()/run_detached() of the python tools).
pub fn spawn_detached(cmd: &[&str]) {
    if dry() {
        record_action(cmd);
        return;
    }
    if cmd.is_empty() {
        return;
    }
    let mut c = Command::new(cmd[0]);
    c.args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // own session: survives our exit, no controlling terminal (start_new_session=True)
    unsafe {
        use std::os::unix::process::CommandExt;
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = c.spawn();
}

/// Run a command, wait for it, return (exit_ok, stdout).
pub fn run_capture(cmd: &[&str]) -> (bool, String) {
    match Command::new(cmd[0]).args(&cmd[1..]).output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(_) => (false, String::new()),
    }
}

/// Run a command with input piped to stdin, return its stdout (rofi -dmenu…).
pub fn run_with_input(cmd: &[&str], input: &str) -> Option<String> {
    let mut child = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn notify(args: &[&str]) {
    if dry() {
        let mut full = vec!["notify-send"];
        full.extend_from_slice(args);
        record_action(&full);
        return;
    }
    let _ = Command::new("notify-send")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Local time as (Y, M, D, h, m, s) via libc — no chrono dependency.
pub fn localtime_now() -> (i32, u32, u32, u32, u32, u32) {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (
            tm.tm_year + 1900,
            (tm.tm_mon + 1) as u32,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
}

/// Replace a leading $HOME with `~` for display (bash's ${f/#$HOME/~}).
pub fn tilde(path: &str) -> String {
    let h = home();
    let hs = h.to_string_lossy();
    match path.strip_prefix(hs.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// Small deterministic random index in 0..n (reads /dev/urandom).
pub fn random_index(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let mut buf = [0u8; 8];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .is_ok()
    {
        (u64::from_le_bytes(buf) % n as u64) as usize
    } else {
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0)
            % n as u64) as usize
    }
}

pub fn ok_exit() -> ExitCode {
    ExitCode::SUCCESS
}

pub fn fail_exit() -> ExitCode {
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_replaces_home_prefix_only() {
        let h = home();
        let p = format!("{}/Pictures/x.png", h.display());
        assert_eq!(tilde(&p), "~/Pictures/x.png");
        assert_eq!(tilde("/etc/passwd"), "/etc/passwd");
    }

    #[test]
    fn random_index_in_range() {
        for _ in 0..50 {
            assert!(random_index(7) < 7);
        }
        assert_eq!(random_index(1), 0);
        assert_eq!(random_index(0), 0);
    }
}
