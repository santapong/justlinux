//! waybar status for ufw firewall (JSON) — port of fw-status.sh.

use crate::applets::av_status::service_active;
use crate::util;
use std::process::ExitCode;

/// grep -q '^ENABLED=yes' /etc/ufw/ufw.conf
pub fn ufw_enabled_in_conf() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .map(|t| t.lines().any(|l| l.starts_with("ENABLED=yes")))
        .unwrap_or(false)
}

pub fn run() -> ExitCode {
    if ufw_enabled_in_conf() && service_active("ufw") {
        println!(r#"{{"text":"󰕥","class":"on","tooltip":"Firewall (ufw): ACTIVE — click for options"}}"#);
    } else {
        println!(r#"{{"text":"󰕥","class":"off","tooltip":"Firewall (ufw): OFF — click for options"}}"#);
    }
    util::ok_exit()
}
