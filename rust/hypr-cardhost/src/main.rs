//! hypr-cardhost — the desktop widget host: one process, N glass cards.
//! bin/hypr-cardhost (python) is the spec; this is the COMPLETE port:
//! multi-surface architecture, builtin + script sources with generation
//! tokens, the ctl socket (--ctl ping|reload|reload-theme), action-click
//! cards, error cards, "(stale)" badges, monitor hotplug and the
//! reserved-inset recheck, the first-run toast.
//!
//! The founding rule carries over structurally: a bad card must never
//! kill its siblings — template errors render an error card, a closed
//! surface drops one card, a failed fetch degrades one card to stale.

mod sources;

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer};
use calloop::{channel, EventLoop};
use calloop_wayland_source::WaylandSource;
use hyprdesk::cardspec::{self, Template};
use hyprdesk::rows;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
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
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const TICK: Duration = Duration::from_secs(1);

pub fn home_string() -> String {
    hyprdesk::home().display().to_string()
}

fn ctl_sock_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
        .join("hyprcard.sock")
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
    width: u32,
    height: u32,
    scale: f32,
    params: HashMap<String, String>,
    fields: HashMap<String, String>,
    action: String,
    has_clock: bool,
    interval: u64,
    next_due: u64,     // tick when the next fetch fires (cmd sources)
    fetch_gen: u64,    // generation token — stale deliveries are dropped
    last_ok: Option<Instant>,
    error: Option<String>,
    configured: bool,
    dirty: bool,
}

impl Card {
    fn is_stale(&self) -> bool {
        if self.template.source_cmd.is_empty() {
            return false;
        }
        match self.last_ok {
            None => false,
            Some(t) => t.elapsed().as_secs() > 2 * self.template.interval,
        }
    }
}

fn rows_have(t: &Template, kinds: &[&str]) -> bool {
    t.rows
        .iter()
        .any(|r| kinds.contains(&r.get("type").and_then(|v| v.as_str()).unwrap_or("")))
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    cards: Vec<Card>,
    pal: hyprdesk::Palette,
    text: hyprdesk::draw::Text,
    stats: sources::StatsSource,
    net: sources::NetgraphSource,
    series: HashMap<String, Vec<f64>>,
    fetch_tx: channel::Sender<sources::Delivery>,
    tick: u64,
    last_minute: String,
    reserved: HashMap<String, (f64, f64, f64, f64)>,
    respawn_at: Option<u64>, // debounced respawn (hotplug / reload / reserve)
    pointer: Option<wl_pointer::WlPointer>,
    pointer_surface: Option<wl_surface::WlSurface>,
    exit: bool,
}

impl App {
    fn measure_card(&self, card: &Card) -> (u32, u32) {
        let content_w = (card.template.width as f32) * card.scale;
        let ctx = rows::Ctx {
            pal: &self.pal,
            params: &card.params,
            fields: &card.fields,
            series: &self.series,
            width: content_w,
            scale: card.scale,
            measure: true,
            text: &self.text,
            now: None,
        };
        let content_h = rows::measure(&card.template.rows, &ctx);
        (
            (content_w + 2.0 * rows::PAD).round() as u32,
            (content_h + 2.0 * rows::PAD).round() as u32,
        )
    }

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
        if let Some(err) = &card.error {
            // error card: glass + the problem, never silence
            rows::draw_glass(&mut pix, w as f32, h as f32, &self.pal);
            self.text.draw_weight(
                &mut pix,
                rows::PAD,
                26.0,
                12.0,
                self.pal.bad,
                &format!("⚠ {}", card.id),
                true,
            );
            let msg: String = err.chars().take(60).collect();
            self.text
                .draw(&mut pix, rows::PAD, 46.0, 10.0, self.pal.sub, &msg);
        } else {
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
            if card.is_stale() {
                let s = "(stale)";
                let adv = self.text.advance(9.0, false, s);
                self.text.draw(
                    &mut pix,
                    w as f32 - rows::PAD - adv,
                    h as f32 - 6.0,
                    9.0,
                    self.pal.sub,
                    s,
                );
            }
        }
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

    /// python update_fields, REPLACE-don't-merge: every source emits its
    /// complete set per run; merging leaves ghost repeat rows rendering
    /// stale data as fresh forever when a group count shrinks.
    fn update_fields(&mut self, i: usize, fields: Option<HashMap<String, String>>) {
        {
            let card = &mut self.cards[i];
            match fields {
                None => {
                    // preview fields must NOT mask a failing source
                    if card.last_ok.is_none() {
                        card.error = Some("source failed".into());
                    }
                }
                Some(f) => {
                    card.error = None;
                    if !f.is_empty() {
                        card.fields = card.template.preview_fields.clone();
                        card.fields.extend(f);
                    } // empty = "just redraw" (none-ticker): keep fields
                    card.last_ok = Some(Instant::now());
                }
            }
        }
        let (w, h) = self.measure_card(&self.cards[i]);
        let card = &mut self.cards[i];
        if h != card.height || w != card.width {
            card.width = w;
            card.height = h;
            card.layer.set_size(w, h);
        }
        card.dirty = true;
    }

    fn fire_cmd_fetch(&mut self, i: usize) {
        let card = &mut self.cards[i];
        let cmd = cardspec::subst(&card.template.source_cmd, &card.params, None);
        let argv = sources::split_argv(&sources::expand_home(&cmd));
        card.fetch_gen += 1;
        sources::spawn_fetch(card.id.clone(), card.fetch_gen, argv, self.fetch_tx.clone());
    }

    fn deliver(&mut self, (id, gen, text): sources::Delivery) {
        if id == "__ps" {
            if let Some(t) = text {
                self.stats.absorb_top(&t);
            }
            return;
        }
        let Some(i) = self.cards.iter().position(|c| c.id == id) else {
            return;
        };
        if self.cards[i].fetch_gen != gen {
            return; // superseded by a later fetch — never overwrite fresher
        }
        // empty output counts as FAILURE, not success-with-no-data
        let fields = text.and_then(|t| {
            let f = cardspec::parse_fields(&t);
            if f.is_empty() {
                None
            } else {
                Some(f)
            }
        });
        self.update_fields(i, fields);
    }

    fn respawn_soon(&mut self) {
        if self.respawn_at.is_none() {
            self.respawn_at = Some(self.tick + 1);
        }
    }

    fn on_ctl(&mut self, mut client: UnixStream) {
        let mut buf = [0u8; 256];
        let n = client.read(&mut buf).unwrap_or(0);
        let cmd = String::from_utf8_lossy(&buf[..n]).trim().to_string();
        let reply: &[u8] = match cmd.as_str() {
            "reload" => {
                self.respawn_soon();
                b"ok\n"
            }
            "reload-theme" => {
                self.pal = hyprdesk::colors();
                for c in self.cards.iter_mut() {
                    c.dirty = true;
                }
                b"ok\n"
            }
            "ping" | "" => b"ok\n",
            _ => b"err\n", // an unknown verb must not masquerade as success
        };
        let _ = client.write_all(reply);
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
    tick: u64,
) -> (Vec<Card>, HashMap<String, (f64, f64, f64, f64)>) {
    let mut c = hyprdesk::conf_all();
    let templates = cardspec::load_templates();
    // fold legacy global params into per-instance keys once (python)
    let migration = cardspec::migrate_legacy_params(&c, &templates);
    if !migration.is_empty() {
        for (k, v) in &migration {
            hyprdesk::conf_set(k, v);
        }
        c = hyprdesk::conf_all();
    }
    let geo = monitor_geo();
    let reserved: HashMap<String, (f64, f64, f64, f64)> =
        geo.iter().map(|(k, v)| (k.clone(), v.2)).collect();
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
        // missing/broken template → ERROR CARD, not silence (python)
        let (template, error): (Template, Option<String>) = match templates.get(&tname) {
            Some(Ok(t)) => (t.clone(), None),
            other => {
                let msg = match other {
                    Some(Err(e)) => format!("template {tname:?}: {}", e.0),
                    _ => format!("template {tname:?}: not found"),
                };
                (error_template(&inst_id), Some(msg))
            }
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
        let params = cardspec::instance_params(&c, &inst_id, &template);
        let fields = template.preview_fields.clone();
        let action = c
            .get(&format!("{inst_id}_action"))
            .cloned()
            .unwrap_or_else(|| template.action.clone())
            .trim()
            .to_string();

        let content_w = (template.width as f32) * scale;
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
        let content_h = rows::measure(&template.rows, &mctx);
        let card_w = (content_w + 2.0 * rows::PAD).round() as u32;
        let card_h = (content_h + 2.0 * rows::PAD).round() as u32;

        let fx = c.get(&format!("{inst_id}_fx")).and_then(|v| v.parse::<f64>().ok());
        let fy = c.get(&format!("{inst_id}_fy")).and_then(|v| v.parse::<f64>().ok());
        let cell = (
            c.get(&format!("{inst_id}_col")).and_then(|v| v.parse::<i32>().ok()),
            c.get(&format!("{inst_id}_row")).and_then(|v| v.parse::<i32>().ok()),
        );
        let surface = compositor.create_surface(qh);
        if action.is_empty() {
            // display-only: a card must NEVER eat desktop clicks
            if let Ok(region) = Region::new(compositor) {
                surface.set_input_region(Some(region.wl_region()));
            }
        } // an action card claims its whole rectangle: default input region

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
            let (pos, x, y) = hyprdesk::placement(
                &inst_id,
                hyprdesk::Pos::parse(&template.default_pos).unwrap_or(hyprdesk::Pos::TopLeft),
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

        let has_clock = rows_have(&template, &["clock", "calgrid"]);
        let interval = template.interval;
        // stagger first fetches so 10 scripts don't fork at once at login
        let next_due = tick + 1 + (cards.len() as u64 % 5);
        cards.push(Card {
            id: inst_id,
            template,
            layer,
            width: card_w,
            height: card_h,
            scale,
            params,
            fields,
            action,
            has_clock,
            interval,
            next_due,
            fetch_gen: 0,
            last_ok: None,
            error,
            configured: false,
            dirty: true,
        });
    }
    (cards, reserved)
}

fn error_template(inst_id: &str) -> Template {
    let toml_src = format!(
        "[template]\nname = \"{inst_id}\"\nlabel = \"{inst_id}\"\n\n[[rows]]\ntype = \"title\"\ntext = \"⚠ {inst_id}\"\n\n[[rows]]\ntype = \"text\"\nink = \"sub\"\ntext = \"broken template\"\n"
    );
    let data: toml::Table = toml::from_str(&toml_src).unwrap();
    Template::parse(inst_id, &data, None).unwrap()
}

fn ctl_client(cmd: &str) -> i32 {
    match UnixStream::connect(ctl_sock_path()) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = s.write_all(cmd.as_bytes());
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf).unwrap_or(0);
            let reply = String::from_utf8_lossy(&buf[..n]).trim().to_string();
            println!("{}", if reply.is_empty() { "no reply" } else { &reply });
            i32::from(reply != "ok")
        }
        Err(e) => {
            eprintln!("cardhost not reachable: {e}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--ctl") {
        std::process::exit(ctl_client(args.get(i + 1).map(|s| s.as_str()).unwrap_or("ping")));
    }
    // a live host owns the socket — EXIT rather than run a second,
    // socketless duplicate fleet (concurrent-restart race, python parity)
    if UnixStream::connect(ctl_sock_path()).is_ok() {
        eprintln!("hypr-cardhost: another instance is running — exiting");
        return;
    }
    let _ = std::fs::remove_file(ctl_sock_path());
    let listener = UnixListener::bind(ctl_sock_path()).ok();
    if let Some(l) = &listener {
        let _ = l.set_nonblocking(true);
    }

    let conn = Connection::connect_to_env().expect("no wayland display");
    let (globals, mut event_queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh: QueueHandle<App> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new(4096, &shm).expect("shm pool");
    let (fetch_tx, fetch_rx) = channel::channel::<sources::Delivery>();

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        shm,
        pool,
        cards: Vec::new(),
        pal: hyprdesk::colors(),
        text: hyprdesk::draw::Text::load(),
        stats: sources::StatsSource::new(),
        net: sources::NetgraphSource::new(),
        series: HashMap::new(),
        fetch_tx,
        tick: 0,
        last_minute: String::new(),
        reserved: HashMap::new(),
        respawn_at: None,
        pointer: None,
        pointer_surface: None,
        exit: false,
    };
    let _ = event_queue.roundtrip(&mut app); // output names before spawning
    let (cards, reserved) = spawn_cards(
        &qh,
        &app.compositor,
        &layer_shell,
        &app.output_state,
        &app.pal,
        &app.text,
        0,
    );
    app.cards = cards;
    app.reserved = reserved;
    eprintln!("hypr-cardhost: {} card(s) spawned", app.cards.len());

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");
    event_loop
        .handle()
        .insert_source(fetch_rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(d) = ev {
                app.deliver(d);
            }
        })
        .expect("fetch channel");

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

    // ONE master 1 s tick drives everything time-based: the clock minute
    // edge, source intervals, the debounced respawn, the reserve recheck,
    // the first-run toast. Deliveries arrive via the fetch channel.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(TICK), |_, _, app: &mut App| {
            app.tick += 1;
            let tick = app.tick;

            // pending respawn (reload verb, hotplug, reserved change)
            if app.respawn_at.is_some_and(|t| tick >= t) {
                app.respawn_at = None;
                // teardown: dropping a Card drops its LayerSurface
                app.cards.clear();
                // spawn needs the qh + layer_shell captured at setup — the
                // RESPAWN_REQ flag hands it to the main loop below
                unsafe { RESPAWN_REQ = true };
            }

            // reserved-inset recheck, once, a few seconds after start —
            // cards may spawn BEFORE waybar reserves its strip (python)
            if tick == 3 {
                let now: HashMap<String, (f64, f64, f64, f64)> = monitor_geo()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.2))
                    .collect();
                if now != app.reserved {
                    app.respawn_soon();
                }
            }

            // first-run toast at +10 s (marker-gated, python parity)
            if tick == 10 {
                let marker = hyprdesk::home().join(".local/state/hyprcard/first-run-done");
                if !marker.exists() {
                    let ok = Command::new("notify-send")
                        .args([
                            "-a",
                            "hyprcard",
                            "Desktop widgets",
                            "Press ALT+SHIFT+E to arrange — drag cards, they snap to a grid. \
                             Add more via Settings → Widgets.",
                        ])
                        .spawn()
                        .is_ok();
                    if ok {
                        let _ = std::fs::create_dir_all(marker.parent().unwrap());
                        let _ = std::fs::write(&marker, "");
                    }
                }
            }

            // clock minute edge
            let now_min = unsafe {
                let t = libc_time(std::ptr::null_mut());
                let mut tm: Tm = std::mem::zeroed();
                localtime_r(&t, &mut tm);
                format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
            };
            if now_min != app.last_minute {
                app.last_minute = now_min;
                for c in app.cards.iter_mut() {
                    if c.has_clock {
                        c.dirty = true;
                    }
                }
            }

            // builtin sources at their card's cadence
            let stats_ivl = app
                .cards
                .iter()
                .filter(|c| c.template.source_builtin == "stats")
                .map(|c| c.interval)
                .min();
            if let Some(ivl) = stats_ivl {
                if tick % ivl.max(1) == 0 {
                    let fields = app.stats.fetch();
                    // async ps top refresh rides the fetch channel
                    sources::spawn_fetch(
                        "__ps".into(),
                        0,
                        vec![
                            "ps".into(),
                            "-eo".into(),
                            "comm,pcpu".into(),
                            "--sort=-pcpu".into(),
                        ],
                        app.fetch_tx.clone(),
                    );
                    for i in 0..app.cards.len() {
                        if app.cards[i].template.source_builtin == "stats" {
                            self_update(app, i, Some(fields.clone()));
                        }
                    }
                }
            }
            let net_ivl = app
                .cards
                .iter()
                .filter(|c| c.template.source_builtin == "netgraph")
                .map(|c| c.interval)
                .min();
            if let Some(ivl) = net_ivl {
                if tick % ivl.max(1) == 0 {
                    let fields = app.net.fetch();
                    app.series
                        .insert("down".into(), app.net.series_down.clone());
                    app.series.insert("up".into(), app.net.series_up.clone());
                    for i in 0..app.cards.len() {
                        if app.cards[i].template.source_builtin == "netgraph" {
                            self_update(app, i, Some(fields.clone()));
                        }
                    }
                }
            }
            // script sources + the none-ticker (notesfile / calgrid re-read)
            for i in 0..app.cards.len() {
                if tick >= app.cards[i].next_due {
                    let ivl = app.cards[i].interval.max(1);
                    app.cards[i].next_due = tick + ivl;
                    if !app.cards[i].template.source_cmd.is_empty() {
                        self_fire(app, i);
                    } else if app.cards[i].template.source_builtin == "none"
                        && rows_have(&app.cards[i].template, &["notesfile", "calgrid"])
                    {
                        self_update(app, i, Some(HashMap::new())); // just redraw
                    }
                }
            }
            TimeoutAction::ToDuration(TICK)
        })
        .expect("timer");

    // ctl socket: polled cheaply on every loop pass (nonblocking accept)
    loop {
        event_loop
            .dispatch(Some(Duration::from_millis(250)), &mut app)
            .expect("dispatch");
        if let Some(l) = &listener {
            while let Ok((client, _)) = l.accept() {
                app.on_ctl(client);
            }
        }
        if unsafe { RESPAWN_REQ } {
            unsafe { RESPAWN_REQ = false };
            let (cards, reserved) = spawn_cards(
                &qh,
                &app.compositor,
                &layer_shell,
                &app.output_state,
                &app.pal,
                &app.text,
                app.tick,
            );
            app.cards = cards;
            app.reserved = reserved;
            eprintln!("hypr-cardhost: respawned {} card(s)", app.cards.len());
        }
        for i in 0..app.cards.len() {
            if app.cards[i].dirty && app.cards[i].configured {
                app.draw_card(i);
            }
        }
        if app.exit {
            let _ = std::fs::remove_file(ctl_sock_path());
            return;
        }
    }
}

// timer closures can't reach App methods through the borrow of the fields
// they iterate — tiny free-fn trampolines keep the call sites readable
fn self_update(app: &mut App, i: usize, fields: Option<HashMap<String, String>>) {
    app.update_fields(i, fields);
}
fn self_fire(app: &mut App, i: usize) {
    app.fire_cmd_fetch(i);
}

static mut RESPAWN_REQ: bool = false;

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
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.respawn_soon(); // hotplug: rebuild on the debounced tick
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.respawn_soon();
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.cards
            .retain(|c| c.layer.wl_surface() != layer.wl_surface());
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
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

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.pointer = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for ev in events {
            match &ev.kind {
                PointerEventKind::Enter { .. } => {
                    // only action cards have a non-empty input region, so
                    // an Enter can only come from one of them
                    self.pointer_surface = Some(ev.surface.clone());
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_surface = None;
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    let Some(surf) = &self.pointer_surface else { continue };
                    if let Some(card) = self
                        .cards
                        .iter()
                        .find(|c| c.layer.wl_surface() == surf && !c.action.is_empty())
                    {
                        // detached, so a slow tool never blocks the other
                        // 13 cards' render loop (python comment, kept)
                        let _ = Command::new("setsid")
                            .args(["sh", "-c", &card.action])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                    }
                }
                _ => {}
            }
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
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_pointer!(App);
delegate_registry!(App);
