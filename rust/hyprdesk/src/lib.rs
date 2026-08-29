//! The fleet contracts, in Rust — one crate every migrated tool shares.
//! Everything here parses the SAME files `lib/hyprdesk` (Python) parses:
//! the files are the interface (docs/rust-migration.md), so the two
//! implementations cannot drift apart without a visible symptom.

mod conf;
pub mod cardspec;
pub mod grid;
#[cfg(feature = "draw")]
pub mod draw;
#[cfg(feature = "draw")]
pub mod pixicons16;
#[cfg(feature = "draw")]
pub mod rows;
mod sessions;
mod theme;

pub use conf::{conf_all, conf_get, conf_set};
pub use sessions::{
    active_subagents, ago, claude_procs, codex_meta, daemon_hosted, tx_for_sid, WindowMap, codex_procs, codex_sid, codex_tx_for_sid,
    recent_codex_transcripts, job_state, recent_transcripts, session_meta,
    session_rows, session_title, sid_key, window_of_pid, ClaudeProc, JobState, Row,
};
pub use theme::{colors, Palette, Rgb};

use std::path::PathBuf;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

/// The 8-way widget position vocabulary (`<name>_pos` in widgets.conf).
/// Kept wayland-free so this crate carries no compositor dependency —
/// each binary maps these onto its toolkit's anchor flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pos {
    TopLeft,
    TopMiddle,
    TopRight,
    MiddleRight,
    BottomRight,
    BottomMiddle,
    BottomLeft,
    MiddleLeft,
}

impl Pos {
    pub fn parse(s: &str) -> Option<Pos> {
        Some(match s {
            "top_left" => Pos::TopLeft,
            "top_middle" => Pos::TopMiddle,
            "top_right" => Pos::TopRight,
            "middle_right" => Pos::MiddleRight,
            "bottom_right" => Pos::BottomRight,
            "bottom_middle" => Pos::BottomMiddle,
            "bottom_left" => Pos::BottomLeft,
            "middle_left" => Pos::MiddleLeft,
            _ => return None,
        })
    }
}

/// `<name>_pos/_x/_y` for a widget, with its defaults — the placement
/// half of the layer contract. `<name>_mon` is read by the caller (it
/// needs toolkit output handles to act on it).
pub fn placement(name: &str, default_pos: Pos, dx: i32, dy: i32) -> (Pos, i32, i32) {
    let pos = Pos::parse(&conf_get(&format!("{name}_pos"), "")).unwrap_or(default_pos);
    let x = conf_get(&format!("{name}_x"), "")
        .parse()
        .unwrap_or(dx);
    let y = conf_get(&format!("{name}_y"), "")
        .parse()
        .unwrap_or(dy);
    (pos, x, y)
}
