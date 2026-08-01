//! The phone-style placement grid, ported from lib/hyprdesk/grid.py.
//!
//! THE HIGHEST-STAKES PARITY IN THE MIGRATION: hypr-arrange (python,
//! staying python) WRITES `<id>_col/_row` cells that a rust cardhost
//! READS. The two implementations compute pixels from those cells
//! independently — one rounding difference and every card jumps on the
//! next reload. `examples/grid_parity.rs` sweeps both against each other;
//! run it whenever either side changes.
//!
//! COORDINATE SPACE: workarea dimensions in, workarea-relative pixels out
//! (layer margins are workarea-relative). Python's round() is banker's
//! rounding — round-half-to-even — and Rust's f64::round() is
//! round-half-away-from-zero. They disagree on exact .5 values, which
//! cell_w fractions can and do produce. `pyround` below is the python
//! semantics, verified by the parity sweep.

pub const MARGIN: f64 = 24.0;
pub const BOTTOM_RESERVE: f64 = 68.0;

/// python round(): banker's rounding to the nearest even integer.
fn pyround(x: f64) -> i32 {
    let floor = x.floor();
    let frac = x - floor;
    if (frac - 0.5).abs() < 1e-9 {
        // exactly half: to even
        let f = floor as i64;
        (if f % 2 == 0 { f } else { f + 1 }) as i32
    } else {
        x.round() as i32
    }
}

pub struct Grid {
    pub cols: i32,
    pub rows: i32,
    pub cell_w: f64,
    pub cell_h: f64,
}

pub fn geometry(mon_w: f64, mon_h: f64) -> Grid {
    let cols = crate::conf_get("grid_cols", "12")
        .parse::<i32>()
        .unwrap_or(12)
        .clamp(2, 24);
    let rows = crate::conf_get("grid_rows", "6")
        .parse::<i32>()
        .unwrap_or(6)
        .clamp(2, 16);
    Grid {
        cols,
        rows,
        cell_w: (mon_w - 2.0 * MARGIN) / cols as f64,
        cell_h: (mon_h - MARGIN - BOTTOM_RESERVE) / rows as f64,
    }
}

pub fn origin(mon_w: f64, mon_h: f64, col: i32, row: i32) -> (i32, i32) {
    let g = geometry(mon_w, mon_h);
    let col = col.clamp(0, g.cols - 1);
    let row = row.clamp(0, g.rows - 1);
    (
        pyround(MARGIN + col as f64 * g.cell_w),
        pyround(MARGIN + row as f64 * g.cell_h),
    )
}

pub fn cell_at(mon_w: f64, mon_h: f64, x: f64, y: f64) -> (i32, i32) {
    let g = geometry(mon_w, mon_h);
    let col = pyround((x - MARGIN) / g.cell_w);
    let row = pyround((y - MARGIN) / g.cell_h);
    (col.clamp(0, g.cols - 1), row.clamp(0, g.rows - 1))
}

pub fn span(mon_w: f64, mon_h: f64, w: f64, h: f64) -> (i32, i32) {
    let g = geometry(mon_w, mon_h);
    (
        ((w / g.cell_w).ceil() as i32).max(1),
        ((h / g.cell_h).ceil() as i32).max(1),
    )
}

pub fn clamp(mon_w: f64, mon_h: f64, col: i32, row: i32, cs: i32, rs: i32) -> (i32, i32) {
    let g = geometry(mon_w, mon_h);
    (
        col.clamp(0, (g.cols - cs).max(0)),
        row.clamp(0, (g.rows - rs).max(0)),
    )
}
