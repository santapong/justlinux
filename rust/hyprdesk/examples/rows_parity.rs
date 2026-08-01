//! measure() parity per REAL template (heights drive grid placement), and
//! a rendered PNG per template for visual acceptance.
use std::collections::HashMap;
use std::process::Command;

fn main() {
    // python: measure every template at width=template.width, scale 1
    let py = Command::new("python3").args(["-c", r#"
import sys, json, cairo; sys.path.insert(0,'/home/santapong/.local/lib')
from hyprdesk import cardspec, rows, colors
pal = colors()
out = {}
for name, t in sorted(cardspec.load_templates().items()):
    if not isinstance(t, cardspec.Template): continue
    ctx = {"pal": pal, "params": {k: p["default"] for k, p in t.params.items()},
           "fields": dict(t.preview_fields), "series": {}, "width": t.width,
           "scale": 1.0}
    out[name] = rows.measure(t.rows, ctx)
print(json.dumps(out, sort_keys=True))
"#]).output().unwrap();
    if !py.status.success() {
        eprintln!("python failed: {}", String::from_utf8_lossy(&py.stderr));
        std::process::exit(2);
    }
    let pyv: HashMap<String, f64> =
        serde_json::from_str(String::from_utf8_lossy(&py.stdout).trim()).unwrap();

    let pal = hyprdesk::colors();
    let text = hyprdesk::draw::Text::load();
    let empty_series = HashMap::new();
    let mut bad = 0;
    let tpls = hyprdesk::cardspec::load_templates();
    let mut names: Vec<_> = tpls.keys().cloned().collect();
    names.sort();
    for name in &names {
        let Ok(t) = &tpls[name] else { continue };
        let params: HashMap<String, String> = t
            .params
            .iter()
            .map(|(k, p)| (k.clone(), p.default.clone()))
            .collect();
        let ctx = hyprdesk::rows::Ctx {
            pal: &pal,
            params: &params,
            fields: &t.preview_fields,
            series: &empty_series,
            width: t.width as f32,
            scale: 1.0,
            measure: false,
            text: &text,
            now: None,
        };
        let h = hyprdesk::rows::measure(&t.rows, &ctx) as f64;
        let want = pyv.get(name).copied().unwrap_or(-1.0);
        let ok = (h - want).abs() < 0.01;
        println!("  {:12} python={want:8.2}  rust={h:8.2}  {}", name, if ok { "ok" } else { "MISMATCH" });
        if !ok {
            bad += 1;
        }
        // render the full card to a PNG for the visual pass
        let card_w = t.width as u32 + 24;
        let card_h = (h as u32) + 24 + 4;
        let mut pix = tiny_skia::Pixmap::new(card_w.max(1), card_h.max(1)).unwrap();
        hyprdesk::rows::render_card(&mut pix, &t.rows, &ctx);
        let _ = pix.save_png(format!("/tmp/rowspar/rust-{name}.png"));
    }
    if bad > 0 {
        println!("{bad} template(s) MISMATCHED");
        std::process::exit(1);
    }
    println!("measure parity: {} templates identical", names.len());
}
