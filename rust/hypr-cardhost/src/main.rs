//! hypr-cardhost — the desktop widget host: one process, N glass cards.
//! Rung 3b's host step; bin/hypr-cardhost (python) is the spec.
//!
//! THIS COMMIT: the multi-surface architecture, proven on the real fleet —
//! every enabled instance becomes its own LayerSurface (namespace
//! hypr-card-<id>, so the blur layerrules keep matching), placed by the
//! same precedence the python uses: free fx/fy pixels, else grid col/row
//! cells, else the legacy pos/x/y anchors. Cards render their template
//! with preview fields; clock/calgrid rows tick live on a 1 s timer.
//! Input regions are EMPTY — cards never eat desktop clicks.
//!
//! NOT YET (next commits, in the plan): stats/netgraph/cmd sources with
//! generation tokens, the ctl socket (reload / reload-theme / ping),
//! action-click cards, monitor hotplug + reserved-inset rechecks.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::{channel, EventLoop};
use calloop_wayland_source::WaylandSource;
use hyprdesk::cardspec::{self, Template};
use hyprdesk::rows;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const TICK: Duration = Duration::from_secs(1); // clock cadence, python parity

fn already_running() -> bool {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if me.is_empty() {
        return false;
    }
    let out = Command::new("pgrep")
        .args(["-xf", &me])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mine = std::process::id();
    out.split_whitespace()
        .filter_map(|p| p.parse::<u32>().ok())
        .any(|p| p != mine)
}

fn hypr_json(cmd: &str) -> serde_json::Value {
    Command::new("hyprctl")
        .args(["-j", cmd])
        .output()
        .ok()
        .and_then(|o| serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Deterministic home for cards without a saved <id>_mon: the monitor
/// hosting workspace 1, else the top-left-most. NEVER the focused one —
/// a focus-dependent fallback teleports cards on every reload (python).
fn default_monitor_name() -> String {
    if let Some(ws) = hypr_json("workspaces").as_array() {
        for w in ws {
            if w.get("id").and_then(|v| v.as_i64()) == Some(1) {
                if let Some(m) = w.get("monitor").and_then(|v| v.as_str()) {
                    return m.to_string();
                }
            }
        }
    }
    let mut best: Option<(i64, i64, String)> = None;
    if let Some(ms) = hypr_json("monitors").as_array() {
        for m in ms {
            let x = m.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let y = m.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if best.as_ref().is_none_or(|(by, bx, _)| (y, x) < (*by, *bx)) {
                best = Some((y, x, name));
            }
        }
    }
    best.map(|(_, _, n)| n).unwrap_or_default()
}

/// {connector: (w, h, reserved l,t,r,b)} — grid cells live in the WORKAREA.
fn monitor_geo() -> HashMap<String, (f64, f64, (f64, f64, f64, f64))> {
    let mut out = HashMap::new();
    if let Some(ms) = hypr_json("monitors").as_array() {
        for m in ms {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let scale = m.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0).max(0.1);
            let w = m.get("width").and_then(|v| v.as_f64()).unwrap_or(1600.0) / scale;
            let h = m.get("height").and_then(|v| v.as_f64()).unwrap_or(900.0) / scale;
            let r = m
                .get("reserved")
                .and_then(|v| v.as_array())
                .map(|a| {
                    let g = |i: usize| a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    (g(0), g(1), g(2), g(3))
                })
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            out.insert(name, (w, h, r));
        }
    }
    out
}

struct Card {
    id: String,
    template: Template,
    layer: LayerSurface,
    width: u32,  // card surface width (content + 2*PAD)
    height: u32, // card surface height
    scale: f32,
    params: HashMap<String, String>,
    fields: HashMap<String, String>,
    has_clock: bool, // clock or calgrid rows → the 1 s tick redraws it
    configured: bool,
    dirty: bool,
}

fn rows_have(t: &Template, kinds: &[&str]) -> bool {
    t.rows.iter().any(|r| {
        kinds.contains(&r.get("type").and_then(|v| v.as_str()).unwrap_or(""))
    })
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    cards: Vec<Card>,
    pal: hyprdesk::Palette,
    text: hyprdesk::draw::Text,
    series: HashMap<String, Vec<f64>>,
    last_minute: String,
    exit: bool,
}

impl App {
    fn draw_card(&mut self, i: usize) {
        let card = &self.cards[i];
        if !card.configured {
            return;
        }
        let (w, h) = (card.width.max(1), card.height.max(1));
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        let mut pix = tiny_skia::Pixmap::new(w, h).unwrap();
        let ctx = rows::Ctx {
            pal: &self.pal,
            params: &card.params,
            fields: &card.fields,
            series: &self.series,
            width: (card.template.width as f32) * card.scale,
            scale: card.scale,
            measure: false,
            text: &self.text,
            now: None,
        };
        rows::render_card(&mut pix, &card.template.rows, &ctx);
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let card = &mut self.cards[i];
        let surface = card.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(surface).ok();
        card.layer.commit();
        card.dirty = false;
    }
}

enum Sig {
    Theme,
    Quit,
}

#[allow(clippy::too_many_arguments)]
fn spawn_cards(
    qh: &QueueHandle<App>,
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    output_state: &OutputState,
    pal: &hyprdesk::Palette,
    text: &hyprdesk::draw::Text,
) -> Vec<Card> {
    let c = hyprdesk::conf_all();
    let templates = cardspec::load_templates();
    let geo = monitor_geo();
    let primary = default_monitor_name();
    let outputs: HashMap<String, wl_output::WlOutput> = output_state
        .outputs()
        .filter_map(|o| output_state.info(&o).and_then(|i| i.name).map(|n| (n, o)))
        .collect();

    let mut cards = Vec::new();
    for (inst_id, tname) in cardspec::instances(&c) {
        if c.get(&inst_id).map(|v| v == "off").unwrap_or(false) {
            continue;
        }
        // per-card guard: one broken template must never keep the fleet
        // down — the python renders an error card; this step SKIPS with a
        // stderr line (the error card needs the sources commit's plumbing)
        let Some(Ok(t)) = templates.get(&tname) else {
            eprintln!("hypr-cardhost: {inst_id}: template {tname:?} missing/broken — skipped");
            continue;
        };
        let mon_name = c
            .get(&format!("{inst_id}_mon"))
            .cloned()
            .unwrap_or_else(|| primary.clone());
        let Some(output) = outputs.get(&mon_name).or_else(|| outputs.get(&primary)) else {
            continue;
        };
        let (mw, mh, res) = geo
            .get(&mon_name)
            .copied()
            .unwrap_or((1600.0, 900.0, (0.0, 0.0, 0.0, 0.0)));
        let work_w = mw - res.0 - res.2;
        let work_h = mh - res.1 - res.3;

        let scale = c
            .get(&format!("{inst_id}_scale"))
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.5, 2.0);
        let params = cardspec::instance_params(&c, &inst_id, t);
        let fields = t.preview_fields.clone();

        // measure BEFORE mapping — placement clamps depend on card size
        let content_w = (t.width as f32) * scale;
        let mctx = rows::Ctx {
            pal,
            params: &params,
            fields: &fields,
            series: &HashMap::new(),
            width: content_w,
            scale,
            measure: true,
            text,
            now: None,
        };
        let content_h = rows::measure(&t.rows, &mctx);
        let card_w = (content_w + 2.0 * rows::PAD).round() as u32;
        let card_h = (content_h + 2.0 * rows::PAD).round() as u32;

        // ---- placement precedence: free pixels > grid cell > legacy ----
        let fx = c.get(&format!("{inst_id}_fx")).and_then(|v| v.parse::<f64>().ok());
        let fy = c.get(&format!("{inst_id}_fy")).and_then(|v| v.parse::<f64>().ok());
        let cell = (
            c.get(&format!("{inst_id}_col")).and_then(|v| v.parse::<i32>().ok()),
            c.get(&format!("{inst_id}_row")).and_then(|v| v.parse::<i32>().ok()),
        );
        let surface = compositor.create_surface(qh);
        if let Ok(region) = Region::new(compositor) {
            // display-only: a card must NEVER eat desktop clicks
            surface.set_input_region(Some(region.wl_region()));
        }
        let layer = layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Bottom,
            Some(format!("hypr-card-{inst_id}")),
            Some(output),
        );
        layer.set_size(card_w, card_h);
        layer.set_exclusive_zone(0);
        if let (Some(fx), Some(fy)) = (fx, fy) {
            // free placement: workarea pixels straight onto margins,
            // clamped so a stale pair can't strand a card off screen
            layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            layer.set_margin(
                fy.clamp(0.0, (work_h - 1.0).max(0.0)) as i32,
                0,
                0,
                fx.clamp(0.0, (work_w - 1.0).max(0.0)) as i32,
            );
        } else if let (Some(col), Some(row)) = cell {
            let (cs, rs) = hyprdesk::grid::span(work_w, work_h, card_w as f64, card_h as f64);
            let (ccol, crow) = hyprdesk::grid::clamp(work_w, work_h, col, row, cs, rs);
            let (x, y) = hyprdesk::grid::origin(work_w, work_h, ccol, crow);
            layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            layer.set_margin(y, 0, 0, x);
        } else {
            // legacy pos/x/y anchors (default 24,44, python parity)
            let (pos, x, y) = hyprdesk::placement(
                &inst_id,
                hyprdesk::Pos::parse(&t.default_pos).unwrap_or(hyprdesk::Pos::TopLeft),
                24,
                44,
            );
            use hyprdesk::Pos::*;
            let anchor = match pos {
                TopLeft => Anchor::TOP | Anchor::LEFT,
                TopMiddle => Anchor::TOP,
                TopRight => Anchor::TOP | Anchor::RIGHT,
                MiddleRight => Anchor::RIGHT,
                BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
                BottomMiddle => Anchor::BOTTOM,
                BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
                MiddleLeft => Anchor::LEFT,
            };
            layer.set_anchor(anchor);
            layer.set_margin(y, x, y, x);
        }
        layer.commit();

        let has_clock = rows_have(t, &["clock", "calgrid"]);
        cards.push(Card {
            id: inst_id,
            template: t.clone(),
            layer,
            width: card_w,
            height: card_h,
            scale,
            params,
            fields,
            has_clock,
            configured: false,
            dirty: true,
        });
    }
    cards
}

fn main() {
    if already_running() {
        eprintln!("hypr-cardhost: another instance is running — exiting");
        return;
    }
    let conn = Connection::connect_to_env().expect("no wayland display");
    let (globals, mut event_queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh: QueueHandle<App> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new(4096, &shm).expect("shm pool"); // grows on demand

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        pool,
        cards: Vec::new(),
        pal: hyprdesk::colors(),
        text: hyprdesk::draw::Text::load(),
        series: HashMap::new(),
        last_minute: String::new(),
        exit: false,
    };
    // one roundtrip so output NAMES exist before cards pick monitors
    let _ = event_queue.roundtrip(&mut app);
    app.cards = spawn_cards(
        &qh,
        &app.compositor,
        &layer_shell,
        &app.output_state,
        &app.pal,
        &app.text,
    );
    eprintln!("hypr-cardhost: {} card(s) spawned", app.cards.len());

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");

    let (tx, rx) = channel::channel::<Sig>();
    std::thread::spawn(move || {
        use signal_hook::consts::{SIGINT, SIGTERM, SIGUSR2};
        let mut sigs = signal_hook::iterator::Signals::new([SIGUSR2, SIGTERM, SIGINT]).unwrap();
        for s in sigs.forever() {
            let _ = tx.send(if s == SIGUSR2 { Sig::Theme } else { Sig::Quit });
        }
    });
    event_loop
        .handle()
        .insert_source(rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(sig) = ev {
                match sig {
                    Sig::Theme => {
                        // live recolor, the ctl reload-theme semantics
                        app.pal = hyprdesk::colors();
                        for c in app.cards.iter_mut() {
                            c.dirty = true;
                        }
                    }
                    Sig::Quit => app.exit = true,
                }
            }
        })
        .expect("signal source");

    // the minute ticker drives clock redraws and the calgrid rollover —
    // 1 s cadence, redraw only when the MINUTE changed (python parity)
    event_loop
        .handle()
        .insert_source(Timer::from_duration(TICK), |_, _, app: &mut App| {
            let now = unsafe {
                let t = libc_time(std::ptr::null_mut());
                let mut tm: Tm = std::mem::zeroed();
                localtime_r(&t, &mut tm);
                format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
            };
            if now != app.last_minute {
                app.last_minute = now;
                for c in app.cards.iter_mut() {
                    if c.has_clock {
                        c.dirty = true;
                    }
                }
            }
            TimeoutAction::ToDuration(TICK)
        })
        .expect("timer");

    loop {
        event_loop
            .dispatch(Some(TICK), &mut app)
            .expect("dispatch");
        for i in 0..app.cards.len() {
            if app.cards[i].dirty && app.cards[i].configured {
                self_draw(&mut app, i);
            }
        }
        if app.exit {
            return;
        }
    }
}

fn self_draw(app: &mut App, i: usize) {
    app.draw_card(i);
}

// libc localtime for the minute ticker (same shim rows.rs uses privately)
#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}
extern "C" {
    #[link_name = "time"]
    fn libc_time(t: *mut i64) -> i64;
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
}

// ---------------- sctk boilerplate ----------------
impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // ONE card closing must not exit the host — drop that card only
        self.cards
            .retain(|c| c.layer.wl_surface() != layer.wl_surface());
        if self.cards.is_empty() {
            self.exit = true;
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        // find WHICH card this configure belongs to — the whole point of
        // the multi-surface step
        if let Some(i) = self
            .cards
            .iter()
            .position(|c| c.layer.wl_surface() == layer.wl_surface())
        {
            self.cards[i].configured = true;
            self.draw_card(i);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_shm!(App);
delegate_registry!(App);
