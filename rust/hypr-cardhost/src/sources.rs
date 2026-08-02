//! Data sources, ported from the python host: the stats and netgraph
//! builtins read /proc directly; script sources run detached in their own
//! process group with a hard timeout and a byte cap, delivering through a
//! channel with a GENERATION token — a slow fetch that lands after a newer
//! one must not overwrite fresher data with stale fields.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

pub const SERIES_LEN: usize = 60;
const SCRIPT_TIMEOUT_S: u64 = 15;
const MAX_BUF: u64 = 256 * 1024; // a runaway source can't exhaust memory

pub fn human_bytes(n: f64, suffix: &str) -> String {
    let mut n = n;
    for unit in ["", "Ki", "Mi", "Gi", "Ti"] {
        if n.abs() < 1024.0 {
            return if unit.is_empty() {
                format!("{n:.0}{suffix}")
            } else {
                format!("{n:.1}{unit}{suffix}")
            };
        }
        n /= 1024.0;
    }
    format!("{n:.1}Pi{suffix}")
}

/// First interface with a default route, else first non-lo that's up.
pub fn pick_iface() -> String {
    if let Ok(text) = fs::read_to_string("/proc/net/route") {
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() > 1 && f[1] == "00000000" {
                return f[0].to_string();
            }
        }
    }
    if let Ok(dir) = fs::read_dir("/sys/class/net") {
        let mut names: Vec<String> = dir
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        for n in names {
            if n != "lo" {
                if let Ok(st) = fs::read_to_string(format!("/sys/class/net/{n}/operstate")) {
                    if st.trim() == "up" {
                        return n;
                    }
                }
            }
        }
    }
    "wlan0".into()
}

fn net_counters(iface: &str) -> Option<(u64, u64)> {
    let text = fs::read_to_string("/proc/net/dev").ok()?;
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix(&format!("{iface}:")) {
            let f: Vec<&str> = rest.split_whitespace().collect();
            return Some((f.first()?.parse().ok()?, f.get(8)?.parse().ok()?));
        }
    }
    None
}

fn coretemp_path() -> Option<std::path::PathBuf> {
    let mut hwmons: Vec<_> = fs::read_dir("/sys/class/hwmon").ok()?.flatten().collect();
    hwmons.sort_by_key(|e| e.file_name());
    for p in hwmons {
        if fs::read_to_string(p.path().join("name"))
            .map(|n| n.trim() == "coretemp")
            .unwrap_or(false)
        {
            return Some(p.path().join("temp1_input"));
        }
    }
    None
}

pub struct StatsSource {
    prev_cpu: Option<(u64, u64)>, // (total, idle)
    prev_net: Option<(u64, u64)>,
    prev_t: Option<Instant>,
    temp: Option<std::path::PathBuf>,
    iface: String,
    pub top: [(String, String); 2],
}

impl StatsSource {
    pub fn new() -> StatsSource {
        StatsSource {
            prev_cpu: None,
            prev_net: None,
            prev_t: None,
            temp: coretemp_path(),
            iface: pick_iface(),
            top: [("—".into(), "0".into()), ("—".into(), "0".into())],
        }
    }

    fn cpu_pct(&mut self) -> f64 {
        let Ok(text) = fs::read_to_string("/proc/stat") else {
            return 0.0;
        };
        let vals: Vec<u64> = text
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .skip(1)
            .take(8)
            .filter_map(|v| v.parse().ok())
            .collect();
        if vals.len() < 5 {
            return 0.0;
        }
        let idle = vals[3] + vals[4];
        let total: u64 = vals.iter().sum();
        let mut pct = 0.0;
        if let Some((pt, pi)) = self.prev_cpu {
            let dt = total.saturating_sub(pt);
            let di = idle.saturating_sub(pi);
            if dt > 0 {
                pct = 100.0 * (dt - di) as f64 / dt as f64;
            }
        }
        self.prev_cpu = Some((total, idle));
        pct
    }

    /// Synchronous field set (everything but `top`, which the caller
    /// refreshes from the async ps fetch this struct's `top` caches).
    pub fn fetch(&mut self) -> HashMap<String, String> {
        let mut f = HashMap::new();
        f.insert("cpu".into(), format!("{:.0}", self.cpu_pct()));
        f.insert(
            "cputemp".into(),
            self.temp
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|t| t.trim().parse::<i64>().ok())
                .map(|t| format!("{}", t / 1000))
                .unwrap_or_else(|| "—".into()),
        );
        // memory
        let mut mem_done = false;
        if let Ok(text) = fs::read_to_string("/proc/meminfo") {
            let mut mi: HashMap<&str, u64> = HashMap::new();
            for line in text.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    if let Some(n) = v.split_whitespace().next().and_then(|n| n.parse::<u64>().ok())
                    {
                        mi.insert(k, n * 1024);
                    }
                }
            }
            if let Some(&total) = mi.get("MemTotal") {
                let used = total.saturating_sub(*mi.get("MemAvailable").unwrap_or(&0));
                f.insert("mem_pct".into(), format!("{:.0}", 100.0 * used as f64 / total as f64));
                f.insert("mem_used".into(), human_bytes(used as f64, ""));
                f.insert("mem_max".into(), human_bytes(total as f64, ""));
                mem_done = true;
            }
        }
        if !mem_done {
            f.insert("mem_pct".into(), "0".into());
            f.insert("mem_used".into(), "—".into());
            f.insert("mem_max".into(), "—".into());
        }
        // disk (statvfs via libc)
        unsafe {
            let mut st: Statvfs = std::mem::zeroed();
            let path = std::ffi::CString::new("/").unwrap();
            if statvfs(path.as_ptr(), &mut st) == 0 && st.f_blocks > 0 {
                let total = st.f_blocks as f64 * st.f_frsize as f64;
                let free = st.f_bavail as f64 * st.f_frsize as f64;
                f.insert("disk_pct".into(), format!("{:.0}", 100.0 * (total - free) / total));
                f.insert("disk_used".into(), human_bytes(total - free, ""));
                f.insert("disk_size".into(), human_bytes(total, ""));
            } else {
                f.insert("disk_pct".into(), "0".into());
                f.insert("disk_used".into(), "—".into());
                f.insert("disk_size".into(), "—".into());
            }
        }
        // net rates
        let now = Instant::now();
        let cnt = net_counters(&self.iface);
        if let (Some(c), Some(p), Some(t)) = (cnt, self.prev_net, self.prev_t) {
            let dt = now.duration_since(t).as_secs_f64().max(0.001);
            f.insert(
                "net_down".into(),
                human_bytes(c.0.saturating_sub(p.0) as f64 / dt, "B/s"),
            );
            f.insert(
                "net_up".into(),
                human_bytes(c.1.saturating_sub(p.1) as f64 / dt, "B/s"),
            );
        } else {
            f.insert("net_down".into(), "—".into());
            f.insert("net_up".into(), "—".into());
        }
        if let Some(c) = cnt {
            self.prev_net = Some(c);
            self.prev_t = Some(now);
        }
        f.insert("iface".into(), self.iface.clone());
        f.insert("top1_name".into(), self.top[0].0.clone());
        f.insert("top1_cpu".into(), self.top[0].1.clone());
        f.insert("top2_name".into(), self.top[1].0.clone());
        f.insert("top2_cpu".into(), self.top[1].1.clone());
        f
    }

    /// Parse `ps -eo comm,pcpu --sort=-pcpu` output into the top cache.
    /// comm may contain spaces — split from the RIGHT (python comment).
    pub fn absorb_top(&mut self, text: &str) {
        let mut procs: Vec<(String, String)> = Vec::new();
        for line in text.lines().skip(1) {
            if let Some((name, cpu)) = line.trim().rsplit_once(char::is_whitespace) {
                procs.push((name.trim().chars().take(14).collect(), cpu.to_string()));
            }
            if procs.len() == 2 {
                break;
            }
        }
        if !procs.is_empty() {
            self.top[0] = procs[0].clone();
            self.top[1] = procs
                .get(1)
                .cloned()
                .unwrap_or(("—".into(), "0".into()));
        }
    }
}

pub struct NetgraphSource {
    iface: String,
    prev: Option<(u64, u64)>,
    prev_t: Option<Instant>,
    pub series_down: Vec<f64>,
    pub series_up: Vec<f64>,
}

impl NetgraphSource {
    pub fn new() -> NetgraphSource {
        NetgraphSource {
            iface: pick_iface(),
            prev: None,
            prev_t: None,
            series_down: Vec::new(),
            series_up: Vec::new(),
        }
    }

    pub fn fetch(&mut self) -> HashMap<String, String> {
        let now = Instant::now();
        let cnt = net_counters(&self.iface);
        let (mut down, mut up) = (0.0, 0.0);
        if let (Some(c), Some(p), Some(t)) = (cnt, self.prev, self.prev_t) {
            let dt = now.duration_since(t).as_secs_f64().max(0.001);
            down = c.0.saturating_sub(p.0) as f64 / dt;
            up = c.1.saturating_sub(p.1) as f64 / dt;
        }
        if let Some(c) = cnt {
            self.prev = Some(c);
            self.prev_t = Some(now);
        }
        for (s, v) in [(&mut self.series_down, down), (&mut self.series_up, up)] {
            s.push(v);
            let len = s.len();
            if len > SERIES_LEN {
                s.drain(..len - SERIES_LEN);
            }
        }
        let mut f = HashMap::new();
        f.insert("iface".into(), self.iface.clone());
        f.insert("down".into(), human_bytes(down, "B/s"));
        f.insert("up".into(), human_bytes(up, "B/s"));
        f
    }
}

// ---------------- async script fetches ----------------

/// (card_id, generation, Some(stdout) | None-on-failure)
pub type Delivery = (String, u64, Option<String>);

/// Run argv detached in its own process group; deliver via the channel.
/// Timeout kills the WHOLE group — a script's own children must not
/// outlive it (python killpg parity). Empty output counts as FAILURE at
/// the caller, not here (python: a broken param must not look fresh).
pub fn spawn_fetch(
    card_id: String,
    gen: u64,
    argv: Vec<String>,
    tx: calloop::channel::Sender<Delivery>,
) {
    std::thread::spawn(move || {
        let Some((prog, rest)) = argv.split_first() else {
            let _ = tx.send((card_id, gen, None));
            return;
        };
        let mut cmd = Command::new(prog);
        cmd.args(rest)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                libc_setsid();
                Ok(())
            });
        }
        let Ok(mut child) = cmd.spawn() else {
            let _ = tx.send((card_id, gen, None));
            return;
        };
        let pid = child.id() as i32;
        // hard-timeout killer for the whole group
        let killer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(SCRIPT_TIMEOUT_S));
            unsafe { libc_kill(-pid, 9) }; // SIGKILL the process group
        });
        let mut buf = String::new();
        let ok = child
            .stdout
            .take()
            .map(|out| out.take(MAX_BUF).read_to_string(&mut buf).is_ok())
            .unwrap_or(false);
        let status_ok = child.wait().map(|s| s.success()).unwrap_or(false);
        drop(killer); // detached; its eventual SIGKILL hits a dead group
        let _ = tx.send((
            card_id,
            gen,
            if ok && status_ok { Some(buf) } else { None },
        ));
    });
}

pub fn expand_home(cmd: &str) -> String {
    if let Some(rest) = cmd.strip_prefix("~/") {
        return crate::home_string() + "/" + rest;
    }
    cmd.to_string()
}

/// shlex-lite: whitespace split honouring single/double quotes — the
/// template cmds on this box use neither, but a future one must not
/// silently mis-split.
pub fn split_argv(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in cmd.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// libc shims
#[repr(C)]
struct Statvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    __spare: [i32; 6],
}
extern "C" {
    fn statvfs(path: *const i8, buf: *mut Statvfs) -> i32;
    #[link_name = "setsid"]
    fn libc_setsid() -> i32;
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
