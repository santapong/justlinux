//! widgets.conf — flat `key = value`, keys `\w+`, values `\S+`, exactly
//! theme.py's grammar. Reads are uncached ON PURPOSE: every read picks up
//! edits, which is what makes SIGUSR1 reposition work.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn widgets_conf() -> PathBuf {
    crate::home().join(".config/conky/widgets.conf")
}

pub fn conf_get(key: &str, default: &str) -> String {
    if let Ok(text) = fs::read_to_string(widgets_conf()) {
        for line in text.lines() {
            let mut parts = line.splitn(2, '=');
            let k = parts.next().unwrap_or("").trim();
            if k == key && k.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if let Some(v) = parts.next() {
                    let v = v.trim().split_whitespace().next().unwrap_or("");
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
    }
    default.to_string()
}

/// One-key write honouring the confwrite discipline: flock on the sidecar
/// lock, previous content one-deep in .undo, temp + rename so no reader
/// ever sees a half-written file. Order and comments preserved.
pub fn conf_set(key: &str, value: &str) {
    use fs2::FileExt;
    let conf = widgets_conf();
    let lock_path = conf.with_file_name("widgets.conf.lock");
    let Ok(lock) = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
    else {
        return;
    };
    let _ = lock.lock_exclusive(); // best effort, like the python OSError path
    let old = fs::read_to_string(&conf).unwrap_or_default();
    let _ = fs::write(conf.with_file_name("widgets.conf.undo"), &old);
    let mut out = String::new();
    let mut seen = false;
    for line in old.lines() {
        let k = line.splitn(2, '=').next().unwrap_or("").trim();
        if k == key {
            out.push_str(&format!("{key} = {value}\n"));
            seen = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !seen {
        out.push_str(&format!("{key} = {value}\n"));
    }
    let tmp = conf.with_file_name("widgets.conf.tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        if f.write_all(out.as_bytes()).is_ok() {
            let _ = fs::rename(&tmp, &conf);
        }
    }
    let _ = fs2::FileExt::unlock(&lock);
}
