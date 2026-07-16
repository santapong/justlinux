//! Toggle the top bar (ALT+B) — port of bar-toggle.sh.
//!
//! If the auto-hide daemon is running, let IT do the toggle so its internal
//! state stays in sync (it pins the bar open / resumes hiding). Zero
//! subprocesses: /proc scan + kill() replace pgrep/pkill.

use crate::proc;
use crate::util;
use std::process::ExitCode;

pub fn run() -> ExitCode {
    let daemons = proc::pids_with_cmdline("waybar-autohide.sh");
    if !daemons.is_empty() {
        for pid in daemons {
            proc::kill(pid, libc::SIGUSR1);
        }
    } else {
        proc::pkill_comm("waybar", libc::SIGUSR1);
    }
    util::ok_exit()
}
