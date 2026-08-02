//! Sweep the rust grid against the python grid — every function, every
//! cell, across the real monitor geometries plus adversarial sizes.
//! Any mismatch prints and exits 1.
use std::process::Command;

fn main() {
    // (workarea_w, workarea_h) — this box's three heads with waybar's
    // 30px reserve, plus awkward sizes that produce .5 rounding cases
    let geos = [(1600.0, 870.0), (1600.0, 900.0), (1920.0, 1050.0),
                (1366.0, 738.0), (2560.0, 1410.0), (1000.0, 500.0)];
    // one python line per statement — a rust `\` continuation eats the
    // next line's leading spaces, which ARE python's block structure
    let mut py_prog = String::from(
        "import sys; sys.path.insert(0,'/home/santapong/.local/lib')\nfrom hyprdesk import grid\nout=[]\n");
    for (w, h) in geos {
        py_prog.push_str(&format!("w,h={w},{h}\n"));
        py_prog.push_str("for col in range(0,14):\n");
        py_prog.push_str(" for row in range(0,8):\n");
        py_prog.push_str("  out.append(grid.origin(w,h,col,row))\n");
        py_prog.push_str("  out.append(grid.cell_at(w,h,col*97.3,row*61.7))\n");
        py_prog.push_str("for px in (60,150,300,451,777):\n");
        py_prog.push_str(" for py_ in (40,120,260,431):\n");
        py_prog.push_str("  out.append(grid.span(w,h,px,py_))\n");
        py_prog.push_str("  out.append(grid.clamp(w,h,px%15-1,py_%9-1,2,2))\n");
    }
    py_prog.push_str("print(';'.join(f'{a},{b}' for a,b in out))");
    let out = Command::new("python3").args(["-c", &py_prog]).output().unwrap();
    if !out.status.success() {
        eprintln!("python failed:\n{}", String::from_utf8_lossy(&out.stderr));
        std::process::exit(2);
    }
    let py: Vec<(i32, i32)> = String::from_utf8_lossy(&out.stdout)
        .trim()
        .split(';')
        .map(|p| {
            let (a, b) = p.split_once(',').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        })
        .collect();

    let mut rs: Vec<(i32, i32)> = Vec::new();
    for (w, h) in geos {
        for col in 0..14 {
            for row in 0..8 {
                rs.push(hyprdesk::grid::origin(w, h, col, row));
                rs.push(hyprdesk::grid::cell_at(w, h, col as f64 * 97.3, row as f64 * 61.7));
            }
        }
        for px in [60.0, 150.0, 300.0, 451.0, 777.0] {
            for py_ in [40.0, 120.0, 260.0, 431.0] {
                rs.push(hyprdesk::grid::span(w, h, px, py_));
                rs.push(hyprdesk::grid::clamp(
                    w, h, (px as i32) % 15 - 1, (py_ as i32) % 9 - 1, 2, 2,
                ));
            }
        }
    }
    assert_eq!(py.len(), rs.len(), "case-count mismatch");
    let bad: Vec<usize> = (0..py.len()).filter(|&i| py[i] != rs[i]).collect();
    if bad.is_empty() {
        println!("grid parity: {} cases, all identical", py.len());
    } else {
        for i in bad.iter().take(10) {
            println!("MISMATCH case {i}: python {:?} rust {:?}", py[*i], rs[*i]);
        }
        println!("grid parity: {}/{} MISMATCHED", bad.len(), py.len());
        std::process::exit(1);
    }
}
