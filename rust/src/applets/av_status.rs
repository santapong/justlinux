//! waybar status for clamav antivirus (JSON) — port of av-status.sh.

use crate::util;
use std::process::ExitCode;

pub fn service_active(name: &str) -> bool {
    util::run_capture(&["systemctl", "-q", "is-active", name]).0
}

pub fn run() -> ExitCode {
    if service_active("clamav-daemon") {
        println!(r#"{{"text":"󰃤","class":"on","tooltip":"Antivirus (ClamAV): daemon running — click to scan"}}"#);
    } else {
        println!(r#"{{"text":"󰃤","class":"off","tooltip":"Antivirus (ClamAV): daemon NOT running — click for options"}}"#);
    }
    util::ok_exit()
}
