//! hypr-pet — pixel creatures wandering the bottom edge (rung 3a).
//! bin/hypr-pet (python) is the spec; docs/rust-migration.md the plan.
//!
//! The contract that makes this widget what it is: the INPUT REGION
//! follows the sprites exactly — the rest of the strip stays
//! click-through (hyprland-ui-ux rules). While dragging it widens to the
//! whole strip so motion events cannot outrun the region between ticks.
//!   · click a pet    → hearts + happy wiggle (deferred 280 ms so a
//!                      double-click can cancel it)
//!   · double-click   → sleep / wake
//!   · drag (>6 px)   → pick it up; release mid-air and gravity takes it
//!   · sometimes a pet wanders toward your mouse on its own

mod pet;
mod sprites;

use std::process::Command;
use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer};
use calloop::{channel, EventLoop};
use calloop_wayland_source::WaylandSource;
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

use pet::{Pet, State, GROUND, SCALE, STRIP_H};

const FRAME: Duration = Duration::from_millis(140); // python FPS_MS
const DRAG_PX: f64 = 6.0; // click-vs-drag threshold (ui-ux heuristics)
const DBLCLICK: Duration = Duration::from_millis(400);
const LOVE_DEFER_TICKS: u64 = 2; // ≈280 ms at 140 ms/tick, python parity

fn already_running() -> bool {
    // the manager guards with pgrep too, but a hand launch must not
    // double-spawn a second strip over the first
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

struct Grab {
    idx: usize,
    press_x: f64,
    press_y: f64,
    at: Instant,
    dragging: bool,
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    configured: bool,
    need_draw: bool,
    tick: u64,
    pets: Vec<Pet>,
    pal: hyprdesk::Palette,
    text: hyprdesk::draw::Text,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_pos: (f64, f64),
    pressed: bool,
    grab: Option<Grab>,
    last_press: Option<(usize, Instant)>, // for double-click detection
    pending_love: Option<(usize, u64)>,   // (pet, due tick)
    mon_off_x: Option<i32>,               // global x of this strip
    exit: bool,
}

impl App {
    fn spawn_pets(&mut self) {
        if !self.pets.is_empty() {
            return;
        }
        let w = self.width as f32;
        self.pets.push(Pet::new(pet::cat(), w));
        if hyprdesk::conf_get("claude_pet", "on") == "on" {
            self.pets.push(Pet::new(pet::clawd(), w));
        }
    }

    /// Clicks land only ON the pets; the rest of the strip passes through.
    fn update_input_region(&mut self, everything: bool) {
        let Ok(region) = Region::new(&self.compositor) else {
            return;
        };
        if everything {
            region.add(0, 0, self.width as i32, STRIP_H as i32);
        } else {
            for p in &self.pets {
                let (x, top, w, h) = p.bbox();
                let cx = x.max(0.0) as i32;
                region.add(cx, top as i32, (w - (cx as f32 - x)) as i32, h as i32);
            }
        }
        self.layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
        // the region lands on the next commit; draw() commits every frame
    }

    fn pet_at(&self, x: f64, y: f64) -> Option<usize> {
        // topmost-drawn pet wins (python iterates reversed)
        (0..self.pets.len())
            .rev()
            .find(|&i| self.pets[i].hit(x as f32, y as f32))
    }

    fn draw(&mut self) {
        if !self.configured || self.width == 0 {
            return;
        }
        let (w, h) = (self.width, STRIP_H as u32);
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        let mut pix = tiny_skia::Pixmap::new(w, h).unwrap();
        for p in &self.pets {
            let frame = p.frame();
            let y = p.top_y();
            sprites::blit(
                &mut pix,
                frame,
                p.species.ink,
                &self.pal,
                p.x,
                y,
                SCALE,
                p.dir > 0.0,
            );
            match p.state {
                State::Sleep => {
                    for i in 0..=(p.zzz) {
                        self.text.draw(
                            &mut pix,
                            p.x + 9.0 * SCALE + i as f32 * 10.0,
                            y - 2.0 - i as f32 * 8.0,
                            8.0 + i as f32 * 3.0,
                            self.pal.accent2,
                            "z",
                        );
                    }
                }
                State::Love => {
                    for i in 0..=(p.zzz) {
                        self.text.draw(
                            &mut pix,
                            p.x + 3.0 * SCALE + i as f32 * 12.0,
                            y - 4.0 - i as f32 * 7.0,
                            9.0 + i as f32 * 2.0,
                            hyprdesk::Rgb(0xE0, 0x6C, 0x75), // python's ♥ pink
                            "♥",
                        );
                    }
                }
                _ => {}
            }
        }
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(surface).ok();
        self.layer.commit();
    }

    fn find_offset(&self) -> i32 {
        let Ok(out) = Command::new("hyprctl").args(["-j", "layers"]).output() else {
            return 0;
        };
        let Ok(v) =
            serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&out.stdout))
        else {
            return 0;
        };
        if let Some(mons) = v.as_object() {
            for mon in mons.values() {
                if let Some(levels) = mon.get("levels").and_then(|l| l.as_object()) {
                    for lvl in levels.values() {
                        if let Some(arr) = lvl.as_array() {
                            for l in arr {
                                if l.get("namespace").and_then(|n| n.as_str())
                                    == Some("hypr-pet")
                                {
                                    return l.get("x").and_then(|x| x.as_i64()).unwrap_or(0)
                                        as i32;
                                }
                            }
                        }
                    }
                }
            }
        }
        0
    }

    fn follow_poll(&mut self) {
        if !self.pets.iter().any(|p| p.state == State::Follow) {
            return;
        }
        if self.mon_off_x.is_none() {
            self.mon_off_x = Some(self.find_offset());
        }
        let Ok(out) = Command::new("hyprctl").arg("cursorpos").output() else {
            return;
        };
        let s = String::from_utf8_lossy(&out.stdout);
        let Some(gx) = s.split(',').next().and_then(|t| t.trim().parse::<i32>().ok()) else {
            return;
        };
        let local = (gx - self.mon_off_x.unwrap_or(0)) as f32;
        for p in self.pets.iter_mut() {
            if p.state == State::Follow {
                p.follow_x = if local >= 0.0 && local <= p.width {
                    Some(local)
                } else {
                    None
                };
            }
        }
    }
}

enum Sig {
    Move,
    Theme,
    Quit,
}

fn main() {
    if already_running() {
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
    // env wins over conf — HYPRPET_LAYER is how desktop-widgets passes the
    // Settings' pet_layer through, python parity
    let lname = std::env::var("HYPRPET_LAYER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| hyprdesk::conf_get("pet_layer", "bottom"));
    let lvl = if lname == "top" { Layer::Top } else { Layer::Bottom };

    let want_mon = hyprdesk::conf_get("pet_mon", "");
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

    let layer =
        layer_shell.create_layer_surface(&qh, surface, lvl, Some("hypr-pet"), output.as_ref());
    // full-width strip on the bottom edge: anchor three sides, height only
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, STRIP_H as u32);
    layer.set_exclusive_zone(0);
    layer.commit();

    let pool = SlotPool::new(4096, &shm).expect("shm pool"); // grows on demand

    let mut app = App {
        registry_state,
        output_state,
        seat_state,
        compositor,
        shm,
        pool,
        layer,
        width: 0,
        configured: false,
        need_draw: false,
        tick: 0,
        pets: Vec::new(),
        pal: hyprdesk::colors(),
        text: hyprdesk::draw::Text::load(),
        pointer: None,
        pointer_pos: (0.0, 0.0),
        pressed: false,
        grab: None,
        last_press: None,
        pending_love: None,
        mon_off_x: None,
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
                        // a full-width strip has no x/y to re-read; the
                        // cached monitor offset is what can go stale
                        app.mon_off_x = None;
                    }
                    Sig::Theme => {
                        app.pal = hyprdesk::colors();
                        app.need_draw = true;
                    }
                    Sig::Quit => app.exit = true,
                }
            }
        })
        .expect("signal source");

    event_loop
        .handle()
        .insert_source(Timer::from_duration(FRAME), |_, _, app: &mut App| {
            app.tick += 1;
            // deferred love: fires unless a double-click cancelled it
            if let Some((idx, due)) = app.pending_love {
                if app.tick >= due {
                    app.pending_love = None;
                    if let Some(p) = app.pets.get_mut(idx) {
                        p.love();
                        app.need_draw = true;
                    }
                }
            }
            // follow-the-mouse, ~1 s cadence like the python poll
            if app.tick % 7 == 0 {
                app.follow_poll();
            }
            let mut moved = false;
            for p in app.pets.iter_mut() {
                p.width = app.width as f32;
                if p.tick() {
                    moved = true;
                }
            }
            if moved {
                app.need_draw = true;
                let dragging = app.grab.as_ref().is_some_and(|g| g.dragging);
                if !dragging {
                    app.update_input_region(false);
                }
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
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        // full width arrives HERE, not at creation — python read the Gdk
        // monitor; a layer surface reads its configure
        self.width = configure.new_size.0.max(1);
        self.configured = true;
        self.spawn_pets();
        self.update_input_region(false);
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
        if capability == Capability::Pointer {
            // release any stale proxy first — a leaked wl_pointer keeps
            // delivering events, doubling every click (same bug as appdock)
            if let Some(old) = self.pointer.take() {
                old.release();
            }
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
            if let Some(old) = self.pointer.take() {
                old.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        // only honour the pointer we currently own — a stale proxy
        // must stay silent (double-launch bug, same as appdock)
        if self.pointer.as_ref() != Some(pointer) {
            return;
        }
        for ev in events {
            match ev.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_pos = ev.position;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_pos = ev.position;
                    let Some(g) = self.grab.as_mut() else { continue };
                    if !self.pressed {
                        // release outside our surface — recover, don't freeze
                        let (idx, dragging) = (g.idx, g.dragging);
                        self.grab = None;
                        if dragging {
                            if let Some(p) = self.pets.get_mut(idx) {
                                p.drop();
                            }
                        }
                        self.update_input_region(false);
                        self.need_draw = true;
                        continue;
                    }
                    let (x, y) = ev.position;
                    if !g.dragging
                        && ((x - g.press_x).abs() > DRAG_PX || (y - g.press_y).abs() > DRAG_PX)
                    {
                        g.dragging = true;
                        self.update_input_region(true);
                    }
                    if self.grab.as_ref().is_some_and(|g| g.dragging) {
                        let idx = self.grab.as_ref().unwrap().idx;
                        if let Some(p) = self.pets.get_mut(idx) {
                            p.carry(x as f32, y as f32);
                        }
                        self.need_draw = true;
                    }
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    self.pressed = true;
                    let (x, y) = self.pointer_pos;
                    let Some(idx) = self.pet_at(x, y) else { continue };
                    let now = Instant::now();
                    let dbl = self
                        .last_press
                        .is_some_and(|(i, t)| i == idx && now.duration_since(t) < DBLCLICK);
                    self.last_press = Some((idx, now));
                    self.pending_love = None; // the tap starting a dbl-click
                    if dbl {
                        // must not also fire love
                        if let Some(p) = self.pets.get_mut(idx) {
                            p.toggle_sleep();
                        }
                        self.grab = None;
                        self.need_draw = true;
                        continue;
                    }
                    self.grab = Some(Grab {
                        idx,
                        press_x: x,
                        press_y: y,
                        at: now,
                        dragging: false,
                    });
                }
                PointerEventKind::Release { button: 0x110, .. } => {
                    self.pressed = false;
                    let Some(g) = self.grab.take() else { continue };
                    if g.dragging {
                        if let Some(p) = self.pets.get_mut(g.idx) {
                            p.drop();
                        }
                    } else if g.at.elapsed() < Duration::from_millis(400) {
                        // defer: a double-click's second press cancels this
                        self.pending_love = Some((g.idx, self.tick + LOVE_DEFER_TICKS));
                    }
                    self.update_input_region(false);
                    self.need_draw = true;
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

// the python draws pets above GROUND with y math tied to these — keep the
// constants visible to reviewers comparing the two implementations
#[allow(dead_code)]
const _PARITY_NOTE: (f32, f32) = (GROUND, STRIP_H);
