//! hypr-office2d — the 2D office (docs/rust-migration.md rung 2, P1+P2).
//! Ships SIDE BY SIDE with the python office behind `office_layout =
//! grid|floor`; desktop-widgets.sh keeps starting the python one until
//! that key says otherwise, so this binary carries its own toggle only.
//!
//! Layer contract (same as every card): namespace hypr-office2d, BOTTOM
//! layer, exclusive zone 0, placement from office2d_pos/_x/_y/_mon.
//! The surface is INTERACTIVE (desks are click targets, python-office
//! parity), so unlike viz the input region is the whole card.

mod scene;
mod sprites;
use hyprdesk::draw as text;

use std::process::Command;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::{channel, EventLoop};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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

use scene::{Click, Pass, Scene};
use smithay_client_toolkit::compositor::Region;

const FRAME: Duration = Duration::from_millis(160); // python FPS_MS
const RECONCILE_EVERY: u64 = 25; // ticks — ~4 s: the states this floor
// renders move on 15 s+ granularity, and reconcile is the CPU (measured
// 60 ms: proc scan + subagent walk + live-transcript re-reads)

fn toggle_kill_existing() -> bool {
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
    let others: Vec<i32> = out
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .filter(|&p| p as u32 != mine)
        .collect();
    if others.is_empty() {
        return false;
    }
    for p in others {
        unsafe { libc_kill(p, 15) };
    }
    // side-by-side phase: no conf persist — office_layout decides which
    // office autostarts, and this toggle must not fight the python one
    true
}

unsafe fn libc_kill(pid: i32, sig: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, sig);
}

fn anchor_for(pos: hyprdesk::Pos) -> Anchor {
    use hyprdesk::Pos::*;
    match pos {
        TopLeft => Anchor::TOP | Anchor::LEFT,
        TopMiddle => Anchor::TOP,
        TopRight => Anchor::TOP | Anchor::RIGHT,
        MiddleRight => Anchor::RIGHT,
        BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
        BottomMiddle => Anchor::BOTTOM,
        BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
        MiddleLeft => Anchor::LEFT,
    }
}

// fullscreen floor: anchored to every edge, size follows the monitor.
// The plates carve the input region; the rest of the desktop stays
// click-through (layer law).
fn apply_placement(layer: &LayerSurface) {
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0);
    layer.set_exclusive_zone(-1); // cover the whole monitor, waybar included
}

fn parse_clients(s: &str) -> std::collections::HashMap<i32, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
        if let Some(arr) = v.as_array() {
            for c in arr {
                if let (Some(pid), Some(addr)) = (
                    c.get("pid").and_then(|p| p.as_i64()),
                    c.get("address").and_then(|a| a.as_str()),
                ) {
                    map.insert(pid as i32, addr.to_string());
                }
            }
        }
    }
    map
}

/// The Hyprland window that HOSTS a pid — the claude process itself has
/// no window, its terminal does, so walk the ppid chain until a pid that
/// hyprctl lists as a client (python window_of_pid, same idea). A studio
/// tab's chain dead-ends at the tmux server and correctly returns None.
fn window_of_pid(pid: i32) -> Option<String> {
    let out = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    let clients = parse_clients(&String::from_utf8_lossy(&out.stdout));
    let mut cur = pid;
    for _ in 0..12 {
        if let Some(addr) = clients.get(&cur) {
            return Some(addr.clone());
        }
        let stat = std::fs::read_to_string(format!("/proc/{cur}/stat")).ok()?;
        let after = stat.rsplit_once(')')?.1;
        cur = after.split_whitespace().nth(1)?.parse().ok()?;
        if cur <= 1 {
            break;
        }
    }
    None
}

fn launch_studio() {
    let _ = Command::new("setsid")
        .arg(hyprdesk::home().join(".local/bin/hypr-claude-studio"))
        .spawn();
}

fn on_click(click: Click) {
    match click {
        Click::Session(row) => {
            if let Some(addr) = window_of_pid(row.pid) {
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "focuswindow", &format!("address:{addr}")])
                    .status();
            } else {
                // hosted in the studio (no window of its own) or unfindable:
                // the studio is where you work — summon it
                launch_studio();
            }
        }
        Click::Ghost { sid, cwd } => {
            // NEVER a live session — the scene only ghosts transcripts that
            // are unbound and 120 s idle, so resume is safe by construction
            let dir = if cwd.is_empty() {
                hyprdesk::home().display().to_string()
            } else {
                cwd
            };
            let _ = Command::new("setsid")
                .args(["kitty", "--class", "hyprclaude", "-d", &dir, "-e", "claude", "--resume", &sid])
                .spawn();
        }
        Click::Meeting | Click::Background => launch_studio(),
    }
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    w: u32,
    h: u32,
    pool: SlotPool,
    layer: LayerSurface,
    configured: bool,
    need_draw: bool,
    need_region: bool,
    tick: u64,
    scene: Scene,
    pixmap: tiny_skia::Pixmap,
    bg: tiny_skia::Pixmap,     // cached static pass (RGBA)
    bg_bgra: Vec<u8>,          // the same, pre-swapped for the wl buffer
    prev_dyn: Vec<(i32, i32, i32, i32)>, // last frame's dynamic damage
    // per-slot stale regions: a slot-pool buffer we get back holds a frame
    // from two ticks ago; only what changed since then needs rewriting —
    // this is what turned a 5.7 MB per-tick memcpy into a few plate rects
    slot_dirty: std::collections::HashMap<usize, Vec<(i32, i32, i32, i32)>>,
    static_dirty: bool,
    pal: hyprdesk::Palette,
    text: text::Text,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_pos: (f64, f64),
    exit: bool,
}

impl App {
    /// The interactive pixels become the input region on every reconcile —
    /// a fullscreen bottom surface must never eat desktop clicks.
    fn apply_input_region(&mut self) {
        if let Ok(region) = Region::new(&self.compositor) {
            for (x, y, w, h) in self.scene.interactive_rects() {
                region.add(x, y, w, h);
            }
            self.layer.wl_surface().set_input_region(Some(region.wl_region()));
        }
    }

    fn draw(&mut self) {
        if !self.configured || self.w == 0 {
            return;
        }
        let t0 = std::time::Instant::now();
        let (w, h) = (self.w, self.h);
        if self.pixmap.width() != w || self.pixmap.height() != h {
            self.pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
            self.bg = tiny_skia::Pixmap::new(w, h).unwrap();
            self.scene.w = w as f32;
            self.scene.h = h as f32;
            self.static_dirty = true;
        }
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        if self.static_dirty {
            self.static_dirty = false;
            let mut bg = std::mem::replace(&mut self.bg, tiny_skia::Pixmap::new(1, 1).unwrap());
            bg.data_mut().fill(0);
            self.scene.render(&mut bg, &self.pal, &self.text, Pass::Static);
            // pre-swap ONCE — per-frame full-surface RGBA→BGRA was the bill
            self.bg_bgra.clear();
            self.bg_bgra.reserve(bg.data().len());
            for px in bg.data().chunks_exact(4) {
                self.bg_bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
            self.bg = bg;
            self.prev_dyn = vec![(0, 0, w as i32, h as i32)]; // full damage once
            self.slot_dirty.clear(); // every slot is stale now
        }
        let rects = self.scene.dynamic_rects();
        let slot = canvas.as_ptr() as usize;
        // restore = regions this SLOT is stale in (accumulated while other
        // slots were on screen) ∪ this frame's dynamic regions
        let mut restore = self.slot_dirty.remove(&slot).unwrap_or_else(|| {
            vec![(0, 0, w as i32, h as i32)] // first sight of this slot
        });
        restore.extend(rects.iter().copied());
        for &(rx, ry, rw, rh) in &restore {
            let (x0, y0) = (
                rx.clamp(0, w as i32) as usize,
                ry.clamp(0, h as i32) as usize,
            );
            let (x1, y1) = (
                (rx + rw).clamp(0, w as i32) as usize,
                (ry + rh).clamp(0, h as i32) as usize,
            );
            let stride_b = w as usize * 4;
            for row in y0..y1 {
                let a = row * stride_b + x0 * 4;
                let b = row * stride_b + x1 * 4;
                canvas[a..b].copy_from_slice(&self.bg_bgra[a..b]);
            }
        }
        // every OTHER slot goes stale wherever we draw this frame
        let all_dirty: Vec<(i32, i32, i32, i32)> =
            self.prev_dyn.iter().chain(rects.iter()).copied().collect();
        for (k, v) in self.slot_dirty.iter_mut() {
            if *k != slot {
                v.extend(all_dirty.iter().copied());
                if v.len() > 64 {
                    *v = vec![(0, 0, w as i32, h as i32)];
                }
            }
        }
        self.slot_dirty.entry(slot).or_default().clear();
        let stride_px = w as usize * 4;
        let clamp = |v: i32, hi: u32| v.clamp(0, hi as i32) as usize;
        for &(rx, ry, rw, rh) in &rects {
            let (x0, y0) = (clamp(rx, w), clamp(ry, h));
            let (x1, y1) = (clamp(rx + rw, w), clamp(ry + rh, h));
            for row in y0..y1 {
                let a = row * stride_px + x0 * 4;
                let b = row * stride_px + x1 * 4;
                self.pixmap.data_mut()[a..b].copy_from_slice(&self.bg.data()[a..b]);
            }
        }
        if !rects.is_empty() {
            let mut pixmap =
                std::mem::replace(&mut self.pixmap, tiny_skia::Pixmap::new(1, 1).unwrap());
            self.scene.render(&mut pixmap, &self.pal, &self.text, Pass::Dynamic);
            self.pixmap = pixmap;
            for &(rx, ry, rw, rh) in &rects {
                let (x0, y0) = (clamp(rx, w), clamp(ry, h));
                let (x1, y1) = (clamp(rx + rw, w), clamp(ry + rh, h));
                for row in y0..y1 {
                    let a = row * stride_px + x0 * 4;
                    let b = row * stride_px + x1 * 4;
                    for (dst, src) in canvas[a..b]
                        .chunks_exact_mut(4)
                        .zip(self.pixmap.data()[a..b].chunks_exact(4))
                    {
                        dst[0] = src[2];
                        dst[1] = src[1];
                        dst[2] = src[0];
                        dst[3] = src[3];
                    }
                }
            }
        }
        let surface = self.layer.wl_surface();
        // damage = last frame's dynamic rects ∪ this frame's
        for &(rx, ry, rw, rh) in self.prev_dyn.iter().chain(rects.iter()) {
            surface.damage_buffer(rx, ry, rw, rh);
        }
        // remember what THIS slot now shows beyond bg (so siblings learn)
        self.slot_dirty.insert(slot, rects.clone());
        self.prev_dyn = rects;
        buffer.attach_to(surface).ok();
        self.layer.commit();
        if std::env::var("OFFICE2D_PROFILE").is_ok() {
            eprintln!("draw {:?}", t0.elapsed());
        }
    }
}

enum Sig {
    Move,
    Theme,
    Quit,
}

fn main() {
    if toggle_kill_existing() {
        return;
    }
    let conn = Connection::connect_to_env().expect("no wayland display");
    let (globals, event_queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh: QueueHandle<App> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);

    let surface = compositor.create_surface(&qh);
    let want_mon = hyprdesk::conf_get("office2d_mon", "");
    let output: Option<wl_output::WlOutput> = if want_mon.is_empty() {
        None
    } else {
        output_state.outputs().find(|o| {
            output_state
                .info(o)
                .and_then(|i| i.name)
                .is_some_and(|n| n == want_mon)
        })
    };
    // OFFICE2D_LAYER=top: dev override so acceptance screenshots can see
    // a bottom-layer surface without clearing every window off a monitor
    let lvl = if std::env::var("OFFICE2D_LAYER").as_deref() == Ok("top") {
        Layer::Top
    } else {
        Layer::Bottom
    };
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        lvl,
        Some("hypr-office2d"),
        output.as_ref(),
    );
    apply_placement(&layer);
    layer.commit();

    let pool = SlotPool::new(1600 * 900 * 4, &shm).expect("shm pool");
    let mut scene = Scene::new();
    scene.motion = hyprdesk::conf_get("office_motion", "on") != "off";
    scene.reconcile(); // first frame shows the world, not an empty floor

    let mut app = App {
        registry_state,
        output_state,
        seat_state,
        compositor,
        shm,
        w: 0,
        h: 0,
        pool,
        layer,
        configured: false,
        need_draw: false,
        need_region: false,
        tick: 0,
        scene,
        pixmap: tiny_skia::Pixmap::new(1, 1).unwrap(),
        bg: tiny_skia::Pixmap::new(1, 1).unwrap(),
        bg_bgra: Vec::new(),
        prev_dyn: Vec::new(),
        slot_dirty: std::collections::HashMap::new(),
        static_dirty: true,
        pal: hyprdesk::colors(),
        text: text::Text::load(),
        pointer: None,
        pointer_pos: (0.0, 0.0),
        exit: false,
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");

    let (tx, rx) = channel::channel::<Sig>();
    std::thread::spawn(move || {
        use signal_hook::consts::{SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
        let mut sigs =
            signal_hook::iterator::Signals::new([SIGUSR1, SIGUSR2, SIGTERM, SIGINT]).unwrap();
        for s in sigs.forever() {
            let _ = tx.send(match s {
                SIGUSR1 => Sig::Move,
                SIGUSR2 => Sig::Theme,
                _ => Sig::Quit,
            });
        }
    });
    event_loop
        .handle()
        .insert_source(rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(sig) = ev {
                match sig {
                    Sig::Move => {} // fullscreen: nothing to reposition
                    Sig::Theme => {
                        app.pal = hyprdesk::colors();
                        app.static_dirty = true;
                        app.need_draw = true;
                    }
                    Sig::Quit => app.exit = true,
                }
            }
        })
        .expect("signal source");

    // ONE timer paces both halves: every tick animates, every 12th
    // reconciles. The lesson from viz is baked in — nothing but this
    // timer ever schedules a redraw.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(FRAME), |_, _, app: &mut App| {
            app.tick += 1;
            let mut changed = false;
            if app.tick % RECONCILE_EVERY == 0 {
                let tr = std::time::Instant::now();
                app.scene.reconcile();
                if std::env::var("OFFICE2D_PROFILE").is_ok() {
                    eprintln!("reconcile {:?}", tr.elapsed());
                }
                app.static_dirty = true; // labels/states/counts may differ
                app.need_region = true;  // plates may have moved
                changed = true;
            }
            let (redraw, sdirty) = app.scene.animate();
            changed |= redraw;
            app.static_dirty |= sdirty;
            if changed {
                app.need_draw = true;
            }
            TimeoutAction::ToDuration(FRAME)
        })
        .expect("timer");

    loop {
        event_loop
            .dispatch(Some(FRAME), &mut app)
            .expect("dispatch");
        if app.need_region {
            app.need_region = false;
            app.apply_input_region();
        }
        if app.need_draw {
            app.need_draw = false;
            app.draw();
        }
        if app.exit {
            return;
        }
    }
}

// ---------- sctk boilerplate ----------
impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        // adopt the granted size BEFORE drawing — a wayland surface IS
        // its buffer (the 1 px strip lesson, docs/language-policy.md)
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.w = w;
            self.h = h;
        }
        self.configured = true;
        self.apply_input_region();
        self.draw();
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
            match ev.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_pos = ev.position;
                    let over = self
                        .scene
                        .desk_at(ev.position.0 as f32, ev.position.1 as f32);
                    if over != self.scene.hover {
                        self.scene.hover = over;
                        self.static_dirty = true; // the wash lives in the bg
                        self.need_draw = true; // same-frame feedback
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.scene.hover.is_some() {
                        self.scene.hover = None;
                        self.static_dirty = true;
                        self.need_draw = true;
                    }
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    // BTN_LEFT. Click actions fork detached externals; the
                    // loop never waits on them.
                    let (x, y) = self.pointer_pos;
                    on_click(self.scene.hit(x as f32, y as f32));
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
