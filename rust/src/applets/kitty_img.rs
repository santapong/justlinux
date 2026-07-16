//! Kitty graphics protocol — just enough to draw PNG icons/thumbnails in a
//! grid (what textual-image did for the python launcher).
//!
//! We only ever display PNGs from our own cache, so transmission uses
//! t=f/f=100: the payload is the base64 of the FILE PATH and kitty reads
//! the file itself — no image decoding on our side at all. Each unique
//! file is transmitted once per app lifetime (a=t), then placed per frame
//! with cheap a=p placements; placements are cleared with a=d,d=a whenever
//! the visible set changes.

use base64::Engine;
use std::collections::HashMap;
use std::io::Write;

/// Draw images only inside kitty (same practical scope textual-image had
/// here — the launcher always runs in a kitty float). HYPR_NO_IMAGES=1
/// forces the glyph fallback, e.g. for benchmarks in a plain pty.
pub fn enabled() -> bool {
    if crate::util::dry() {
        return false;
    }
    if std::env::var("HYPR_NO_IMAGES").map(|v| v == "1").unwrap_or(false) {
        return false;
    }
    std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("TERM").map(|t| t.contains("kitty")).unwrap_or(false)
}

#[derive(Default)]
pub struct Canvas {
    ids: HashMap<String, u32>,
    next_id: u32,
    /// (image id, col, row, cols, rows) placements currently on screen
    last: Vec<(u32, u16, u16, u16, u16)>,
}

impl Canvas {
    pub fn new() -> Self {
        Canvas { ids: HashMap::new(), next_id: 1, last: Vec::new() }
    }

    fn ensure_transmitted(&mut self, out: &mut impl Write, path: &str) -> u32 {
        if let Some(id) = self.ids.get(path) {
            return *id;
        }
        let id = self.next_id;
        self.next_id += 1;
        let b64 = base64::engine::general_purpose::STANDARD.encode(path.as_bytes());
        // q=2: never send responses; t=f/f=100: kitty reads the PNG file
        let _ = write!(out, "\x1b_Gq=2,a=t,t=f,f=100,i={id};{b64}\x1b\\");
        self.ids.insert(path.to_string(), id);
        id
    }

    /// Replace all on-screen placements with `wanted` (path, col, row, cols,
    /// rows — all 0-based terminal cells). No-op if nothing changed.
    pub fn sync(&mut self, wanted: &[(String, u16, u16, u16, u16)]) {
        let mut out = std::io::stdout().lock();
        let desired: Vec<(u32, u16, u16, u16, u16)> = {
            let mut v = Vec::with_capacity(wanted.len());
            for (path, col, row, cols, rows) in wanted {
                let id = self.ensure_transmitted(&mut out, path);
                v.push((id, *col, *row, *cols, *rows));
            }
            v
        };
        if desired == self.last {
            let _ = out.flush();
            return;
        }
        // delete all placements (keep transmitted data), then re-place
        let _ = write!(out, "\x1b_Gq=2,a=d,d=a\x1b\\");
        for (id, col, row, cols, rows) in &desired {
            // save cursor, jump, place (C=1: don't move cursor), restore
            let _ = write!(
                out,
                "\x1b[s\x1b[{};{}H\x1b_Gq=2,a=p,i={id},p={},c={cols},r={rows},C=1\x1b\\\x1b[u",
                row + 1,
                col + 1,
                ((*row as u32) << 16) | *col as u32,
            );
        }
        let _ = out.flush();
        self.last = desired;
    }

    /// Remove every placement (leaving transmitted data cached in kitty).
    pub fn clear(&mut self) {
        if self.last.is_empty() {
            return;
        }
        let mut out = std::io::stdout().lock();
        let _ = write!(out, "\x1b_Gq=2,a=d,d=a\x1b\\");
        let _ = out.flush();
        self.last.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_per_path() {
        let mut c = Canvas::new();
        let mut sink = Vec::new();
        let a = c.ensure_transmitted(&mut sink, "/tmp/a.png");
        let b = c.ensure_transmitted(&mut sink, "/tmp/b.png");
        let a2 = c.ensure_transmitted(&mut sink, "/tmp/a.png");
        assert_eq!(a, a2);
        assert_ne!(a, b);
        // transmission escape emitted once per unique path
        let s = String::from_utf8_lossy(&sink);
        assert_eq!(s.matches("a=t,t=f,f=100").count(), 2);
        // payload is base64 of the path
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode("/tmp/a.png");
        assert!(s.contains(&b64));
    }
}
