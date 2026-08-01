//! hypr-viz — ambient audio visualizer. Pilot rung of the Rust migration;
//! bin/hypr-viz (Python) is the spec and stays canonical until every box in
//! docs/rust-migration.md "Pilot acceptance criteria" is ticked.
//!
//! Same contract as the Python: 300x64 glass panel of 12 spectrum bars on
//! the BOTTOM layer, namespace hypr-viz (the blur layerrules match it),
//! empty input region (display-only — never eats a click), placement from
//! viz_pos/_x/_y/_mon, SIGUSR1 reposition, SIGUSR2 retheme, second
//! invocation kills the first and persists viz=off (starting never
//! persists on — the hotkey is a toy, only the Settings switch autostarts).
//!
//! SHIP NOTE: desktop-widgets.sh and the Settings restart path match the
//! PYTHON cmdline (`pkill -xf "python3 .../hypr-viz"`). When this binary
//! replaces bin/hypr-viz those patterns must change in the same commit.

mod audio;
// the fleet contracts moved to the shared workspace crate (step 0 of rung 2)

use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::{channel, EventLoop};
use calloop_wayland_source::WaylandSource;
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

const W: u32 = 300;
const H: u32 = 64;
const FRAME: Duration = Duration::from_millis(40); // ~25fps, the python cadence

// ---------- toggle: second invocation kills the first ----------
fn toggle_kill_existing() -> bool {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if me.is_empty() {
        return false;
    }
    // -xf, exact full cmdline: a substring -f match kills any shell whose
    // command line CONTAINS the path (the hyprcard lesson, kept)
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
        unsafe { libc_kill(p, 15) }; // SIGTERM
    }
    hyprdesk::conf_set("viz", "off");
    true
}

// one raw extern beats pulling in a libc crate for a single call
unsafe fn libc_kill(pid: i32, sig: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, sig);
}

// ---------- placement: the POS_ANCHORS edge map from lib/hyprdesk ----------
fn placement() -> (Anchor, i32, i32) {
    let pos = hyprdesk::conf_get("viz_pos", "bottom_right");
    let x: i32 = hyprdesk::conf_get("viz_x", "24").parse().unwrap_or(24);
    // default y=76 clears the 56px pet strip (widget-card rule)
    let y: i32 = hyprdesk::conf_get("viz_y", "76").parse().unwrap_or(76);
    let anchor = match pos.as_str() {
        "top_left" => Anchor::TOP | Anchor::LEFT,
        "top_middle" => Anchor::TOP,
        "top_right" => Anchor::TOP | Anchor::RIGHT,
        "middle_right" => Anchor::RIGHT,
        "bottom_middle" => Anchor::BOTTOM,
        "bottom_left" => Anchor::BOTTOM | Anchor::LEFT,
        "middle_left" => Anchor::LEFT,
        _ => Anchor::BOTTOM | Anchor::RIGHT,
    };
    (anchor, x, y)
}

fn apply_placement(layer: &LayerSurface) {
    let (anchor, x, y) = placement();
    layer.set_anchor(anchor);
    // margins land on the anchored edges only; setting all four is harmless
    layer.set_margin(y, x, y, x);
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    configured: bool,
    need_draw: bool,
    levels: Arc<Mutex<[f32; audio::BANDS]>>,
    pal: hyprdesk::Palette,
    exit: bool,
}

impl App {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
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

        let mut pixmap = tiny_skia::Pixmap::new(W, H).unwrap();
        // glass background: rounded rect, bg at 0.80 — blur layerrule does the rest
        let r = 14.0f32;
        let mut pb = tiny_skia::PathBuilder::new();
        let (w, h) = (W as f32, H as f32);
        pb.move_to(r, 0.0);
        pb.line_to(w - r, 0.0);
        pb.quad_to(w, 0.0, w, r);
        pb.line_to(w, h - r);
        pb.quad_to(w, h, w - r, h);
        pb.line_to(r, h);
        pb.quad_to(0.0, h, 0.0, h - r);
        pb.line_to(0.0, r);
        pb.quad_to(0.0, 0.0, r, 0.0);
        pb.close();
        let path = pb.finish().unwrap();
        let mut paint = tiny_skia::Paint::default();
        paint.anti_alias = true;
        let hyprdesk::Rgb(br, bgc, bb) = self.pal.bg;
        paint.set_color(tiny_skia::Color::from_rgba8(br, bgc, bb, 204)); // 0.80
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );

        // bars: same geometry as the python (inset 14, gap 5, floor 2px)
        let levels = *self.levels.lock().unwrap();
        let (inset, gap) = (14.0f32, 5.0f32);
        let bw = (w - 2.0 * inset - gap * (audio::BANDS as f32 - 1.0)) / audio::BANDS as f32;
        let hyprdesk::Rgb(lr, lg, lb) = self.pal.accent;
        let hyprdesk::Rgb(hr, hg, hb) = self.pal.accent2;
        for (i, lv) in levels.iter().enumerate() {
            let t = i as f32 / (audio::BANDS as f32 - 1.0);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let bh = (lv * (h - 24.0)).max(2.0);
            let x = inset + i as f32 * (bw + gap);
            paint.set_color(tiny_skia::Color::from_rgba8(
                mix(lr, hr),
                mix(lg, hg),
                mix(lb, hb),
                242, // 0.95
            ));
            if let Some(rect) = tiny_skia::Rect::from_xywh(x, h - 12.0 - bh, bw, bh) {
                pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
            }
        }

        // tiny-skia is premultiplied RGBA in memory; wl ARGB8888 little-endian
        // wants premultiplied BGRA bytes — swap R and B
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pixmap.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, W as i32, H as i32);
        buffer.attach_to(surface).ok();
        // NO frame-callback request: the TIMER paces drawing. Requesting a
        // frame here woke the event loop the instant the compositor was
        // ready, which turned the main loop into a 92%-CPU redraw spin —
        // measured on the first live run, not guessed.
        let _ = qh;
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
    // NOTE: starting does NOT persist viz=on — hotkey starts must never
    // turn themselves into login autostarts (python rule, kept)

    let conn = Connection::connect_to_env().expect("no wayland display");
    let (globals, event_queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh: QueueHandle<App> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);

    let surface = compositor.create_surface(&qh);
    // display-only surface: EMPTY input region, it must never eat a click
    if let Ok(region) = Region::new(&compositor) {
        surface.set_input_region(Some(region.wl_region()));
    }

    // viz_mon names a connector; unset or absent → compositor default
    let want_mon = hyprdesk::conf_get("viz_mon", "");
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

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Bottom,
        Some("hypr-viz"),
        output.as_ref(),
    );
    layer.set_size(W, H);
    layer.set_exclusive_zone(0);
    apply_placement(&layer);
    layer.commit();

    let pool = SlotPool::new((W * H * 4) as usize, &shm).expect("shm pool");
    let levels = Arc::new(Mutex::new([0f32; audio::BANDS]));
    let stop = Arc::new(AtomicBool::new(false));
    audio::spawn(levels.clone(), stop.clone());

    let mut app = App {
        registry_state,
        output_state,
        shm,
        pool,
        layer,
        configured: false,
        need_draw: false,
        levels,
        pal: hyprdesk::colors(),
        exit: false,
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");

    // signals → the loop, via a channel: USR1/USR2 MUST have handlers (the
    // default disposition is terminate — the office lesson, kept)
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
                        // live reposition; the margin change only lands on the
                        // next COMMIT and silence draws nothing, so commit now
                        apply_placement(&app.layer);
                        app.layer.commit();
                    }
                    Sig::Theme => app.pal = hyprdesk::colors(),
                    Sig::Quit => app.exit = true,
                }
            }
        })
        .expect("signal source");

    // the timer is the ONLY thing that schedules a redraw — every other
    // wakeup (wayland events, signals) just handles its event and sleeps
    event_loop
        .handle()
        .insert_source(Timer::from_duration(FRAME), |_, _, app: &mut App| {
            app.need_draw = true;
            TimeoutAction::ToDuration(FRAME)
        })
        .expect("timer");

    loop {
        event_loop
            .dispatch(Some(FRAME), &mut app)
            .expect("dispatch");
        if app.need_draw {
            app.need_draw = false;
            let qh2 = qh.clone();
            app.draw(&qh2);
        }
        if app.exit {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
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
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.configured = true;
        let qh2 = qh.clone();
        self.draw(&qh2);
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
