//! hypr-appdock — pinned applications dock, one per monitor, per
//! workspace. bin/hypr-appdock (python/GTK) is the spec; this is the
//! resident port: docks + reveal strips + smart waybar + the Hyprland
//! event socket + the ctl socket (ping|reload|reload-theme|bar-pin|
//! show-all|resume — hypr-arrange and ALT+B speak all of them).
//!
//! The PICKER stays python (a GTK search dialog is exactly the app class
//! the migration keeps in python): the ＋ button spawns
//! `hypr-appdock-picker --picker --monitor <mon>`, its edits land in
//! pins.json under the shared flock, and the 2 s mtime watch here picks
//! them up — same propagation path the python dock already used for a
//! standalone picker.
//!
//! Auto-hide maps/unmaps GTK windows in python; here a hidden dock's
//! LayerSurface is destroyed and reveal recreates it — deterministic,
//! and the respawn path is exercised constantly anyway.

mod apps;
mod pins;

use std::collections::HashMap;
use std::io::{ErrorKind, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::time::{Duration, Instant};

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
use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const STRIP_H: u32 = 4; // reveal-strip height (px)
const DWELL: Duration = Duration::from_millis(180); // strip hover before reveal
const HIDE_DELAY: Duration = Duration::from_millis(600); // pointer-left → hide grace
const SUBTICK: Duration = Duration::from_millis(200); // master cadence
const SUSPEND_MAX: Duration = Duration::from_secs(360); // show-all failsafe
const RADIUS: f32 = 12.0;
const DOCK_PAD: f32 = 6.0; // CSS: padding 6px 10px
const DOCK_PAD_X: f32 = 10.0;
const BTN_PAD: f32 = 5.0; // CSS: button padding 5px
const SPACING: f32 = 2.0;

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn ctl_sock_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
        .join("hypr-appdock.sock")
}

fn hypr_json(cmd: &str) -> serde_json::Value {
    Command::new("hyprctl")
        .args(["-j", cmd])
        .output()
        .ok()
        .and_then(|o| serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[derive(Default, Clone, PartialEq)]
struct MonState {
    ws: i64,
    empty: bool,
    fullscreen: bool,
}

/// 'fullscreen' means TRUE fullscreen (mode 2) only — a maximized window
/// (mode 1, keeps waybar) must NOT suppress the docks (python).
fn monitor_states() -> HashMap<String, MonState> {
    let mut ws_windows: HashMap<i64, i64> = HashMap::new();
    if let Some(ws) = hypr_json("workspaces").as_array() {
        for w in ws {
            if let Some(id) = w.get("id").and_then(|v| v.as_i64()) {
                ws_windows.insert(id, w.get("windows").and_then(|v| v.as_i64()).unwrap_or(0));
            }
        }
    }
    let mut truefs: Vec<i64> = Vec::new();
    if let Some(cs) = hypr_json("clients").as_array() {
        for c in cs {
            if c.get("fullscreen").and_then(|v| v.as_i64()) == Some(2) {
                if let Some(id) = c
                    .get("workspace")
                    .and_then(|w| w.get("id"))
                    .and_then(|v| v.as_i64())
                {
                    truefs.push(id);
                }
            }
        }
    }
    let mut out = HashMap::new();
    if let Some(ms) = hypr_json("monitors").as_array() {
        for m in ms {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ws = m
                .get("activeWorkspace")
                .and_then(|w| w.get("id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            out.insert(
                name,
                MonState {
                    ws,
                    empty: ws_windows.get(&ws).copied().unwrap_or(0) == 0,
                    fullscreen: truefs.contains(&ws),
                },
            );
        }
    }
    out
}

fn focused_monitor() -> String {
    let ms = hypr_json("monitors");
    if let Some(arr) = ms.as_array() {
        for m in arr {
            if m.get("focused").and_then(|v| v.as_bool()) == Some(true) {
                return m.get("name").and_then(|v| v.as_str()).unwrap_or("").into();
            }
        }
        if let Some(m) = arr.first() {
            return m.get("name").and_then(|v| v.as_str()).unwrap_or("MON0").into();
        }
    }
    "MON0".into()
}

struct Btn {
    stem: String,
    label: String, // 2-letter fallback when the icon is unloadable
    icon: Option<Pixmap>,
    x: f32,
    w: f32,
}

#[derive(PartialEq, Clone, Copy)]
#[repr(u8)]
enum EdgeKind {
    DockBottom,
    DockTop,
    StripBottom,
    StripTop, // reveals that dock
    BarStrip, // reveals waybar (bar_smart)
}

struct Surf {
    mon: String,
    san: String,
    kind: EdgeKind,
    layer: Option<LayerSurface>,
    width: u32,
    height: u32,
    configured: bool,
    dirty: bool,
    pointer_in: bool,
    // docks only:
    ws: i64,
    icon: u32,
    btns: Vec<Btn>,
    plus_x: f32,
    hint: Option<String>,
    hover: Option<usize>, // btns index, or usize::MAX for ＋
    hide_at: Option<Instant>,
    // strips only:
    dwell_since: Option<Instant>,
}

impl Surf {
    fn is_dock(&self) -> bool {
        matches!(self.kind, EdgeKind::DockBottom | EdgeKind::DockTop)
    }
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    pal: hyprdesk::Palette,
    text: hyprdesk::draw::Text,
    surfs: Vec<Surf>,
    apps: HashMap<String, apps::AppEntry>,
    pins: serde_json::Value,
    pins_mtime: i64,
    states: HashMap<String, MonState>,
    // conf-derived, re-read on reload:
    layer_kind: Layer,
    autohide: bool,
    dock_top: bool,
    bar_smart: bool,
    suspend: bool,
    suspend_until: Option<Instant>,
    bar_pin: bool,
    bar_polling: bool,
    subtick: u64,
    refresh_at: Option<Instant>,
    respawn_at: Option<Instant>,
    sock_poll_fallback: bool,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_surface: Option<wl_surface::WlSurface>,
    pointer_pos: (f64, f64),
    exit: bool,
}

impl App {
    fn conf_reload(&mut self) {
        let dock_layer = hyprdesk::conf_get("dock_layer", "top");
        self.layer_kind = if dock_layer == "bottom" { Layer::Bottom } else { Layer::Top };
        self.autohide =
            dock_layer != "bottom" && hyprdesk::conf_get("dock_autohide", "on") == "on";
        self.dock_top = hyprdesk::conf_get("dock_top", "off") == "on";
        let was_smart = self.bar_smart;
        self.bar_smart = hyprdesk::conf_get("bar_smart", "off") == "on";
        if was_smart && !self.bar_smart && !bar_visible() {
            bar_toggle(); // smart mode off: leave the bar up
        }
        if self.bar_smart {
            self.bar_polling = true; // (re)enabled: tuck soon
        }
    }

    // ----- dock content -----
    fn rebuild_dock(&mut self, i: usize) {
        let (mon, ws, icon) = {
            let d = &self.surfs[i];
            (d.mon.clone(), d.ws, d.icon)
        };
        let mut btns = Vec::new();
        let mut x = DOCK_PAD_X;
        for stem in pins::pins_for(&self.pins, &mon, ws) {
            let Some(entry) = self.apps.get(&stem) else { continue };
            let pix = apps::icon_path(&entry.icon).and_then(|p| apps::load_icon(&p, icon));
            let w = icon as f32 + 2.0 * BTN_PAD;
            btns.push(Btn {
                stem,
                label: entry.name.chars().take(2).collect(),
                icon: pix,
                x,
                w,
            });
            x += w + SPACING;
        }
        let hint = if btns.is_empty() {
            Some(format!("{mon} · ws {ws} · no pins"))
        } else {
            None
        };
        if let Some(h) = &hint {
            x += self.text.advance(12.0, false, h) + 16.0;
        }
        let plus_x = x;
        let plus_w = 30.0;
        let d = &mut self.surfs[i];
        let w = (plus_x + plus_w + DOCK_PAD_X).round() as u32;
        let h = (icon as f32 + 2.0 * BTN_PAD + 2.0 * DOCK_PAD).round() as u32;
        d.btns = btns;
        d.hint = hint;
        d.plus_x = plus_x;
        d.hover = None;
        d.dirty = true;
        if (w, h) != (d.width, d.height) {
            d.width = w;
            d.height = h;
            if let Some(l) = &d.layer {
                l.set_size(w, h);
                l.commit();
            }
        }
    }

    // ----- mapping / unmapping -----
    fn map_surf(&mut self, i: usize, qh: &QueueHandle<App>, layer_shell: &LayerShell) {
        if self.surfs[i].layer.is_some() {
            return;
        }
        let outputs: HashMap<String, wl_output::WlOutput> = self
            .output_state
            .outputs()
            .filter_map(|o| self.output_state.info(&o).and_then(|inf| inf.name).map(|n| (n, o)))
            .collect();
        let s = &self.surfs[i];
        let Some(output) = outputs.get(&s.mon) else { return };
        let surface = self.compositor.create_surface(qh);
        let (ns, lay) = match s.kind {
            EdgeKind::DockBottom => (format!("hypr-appdock-{}", s.san), self.layer_kind),
            EdgeKind::DockTop => (format!("hypr-appdock-top-{}", s.san), self.layer_kind),
            EdgeKind::StripBottom => (format!("hypr-dockedge-bottom-{}", s.san), Layer::Top),
            EdgeKind::StripTop | EdgeKind::BarStrip => {
                (format!("hypr-dockedge-top-{}", s.san), Layer::Top)
            }
        };
        let layer = layer_shell.create_layer_surface(qh, surface, lay, Some(ns), Some(output));
        match s.kind {
            EdgeKind::StripBottom => {
                layer.set_anchor(Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM);
                layer.set_size(0, STRIP_H);
            }
            EdgeKind::StripTop | EdgeKind::BarStrip => {
                layer.set_anchor(Anchor::LEFT | Anchor::RIGHT | Anchor::TOP);
                layer.set_size(0, STRIP_H);
            }
            EdgeKind::DockTop => {
                // just below waybar: top-anchored, small gap (python d_y=6);
                // the exclusive-zone offset is the compositor's business
                layer.set_anchor(Anchor::TOP);
                layer.set_margin(6, 0, 0, 0);
                layer.set_size(s.width, s.height);
            }
            EdgeKind::DockBottom => {
                // per-monitor keys with legacy apps_* fallback (python)
                let c = |k: &str, d: &str| {
                    let per = hyprdesk::conf_get(&format!("dock_{}_{k}", s.san), "");
                    if per.is_empty() { hyprdesk::conf_get(&format!("apps_{k}"), d) } else { per }
                };
                let pos = c("pos", "bottom_middle");
                let x = c("x", "0").parse::<i32>().unwrap_or(0).clamp(-32000, 32000);
                let y = c("y", "12").parse::<i32>().unwrap_or(12).clamp(-32000, 32000);
                // LayerWindow._apply_pos: anchor the pos edges; margin x on
                // left/right, y on top/bottom
                let edges: &[Anchor] = match pos.as_str() {
                    "top_left" => &[Anchor::TOP, Anchor::LEFT],
                    "top_middle" => &[Anchor::TOP],
                    "top_right" => &[Anchor::TOP, Anchor::RIGHT],
                    "middle_left" => &[Anchor::LEFT],
                    "middle_right" => &[Anchor::RIGHT],
                    "bottom_left" => &[Anchor::BOTTOM, Anchor::LEFT],
                    "bottom_right" => &[Anchor::BOTTOM, Anchor::RIGHT],
                    _ => &[Anchor::BOTTOM],
                };
                let mut anchor = Anchor::empty();
                let (mut mt, mut mr, mut mb, mut ml) = (0, 0, 0, 0);
                for e in edges {
                    anchor |= *e;
                    match *e {
                        Anchor::LEFT => ml = x,
                        Anchor::RIGHT => mr = x,
                        Anchor::TOP => mt = y,
                        _ => mb = y,
                    }
                }
                layer.set_anchor(anchor);
                layer.set_margin(mt, mr, mb, ml);
                layer.set_size(s.width, s.height);
            }
        }
        layer.set_exclusive_zone(0);
        layer.commit();
        let s = &mut self.surfs[i];
        s.layer = Some(layer);
        s.configured = false;
        s.dirty = true;
    }

    fn unmap_surf(&mut self, i: usize) {
        let s = &mut self.surfs[i];
        s.layer = None; // drop destroys the layer surface
        s.configured = false;
        s.pointer_in = false;
        s.hover = None;
    }

    // ----- drawing -----
    fn draw_surf(&mut self, i: usize) {
        let (w, h, configured, is_dock) = {
            let s = &self.surfs[i];
            (s.width.max(1), s.height.max(1), s.configured, s.is_dock())
        };
        if !configured || self.surfs[i].layer.is_none() {
            return;
        }
        let mut pix = Pixmap::new(w, h).unwrap();
        if is_dock {
            self.paint_dock(&mut pix, i, w as f32, h as f32);
        } // strips stay fully transparent
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let s = &mut self.surfs[i];
        let layer = s.layer.as_ref().unwrap();
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(surface).ok();
        layer.commit();
        s.dirty = false;
    }

    fn paint_dock(&self, pix: &mut Pixmap, i: usize, w: f32, h: f32) {
        let s = &self.surfs[i];
        let pal = &self.pal;
        // glass: bg 0.80, radius 12, border accent2 0.18 (python CSS)
        let mut pb = PathBuilder::new();
        let r = RADIUS.min(w / 2.0).min(h / 2.0);
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
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.set_color_rgba8(pal.bg.0, pal.bg.1, pal.bg.2, 204);
        pix.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        paint.set_color_rgba8(pal.accent2.0, pal.accent2.1, pal.accent2.2, 46);
        pix.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Stroke::default() },
                        Transform::identity(), None);

        let icon = s.icon as f32;
        let top = DOCK_PAD + BTN_PAD;
        // hover highlight: accent 0.25 rounded 6 (CSS button:hover)
        if let Some(hv) = s.hover {
            let (hx, hw) = if hv == usize::MAX {
                (s.plus_x, 30.0)
            } else if let Some(b) = s.btns.get(hv) {
                (b.x, b.w)
            } else {
                (0.0, 0.0)
            };
            if hw > 0.0 {
                if let Some(rect) = Rect::from_xywh(hx, DOCK_PAD, hw, icon + 2.0 * BTN_PAD) {
                    let mut hp = Paint::default();
                    hp.anti_alias = true;
                    hp.set_color_rgba8(pal.accent.0, pal.accent.1, pal.accent.2, 64);
                    let rr = PathBuilder::from_rect(rect);
                    pix.fill_path(&rr, &hp, tiny_skia::FillRule::Winding,
                                  Transform::identity(), None);
                }
            }
        }
        for b in &s.btns {
            if let Some(ic) = &b.icon {
                pix.draw_pixmap(
                    (b.x + BTN_PAD) as i32,
                    top as i32,
                    ic.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            } else {
                self.text.draw(pix, b.x + BTN_PAD, top + icon * 0.65, icon * 0.4,
                               pal.fg, &b.label);
            }
        }
        if let Some(hint) = &s.hint {
            self.text.draw(pix, DOCK_PAD_X + 8.0, h / 2.0 + 4.0, 12.0, pal.sub, hint);
        }
        // ＋ as vector strokes — the fullwidth-plus glyph is tofu territory
        let cx = s.plus_x + 15.0;
        let cy = h / 2.0;
        let arm = (icon * 0.22).max(5.0);
        let mut pp = Paint::default();
        pp.anti_alias = true;
        pp.set_color_rgba8(pal.fg.0, pal.fg.1, pal.fg.2, 230);
        let mut lb = PathBuilder::new();
        lb.move_to(cx - arm, cy);
        lb.line_to(cx + arm, cy);
        lb.move_to(cx, cy - arm);
        lb.line_to(cx, cy + arm);
        pix.stroke_path(&lb.finish().unwrap(), &pp,
                        &Stroke { width: 2.0, ..Stroke::default() },
                        Transform::identity(), None);
    }

    // ----- visibility (python apply_visibility) -----
    fn hovered(&self, i: usize) -> bool {
        let d = &self.surfs[i];
        if d.pointer_in {
            return true;
        }
        // the dock's paired strip counts as hovering
        let want = match d.kind {
            EdgeKind::DockBottom => EdgeKind::StripBottom,
            EdgeKind::DockTop => EdgeKind::StripTop,
            _ => return false,
        };
        self.surfs
            .iter()
            .any(|s| s.kind == want && s.mon == d.mon && s.pointer_in)
    }

    fn want_visible(&self, i: usize) -> bool {
        let d = &self.surfs[i];
        let st = self.states.get(&d.mon).cloned().unwrap_or_default();
        if st.fullscreen {
            return false;
        }
        if !self.autohide || self.suspend || st.empty {
            return true;
        }
        self.hovered(i)
    }

    fn apply_visibility(&mut self, qh: &QueueHandle<App>, layer_shell: &LayerShell) {
        for i in 0..self.surfs.len() {
            if !self.surfs[i].is_dock() {
                continue;
            }
            let want = self.want_visible(i);
            let mapped = self.surfs[i].layer.is_some();
            if want && !mapped {
                self.map_surf(i, qh, layer_shell);
            } else if !want && mapped && self.surfs[i].hide_at.is_none() {
                // instant hides only for fullscreen/suspend transitions;
                // pointer-driven hides go through the 600 ms grace
                let st = self.surfs[i].mon.clone();
                let fs = self.states.get(&st).map(|s| s.fullscreen).unwrap_or(false);
                if fs {
                    self.unmap_surf(i);
                } else {
                    self.surfs[i].hide_at = Some(Instant::now() + HIDE_DELAY);
                }
            }
        }
    }

    // ----- state refresh -----
    fn refresh(&mut self, qh: &QueueHandle<App>, layer_shell: &LayerShell) {
        self.states = monitor_states();
        // reconcile: docks spawned during a transient hyprctl failure
        // carry stale names — respawn once real connectors are available
        let dock_mons: Vec<String> = self
            .surfs
            .iter()
            .filter(|s| s.is_dock())
            .map(|s| s.mon.clone())
            .collect();
        if !self.states.is_empty()
            && !dock_mons.is_empty()
            && !dock_mons.iter().any(|m| self.states.contains_key(m))
        {
            self.respawn_soon();
            return;
        }
        for i in 0..self.surfs.len() {
            if !self.surfs[i].is_dock() {
                continue;
            }
            let ws = self
                .states
                .get(&self.surfs[i].mon)
                .map(|s| s.ws)
                .unwrap_or(self.surfs[i].ws);
            if ws != self.surfs[i].ws {
                self.surfs[i].ws = ws;
                self.rebuild_dock(i);
            }
        }
        self.apply_visibility(qh, layer_shell);
    }

    fn refresh_soon(&mut self) {
        if self.refresh_at.is_none() {
            self.refresh_at = Some(Instant::now() + Duration::from_millis(120));
        }
    }

    fn respawn_soon(&mut self) {
        if self.respawn_at.is_none() {
            self.respawn_at = Some(Instant::now() + Duration::from_millis(500));
        }
    }

    fn on_pins_changed(&mut self) {
        self.pins = pins::load_pins();
        self.pins_mtime = pins::pins_mtime();
        for i in 0..self.surfs.len() {
            if self.surfs[i].is_dock() {
                self.rebuild_dock(i);
            }
        }
    }

    fn set_suspend(&mut self, on: bool) {
        self.suspend = on;
        // failsafe: resume even if the caller (arrange) dies
        self.suspend_until = on.then(|| Instant::now() + SUSPEND_MAX);
    }

    // ----- ctl -----
    fn on_ctl(&mut self, mut client: UnixStream) {
        let mut buf = [0u8; 256];
        let n = client.read(&mut buf).unwrap_or(0);
        let cmd = String::from_utf8_lossy(&buf[..n]).trim().to_string();
        let reply: Vec<u8> = match cmd.as_str() {
            "reload-theme" => {
                self.pal = hyprdesk::colors();
                for i in 0..self.surfs.len() {
                    if self.surfs[i].is_dock() {
                        self.rebuild_dock(i);
                    }
                }
                b"ok\n".to_vec()
            }
            "reload" => {
                self.on_pins_changed();
                self.respawn_soon();
                b"ok\n".to_vec()
            }
            "bar-pin" => {
                self.bar_pin = !self.bar_pin;
                if self.bar_pin && !bar_visible() {
                    bar_toggle();
                }
                if !self.bar_pin {
                    self.bar_polling = true; // released: resume auto-hide
                }
                if self.bar_pin { b"ok pinned\n".to_vec() } else { b"ok released\n".to_vec() }
            }
            "show-all" => {
                self.set_suspend(true);
                self.refresh_soon(); // apply_visibility runs on the refresh
                b"ok\n".to_vec()
            }
            "resume" => {
                self.set_suspend(false);
                self.refresh_soon();
                b"ok\n".to_vec()
            }
            "ping" | "" => b"ok\n".to_vec(),
            _ => b"err\n".to_vec(), // unknown must not masquerade as success
        };
        let _ = client.write_all(&reply);
    }

    // ----- pointer -----
    fn surf_of(&self, surf: &wl_surface::WlSurface) -> Option<usize> {
        self.surfs
            .iter()
            .position(|s| s.layer.as_ref().is_some_and(|l| l.wl_surface() == surf))
    }

    fn hit(&self, i: usize, x: f64) -> Option<usize> {
        let d = &self.surfs[i];
        if x as f32 >= d.plus_x && (x as f32) < d.plus_x + 30.0 {
            return Some(usize::MAX);
        }
        d.btns
            .iter()
            .position(|b| x as f32 >= b.x && (x as f32) < b.x + b.w)
    }
}

// ----- smart waybar (bar_smart) -----
// waybar is NOT our window: SIGUSR1 toggles it shown (level 2) / hidden
// (level 1 — verified on v0.15). While up we poll the cursor against its
// layer boxes and tuck it once the pointer has left the band.

fn bar_layers() -> Vec<(i64, f64, f64, f64, f64)> {
    let mut out = Vec::new();
    let data = hypr_json("layers");
    let Some(obj) = data.as_object() else { return out };
    for d in obj.values() {
        let Some(levels) = d.get("levels").and_then(|v| v.as_object()) else { continue };
        for (lvl, entries) in levels {
            let Some(arr) = entries.as_array() else { continue };
            for e in arr {
                // pid<=0 entries are Hyprland 0.55 corpses — always filter
                if e.get("namespace").and_then(|v| v.as_str()) == Some("waybar")
                    && e.get("pid").and_then(|v| v.as_i64()).unwrap_or(-1) > 0
                {
                    let g = |k: &str| e.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push((lvl.parse().unwrap_or(0), g("x"), g("y"), g("w"), g("h")));
                }
            }
        }
    }
    out
}

fn bar_visible() -> bool {
    bar_layers().iter().any(|(lvl, ..)| *lvl >= 2)
}

fn bar_toggle() {
    let _ = Command::new("pkill").args(["-USR1", "-x", "waybar"]).status();
}

fn bar_hovering(strip_hover: bool) -> bool {
    if strip_hover {
        return true;
    }
    let pos = hypr_json("cursorpos");
    let (cx, cy) = (
        pos.get("x").and_then(|v| v.as_f64()).unwrap_or(-1.0),
        pos.get("y").and_then(|v| v.as_f64()).unwrap_or(-1.0),
    );
    bar_layers().iter().any(|(lvl, x, y, w, h)| {
        *lvl >= 2 && *x <= cx && cx <= x + w && *y <= cy && cy <= y + h + STRIP_H as f64
    })
}

// ----- spawn -----

fn spawn_surfs(app: &mut App, qh: &QueueHandle<App>, layer_shell: &LayerShell) {
    app.surfs.clear();
    app.conf_reload();
    let mons: Vec<String> = app
        .output_state
        .outputs()
        .filter_map(|o| app.output_state.info(&o).and_then(|i| i.name))
        .collect();
    let states = monitor_states();
    for name in mons {
        let san = sanitize(&name);
        // bar_smart strip is independent of the dock being enabled here
        if app.bar_smart {
            app.surfs.push(new_surf(&name, &san, EdgeKind::BarStrip, 0, 0));
        }
        let per = hyprdesk::conf_get(&format!("dock_{san}"), "");
        let enabled = if per.is_empty() {
            hyprdesk::conf_get("apps", "on") == "on"
        } else {
            per == "on"
        };
        if !enabled {
            continue;
        }
        let icon = {
            let v = hyprdesk::conf_get(&format!("dock_{san}_size"), "");
            let v = if v.is_empty() { hyprdesk::conf_get("apps_size", "44") } else { v };
            v.parse::<i64>().unwrap_or(44).clamp(24, 96) as u32
        };
        let ws = states.get(&name).map(|s| s.ws).unwrap_or(1);
        app.surfs.push(new_surf(&name, &san, EdgeKind::DockBottom, icon, ws));
        let di = app.surfs.len() - 1;
        app.rebuild_dock(di);
        if app.autohide {
            app.surfs.push(new_surf(&name, &san, EdgeKind::StripBottom, 0, 0));
        }
        if app.dock_top {
            app.surfs.push(new_surf(&name, &san, EdgeKind::DockTop, icon, ws));
            let ti = app.surfs.len() - 1;
            app.rebuild_dock(ti);
            if app.autohide {
                app.surfs.push(new_surf(&name, &san, EdgeKind::StripTop, 0, 0));
            }
        }
    }
    app.states = states;
    // strips always map; docks map per visibility rules
    for i in 0..app.surfs.len() {
        if !app.surfs[i].is_dock() {
            app.map_surf(i, qh, layer_shell);
        }
    }
    app.apply_visibility(qh, layer_shell);
}

fn new_surf(mon: &str, san: &str, kind: EdgeKind, icon: u32, ws: i64) -> Surf {
    Surf {
        mon: mon.to_string(),
        san: san.to_string(),
        kind,
        layer: None,
        width: if matches!(kind, EdgeKind::DockBottom | EdgeKind::DockTop) { 60 } else { 0 },
        height: if matches!(kind, EdgeKind::DockBottom | EdgeKind::DockTop) {
            icon + (2.0 * BTN_PAD + 2.0 * DOCK_PAD) as u32
        } else {
            STRIP_H
        },
        configured: false,
        dirty: true,
        pointer_in: false,
        ws,
        icon,
        btns: Vec::new(),
        plus_x: DOCK_PAD_X,
        hint: None,
        hover: None,
        hide_at: None,
        dwell_since: None,
    }
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
            i32::from(!reply.starts_with("ok"))
        }
        Err(e) => {
            eprintln!("dock not reachable: {e}");
            1
        }
    }
}

enum Sig {
    Quit,
}

static mut RESPAWN_REQ: bool = false;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--ctl") {
        std::process::exit(ctl_client(args.get(i + 1).map(|s| s.as_str()).unwrap_or("ping")));
    }
    if args.iter().any(|a| a == "--picker") {
        // the picker is the python half of this hybrid — delegate
        let mut cmd = Command::new("hypr-appdock-picker");
        cmd.args(&args[1..]);
        let err = exec_replace(&mut cmd);
        eprintln!("hypr-appdock: cannot exec hypr-appdock-picker: {err}");
        std::process::exit(1);
    }
    // a live dock owns the socket — EXIT rather than run duplicate docks
    if UnixStream::connect(ctl_sock_path()).is_ok() {
        eprintln!("hypr-appdock: another instance is running — exiting");
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

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        shm,
        pool,
        pal: hyprdesk::colors(),
        text: hyprdesk::draw::Text::load(),
        surfs: Vec::new(),
        apps: apps::all_apps(),
        pins: pins::load_pins(),
        pins_mtime: pins::pins_mtime(),
        states: HashMap::new(),
        layer_kind: Layer::Top,
        autohide: true,
        dock_top: false,
        bar_smart: false,
        suspend: false,
        suspend_until: None,
        bar_pin: false,
        bar_polling: false,
        subtick: 0,
        refresh_at: None,
        respawn_at: None,
        sock_poll_fallback: false,
        pointer: None,
        pointer_surface: None,
        pointer_pos: (0.0, 0.0),
        exit: false,
    };
    let _ = event_queue.roundtrip(&mut app); // output names before spawning
    spawn_surfs(&mut app, &qh, &layer_shell);
    eprintln!(
        "hypr-appdock: {} dock(s), {} strip(s)",
        app.surfs.iter().filter(|s| s.is_dock()).count(),
        app.surfs.iter().filter(|s| !s.is_dock()).count()
    );

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");

    // hyprland event socket (.socket2) — a plain line stream
    let hypr_sock = {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
        let run = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        UnixStream::connect(format!("{run}/hypr/{sig}/.socket2.sock")).ok()
    };
    const EVENTS: [&str; 11] = [
        "workspacev2>>", "workspace>>", "focusedmon>>", "openwindow>>", "closewindow>>",
        "movewindow>>", "moveworkspace>>", "moveworkspacev2>>", "fullscreen>>",
        "monitorremoved>>", "monitoradded>>",
    ];
    match hypr_sock {
        Some(stream) => {
            let _ = stream.set_nonblocking(true);
            let mut linebuf = String::new();
            event_loop
                .handle()
                .insert_source(
                    calloop::generic::Generic::new(stream, calloop::Interest::READ,
                                                   calloop::Mode::Level),
                    move |_, stream, app: &mut App| {
                        let mut buf = [0u8; 4096];
                        loop {
                            match unsafe { stream.get_mut() }.read(&mut buf) {
                                Ok(0) => {
                                    // Hyprland restarted: EOF — drop the
                                    // watch or it busy-loops at 100% CPU
                                    app.sock_poll_fallback = true;
                                    return Ok(calloop::PostAction::Remove);
                                }
                                Ok(n) => {
                                    linebuf.push_str(&String::from_utf8_lossy(&buf[..n]));
                                    let mut rest = String::new();
                                    if let Some(pos) = linebuf.rfind('\n') {
                                        rest = linebuf[pos + 1..].to_string();
                                        for line in linebuf[..pos].lines() {
                                            if EVENTS.iter().any(|e| line.starts_with(e)) {
                                                app.refresh_soon();
                                            }
                                        }
                                    } else {
                                        rest = std::mem::take(&mut linebuf);
                                    }
                                    linebuf = rest;
                                }
                                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                    return Ok(calloop::PostAction::Continue)
                                }
                                Err(_) => return Ok(calloop::PostAction::Continue),
                            }
                        }
                    },
                )
                .expect("hypr socket source");
        }
        None => app.sock_poll_fallback = true,
    }

    let (tx, rx) = channel::channel::<Sig>();
    std::thread::spawn(move || {
        use signal_hook::consts::{SIGINT, SIGTERM};
        let mut sigs = signal_hook::iterator::Signals::new([SIGTERM, SIGINT]).unwrap();
        for _ in sigs.forever() {
            let _ = tx.send(Sig::Quit);
        }
    });
    event_loop
        .handle()
        .insert_source(rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(Sig::Quit) = ev {
                app.exit = true;
            }
        })
        .expect("signal source");

    // master 200 ms subtick: dwell, hide grace, bar poll, pins watch,
    // hide sanity, suspend failsafe, socket-poll fallback
    event_loop
        .handle()
        .insert_source(Timer::from_duration(SUBTICK), |_, _, app: &mut App| {
            app.subtick += 1;
            TimeoutAction::ToDuration(SUBTICK)
        })
        .expect("timer");

    // bar_smart: waybar starts visible at login — tuck it once mapped
    let mut bar_startup_at = app.bar_smart.then(|| Instant::now() + Duration::from_secs(2));
    let mut last_subtick = 0u64;
    let mut last_poll = Instant::now();
    let mut last_pins = Instant::now();
    let mut last_sanity = Instant::now();

    loop {
        event_loop
            .dispatch(Some(Duration::from_millis(100)), &mut app)
            .expect("dispatch");
        if let Some(l) = &listener {
            while let Ok((client, _)) = l.accept() {
                app.on_ctl(client);
            }
        }
        if unsafe { RESPAWN_REQ } {
            unsafe { RESPAWN_REQ = false };
        }
        let now = Instant::now();

        if app.subtick != last_subtick {
            last_subtick = app.subtick;

            // strip dwell → reveal
            for i in 0..app.surfs.len() {
                let (kind, fire, mon) = {
                    let s = &app.surfs[i];
                    let fire = s
                        .dwell_since
                        .is_some_and(|t| s.pointer_in && now.duration_since(t) >= DWELL);
                    (s.kind, fire, s.mon.clone())
                };
                if !fire {
                    continue;
                }
                app.surfs[i].dwell_since = None;
                if std::env::var("HYPRDOCK_DEBUG").is_ok() {
                    eprintln!("dwell fire: {mon} kind {}", kind as u8);
                }
                let st = app.states.get(&mon).cloned().unwrap_or_default();
                if st.fullscreen {
                    continue; // same rule as the docks
                }
                match kind {
                    EdgeKind::BarStrip => {
                        if !bar_visible() {
                            bar_toggle();
                        }
                        app.bar_polling = true;
                    }
                    EdgeKind::StripBottom | EdgeKind::StripTop => {
                        let want = if kind == EdgeKind::StripBottom {
                            EdgeKind::DockBottom
                        } else {
                            EdgeKind::DockTop
                        };
                        if let Some(di) = app
                            .surfs
                            .iter()
                            .position(|s| s.kind == want && s.mon == mon)
                        {
                            app.surfs[di].hide_at = None;
                            app.map_surf(di, &qh, &layer_shell);
                        }
                    }
                    _ => {}
                }
            }

            // hide grace expiry
            for i in 0..app.surfs.len() {
                if !app.surfs[i].is_dock() {
                    continue;
                }
                let due = app.surfs[i].hide_at.is_some_and(|t| now >= t);
                if !due {
                    continue;
                }
                app.surfs[i].hide_at = None;
                let st = app
                    .states
                    .get(&app.surfs[i].mon)
                    .cloned()
                    .unwrap_or_default();
                if app.autohide && !app.suspend && !app.hovered(i) && !st.empty {
                    app.unmap_surf(i);
                }
            }

            // smart-bar poll (400 ms) while revealed
            if app.bar_polling && now.duration_since(last_poll) >= Duration::from_millis(400) {
                last_poll = now;
                if !(app.bar_smart && bar_visible()) {
                    app.bar_polling = false;
                } else {
                    let strip_hover = app
                        .surfs
                        .iter()
                        .any(|s| s.kind == EdgeKind::BarStrip && s.pointer_in);
                    if !(app.suspend || app.bar_pin || bar_hovering(strip_hover)) {
                        bar_toggle();
                        app.bar_polling = false;
                    }
                }
            }
            if bar_startup_at.is_some_and(|t| now >= t) {
                bar_startup_at = None;
                app.bar_polling = true;
            }

            // pins watch (2 s)
            if now.duration_since(last_pins) >= Duration::from_secs(2) {
                last_pins = now;
                let mt = pins::pins_mtime();
                if mt != app.pins_mtime {
                    app.on_pins_changed();
                }
            }

            // hide sanity (5 s): never let a dock stick over windows; a
            // respawned waybar always comes back visible — tuck it
            if now.duration_since(last_sanity) >= Duration::from_secs(5) {
                last_sanity = now;
                if !app.suspend {
                    for i in 0..app.surfs.len() {
                        if app.surfs[i].is_dock()
                            && app.surfs[i].layer.is_some()
                            && app.autohide
                            && !app.hovered(i)
                        {
                            let st = app
                                .states
                                .get(&app.surfs[i].mon)
                                .cloned()
                                .unwrap_or_default();
                            if !st.empty {
                                app.unmap_surf(i);
                            }
                        }
                    }
                    if app.bar_smart && bar_visible() {
                        app.bar_polling = true;
                    }
                }
                if app.sock_poll_fallback {
                    app.refresh_soon(); // no event socket: poll instead
                }
            }

            // show-all failsafe
            if app.suspend && app.suspend_until.is_some_and(|t| now >= t) {
                app.set_suspend(false);
                app.apply_visibility(&qh, &layer_shell);
            }
        }

        if app.refresh_at.is_some_and(|t| now >= t) {
            app.refresh_at = None;
            app.refresh(&qh, &layer_shell);
        }
        if app.respawn_at.is_some_and(|t| now >= t) {
            app.respawn_at = None;
            spawn_surfs(&mut app, &qh, &layer_shell);
            eprintln!(
                "hypr-appdock: respawned ({} dock(s))",
                app.surfs.iter().filter(|s| s.is_dock()).count()
            );
        }
        if app.suspend {
            app.apply_visibility(&qh, &layer_shell); // show-all keeps mapping
        }

        for i in 0..app.surfs.len() {
            if app.surfs[i].dirty && app.surfs[i].configured {
                app.draw_surf(i);
            }
        }
        if app.exit {
            // never exit leaving the panel tucked
            if app.bar_smart && !bar_visible() {
                bar_toggle();
            }
            let _ = std::fs::remove_file(ctl_sock_path());
            std::process::exit(130);
        }
    }
}

fn exec_replace(cmd: &mut Command) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    cmd.exec()
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
        self.respawn_soon();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.respawn_soon();
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        for s in self.surfs.iter_mut() {
            if s.layer.as_ref().is_some_and(|l| l.wl_surface() == layer.wl_surface()) {
                s.layer = None;
                s.configured = false;
            }
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(i) = self
            .surfs
            .iter()
            .position(|s| s.layer.as_ref().is_some_and(|l| l.wl_surface() == layer.wl_surface()))
        {
            // adopt the granted size: strips ask for width 0 (= stretch),
            // and drawing from the stored 0 attached a 1×4 buffer — a
            // surface IS its buffer, so the strip was one pixel wide and
            // unhoverable (found live: zero pointer events ever arrived)
            let (w, h) = configure.new_size;
            if w > 0 {
                self.surfs[i].width = w;
            }
            if h > 0 {
                self.surfs[i].height = h;
            }
            self.surfs[i].configured = true;
            self.draw_surf(i);
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
            if std::env::var("HYPRDOCK_DEBUG").is_ok() {
                eprintln!("pointer registered: {}", self.pointer.is_some());
            }
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
            let idx = self.surf_of(&ev.surface);
            match &ev.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_surface = Some(ev.surface.clone());
                    self.pointer_pos = ev.position;
                    if let Some(i) = idx {
                        self.surfs[i].pointer_in = true;
                        if self.surfs[i].is_dock() {
                            self.surfs[i].hide_at = None;
                            let hv = self.hit(i, ev.position.0);
                            if hv != self.surfs[i].hover {
                                self.surfs[i].hover = hv;
                                self.surfs[i].dirty = true;
                            }
                        } else if self.surfs[i].dwell_since.is_none() {
                            self.surfs[i].dwell_since = Some(Instant::now());
                            if std::env::var("HYPRDOCK_DEBUG").is_ok() {
                                eprintln!("strip enter: {} {:?}", self.surfs[i].mon,
                                          self.surfs[i].kind as u8);
                            }
                        }
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_surface = None;
                    if let Some(i) = idx {
                        self.surfs[i].pointer_in = false;
                        self.surfs[i].dwell_since = None;
                        if self.surfs[i].is_dock() {
                            if self.surfs[i].hover.is_some() {
                                self.surfs[i].hover = None;
                                self.surfs[i].dirty = true;
                            }
                            self.surfs[i].hide_at = Some(Instant::now() + HIDE_DELAY);
                        } else if matches!(
                            self.surfs[i].kind,
                            EdgeKind::StripBottom | EdgeKind::StripTop
                        ) {
                            // pointer can leave the strip sideways without
                            // entering the dock — schedule the check there
                            let want = if self.surfs[i].kind == EdgeKind::StripBottom {
                                EdgeKind::DockBottom
                            } else {
                                EdgeKind::DockTop
                            };
                            let mon = self.surfs[i].mon.clone();
                            if let Some(di) = self
                                .surfs
                                .iter()
                                .position(|s| s.kind == want && s.mon == mon)
                            {
                                if self.surfs[di].layer.is_some() {
                                    self.surfs[di].hide_at =
                                        Some(Instant::now() + HIDE_DELAY);
                                }
                            }
                        }
                    }
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_pos = ev.position;
                    if let Some(i) = idx {
                        if self.surfs[i].is_dock() {
                            let hv = self.hit(i, ev.position.0);
                            if hv != self.surfs[i].hover {
                                self.surfs[i].hover = hv;
                                self.surfs[i].dirty = true;
                            }
                        }
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let Some(i) = idx else { continue };
                    if !self.surfs[i].is_dock() {
                        continue;
                    }
                    let hv = self.hit(i, self.pointer_pos.0);
                    match (hv, button) {
                        (Some(usize::MAX), 0x110) => {
                            // ＋: picker for THIS monitor (python spawn path)
                            let mon = self.surfs[i].mon.clone();
                            let me = std::env::current_exe()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| "hypr-appdock".into());
                            let _ = Command::new("setsid")
                                .args([me, "--picker".to_string(), "--monitor".to_string(), mon])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .spawn();
                        }
                        (Some(b), 0x110) => {
                            if let Some(entry) = self
                                .surfs[i]
                                .btns
                                .get(b)
                                .and_then(|btn| self.apps.get(&btn.stem))
                                .cloned()
                            {
                                apps::launch(&entry);
                            }
                        }
                        (Some(b), 0x111) => {
                            // right-click: unpin here
                            let (mon, ws) = (self.surfs[i].mon.clone(), self.surfs[i].ws);
                            let Some(stem) =
                                self.surfs[i].btns.get(b).map(|btn| btn.stem.clone())
                            else {
                                continue;
                            };
                            if let Some(state) = pins::pins_update(|st| {
                                let pins = pins::edit_list(st, &mon, ws);
                                pins.retain(|v| v.as_str() != Some(stem.as_str()));
                            }) {
                                self.pins = state;
                                self.pins_mtime = pins::pins_mtime();
                                // rebuild ALL docks (top + bottom mirror
                                // the same pins)
                                for j in 0..self.surfs.len() {
                                    if self.surfs[j].is_dock() {
                                        self.rebuild_dock(j);
                                    }
                                }
                            }
                        }
                        _ => {}
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
