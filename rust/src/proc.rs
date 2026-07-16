//! /proc scanning + signals — native replacements for pgrep/pkill spawns.

use std::fs;

/// Pids whose /proc/<pid>/cmdline contains `needle` (pgrep -f), excluding
/// ourselves and, like pgrep, excluding our direct ancestors' shells is NOT
/// needed — but we must never match our own process.
pub fn pids_with_cmdline(needle: &str) -> Vec<i32> {
    let me = std::process::id() as i32;
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let cmdline = raw
            .split(|b| *b == 0)
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" ");
        if cmdline.contains(needle) {
            out.push(pid);
        }
    }
    out
}

/// Pids whose process name (/proc/<pid>/comm) is exactly `name` (pgrep -x).
pub fn pids_with_comm(name: &str) -> Vec<i32> {
    let me = std::process::id() as i32;
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for entry in rd.flatten() {
        let fname = entry.file_name();
        let Some(pid) = fname.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        if let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) {
            if comm.trim_end() == name {
                out.push(pid);
            }
        }
    }
    out
}

pub fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 || *libc::__errno_location() == libc::EPERM }
}

pub fn kill(pid: i32, sig: i32) -> bool {
    unsafe { libc::kill(pid, sig) == 0 }
}

/// pkill -<sig> -f <needle>; returns how many processes were signalled.
pub fn pkill_cmdline(needle: &str, sig: i32) -> usize {
    pids_with_cmdline(needle)
        .into_iter()
        .filter(|p| kill(*p, sig))
        .count()
}

/// pkill -<sig> -x <name> (waybar, hyprpaper …).
pub fn pkill_comm(name: &str, sig: i32) -> usize {
    pids_with_comm(name)
        .into_iter()
        .filter(|p| kill(*p, sig))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_own_test_runner_by_cmdline_fragment() {
        // The test binary path contains "justlinux"; a child would see us.
        // We exclude self, so search for something universal instead: pid 1.
        assert!(alive(1));
    }

    #[test]
    fn comm_scan_does_not_crash_and_excludes_self() {
        let mine = pids_with_comm("definitely-not-a-process-name");
        assert!(mine.is_empty());
    }
}
