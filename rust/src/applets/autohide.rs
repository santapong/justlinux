//! waybar-autohide — hide waybar; reveal when the mouse touches the top
//! edge, hide again on leave. Port of the python daemon (which itself
//! replaced a bash loop that forked `hyprctl` ~7×/second).
//!
//! Manual toggle while running: SIGUSR1 (bar-toggle sends it — ALT+B):
//! if hidden → show and PIN (auto-hide paused); if pinned → hide and
//! resume auto-hide. Stop the daemon with `pkill -f waybar-autohide`
//! (the bar stays visible).

use crate::{hypr, proc};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_millis(200);
const BAR_HEIGHT: i64 = 34; // a little more than the bar's 30px

static GOT_USR1: AtomicBool = AtomicBool::new(false);
static GOT_TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn on_usr1(_sig: libc::c_int) {
    GOT_USR1.store(true, Ordering::SeqCst);
}
extern "C" fn on_term(_sig: libc::c_int) {
    GOT_TERM.store(true, Ordering::SeqCst);
}

fn install_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_usr1 as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut());
        let mut st: libc::sigaction = std::mem::zeroed();
        st.sa_sigaction = on_term as *const () as usize;
        libc::sigemptyset(&mut st.sa_mask);
        libc::sigaction(libc::SIGTERM, &st, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &st, std::ptr::null_mut());
    }
}

struct State {
    visible: bool,
    pinned: bool,
    waybar: Option<i32>,
}

impl State {
    fn waybar_pid(&mut self) -> Option<i32> {
        if let Some(pid) = self.waybar {
            if proc::alive(pid) {
                return Some(pid);
            }
            self.waybar = None;
        }
        self.waybar = proc::pids_with_comm("waybar").into_iter().next();
        self.waybar
    }

    fn toggle_bar(&mut self) {
        if let Some(pid) = self.waybar_pid() {
            if !proc::kill(pid, libc::SIGUSR1) {
                self.waybar = None;
            }
        }
    }

    fn show(&mut self) {
        if !self.visible {
            self.toggle_bar();
            self.visible = true;
        }
    }

    fn hide(&mut self) {
        if self.visible {
            self.toggle_bar();
            self.visible = false;
        }
    }
}

pub fn run() -> ExitCode {
    install_handlers();
    let mut st = State { visible: true, pinned: false, waybar: None };
    st.hide();
    loop {
        if GOT_TERM.swap(false, Ordering::SeqCst) {
            st.show(); // when the daemon is stopped, leave the bar visible
            return ExitCode::SUCCESS;
        }
        if GOT_USR1.swap(false, Ordering::SeqCst) {
            // ALT+B: hidden -> show and pin; visible -> hide and resume
            if st.visible {
                st.hide();
                st.pinned = false;
            } else {
                st.show();
                st.pinned = true;
            }
        }
        if !st.pinned {
            if let Some((_x, y)) = hypr::cursor_pos() {
                if !st.visible && y <= 1 {
                    st.show();
                } else if st.visible && y > BAR_HEIGHT {
                    st.hide();
                }
            }
        }
        // nanosleep returns early on signals — flags get handled promptly
        std::thread::sleep(INTERVAL);
    }
}
