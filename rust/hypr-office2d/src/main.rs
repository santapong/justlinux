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
mod text;

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

use scene::{Click, Pass, Scene, H, W};

const FRAME: Duration = Duration::from_millis(160); // python FPS_MS
const RECONCILE_EVERY: u64 = 12; // ticks — ~2 s, python POLL_S

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

fn apply_placement(layer: &LayerSurface) {
    let (pos, x, y) = hyprdesk::placement("office2d", hyprdesk::Pos::BottomLeft, 24, 76);
    layer.set_anchor(anchor_for(pos));
    layer.set_margin(y, x, y, x);
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
        .arg("python3")
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
        Click::Background => launch_studio(),
    }
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    configured: bool,
    need_draw: bool,
    tick: u64,
    scene: Scene,
    pixmap: tiny_skia::Pixmap,
    bg: tiny_skia::Pixmap,     // cached static pass
    static_dirty: bool,
    pal: hyprdesk::Palette,
    text: text::Text,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_pos: (f64, f64),
    exit: bool,
}

impl App {
    fn draw(&mut self) {
        if !self.configured {
            return;
        }
        let stride = W as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(W as i32, H as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        if self.static_dirty {
            self.static_dirty = false;
            let mut bg = std::mem::replace(&mut self.bg, tiny_skia::Pixmap::new(1, 1).unwrap());
            bg.data_mut().fill(0);
            self.scene.render(&mut bg, &self.pal, &self.text, Pass::Static);
            self.bg = bg;
        }
        // frame = cached static + the few things that move
        self.pixmap.data_mut().copy_from_slice(self.bg.data());
        let mut pixmap = std::mem::replace(&mut self.pixmap, tiny_skia::Pixmap::new(1, 1).unwrap());
        self.scene.render(&mut pixmap, &self.pal, &self.text, Pass::Dynamic);
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pixmap.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        self.pixmap = pixmap;
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, W as i32, H as i32);
        buffer.attach_to(surface).ok();
        self.layer.commit();
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
    layer.set_size(W, H);
    layer.set_exclusive_zone(0);
    apply_placement(&layer);
    layer.commit();

    let pool = SlotPool::new((W * H * 4) as usize, &shm).expect("shm pool");
    let mut scene = Scene::new();
    scene.reconcile(); // first frame shows the world, not an empty floor

    let mut app = App {
        registry_state,
        output_state,
        seat_state,
        shm,
        pool,
        layer,
        configured: false,
        need_draw: false,
        tick: 0,
        scene,
        pixmap: tiny_skia::Pixmap::new(W, H).unwrap(),
        bg: tiny_skia::Pixmap::new(W, H).unwrap(),
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
                    Sig::Move => {
                        apply_placement(&app.layer);
                        app.layer.commit();
                    }
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
                app.scene.reconcile();
                app.static_dirty = true; // labels/states/counts may differ
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
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.configured = true;
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
