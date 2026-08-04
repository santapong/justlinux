//! Offscreen acceptance: stage every state the handoff names and LOOK.
#[path = "../src/scene.rs"]
mod scene;
#[path = "../src/sprites.rs"]
mod sprites;

use scene::{Pass, Scene, SessionRow, WorkState};

fn main() {
    let pal = hyprdesk::colors();
    let text = hyprdesk::draw::Text::load();
    let mut sc = Scene::new();
    sc.w = 1600.0;
    sc.h = 900.0;
    let mk = |key: &str, title: &str, st: WorkState, subs: usize| SessionRow {
        key: key.into(),
        pid: 0,
        sid: key.into(),
        title: title.into(),
        cwd: "/home/santapong/hyprland-dots".into(),
        state: st,
        subagents: subs,
    };
    // stage: working+subagents, needs-you (hovered), asleep, reading, 2 ghosts
    for (i, row) in [
        mk("a", "Analyze 3D scanning tooling for the mirror rig", WorkState::Working, 2),
        mk("b", "Find Thai language typography for the card host", WorkState::NeedsYou, 0),
        mk("c", "Review ReCall GitHub pipeline", WorkState::Asleep, 0),
        mk("d", "fix-the-parser — Constraint span units", WorkState::Reading, 0),
    ]
    .into_iter()
    .enumerate()
    {
        sc.actors.push(scene::Actor {
            desk: i,
            pos: (0.0, 0.0),
            path: Vec::new(),
            phase: scene::Phase::AtDesk,
            row,
        });
    }
    sc.ghosts.push(scene::Ghost {
        desk: 4,
        sid: "g1".into(),
        cwd: String::new(),
        title: "talos-full-body-mirror".into(),
        age: "2h ago".into(),
    });
    sc.ghosts.push(scene::Ghost {
        desk: 5,
        sid: "g2".into(),
        cwd: String::new(),
        title: "Explore Line MCP for the launcher".into(),
        age: "1d ago".into(),
    });
    sc.hover = Some(1); // the needs-you desk is hovered (bottom strip shows)
    sc.meeting = 3;
    sc.meeting_title = "rust-migration rung 2 — parity harness".into();

    let mut pix = tiny_skia::Pixmap::new(1600, 900).unwrap();
    // wallpaper stand-in so the glass reads like on the desktop
    let mut p = tiny_skia::Paint::default();
    p.set_color(tiny_skia::Color::from_rgba8(38, 32, 30, 255));
    pix.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, 1600.0, 900.0).unwrap(),
        &p,
        tiny_skia::Transform::identity(),
        None,
    );
    sc.render(&mut pix, &pal, &text, Pass::Static);
    sc.render(&mut pix, &pal, &text, Pass::Dynamic);
    pix.save_png(std::env::args().nth(1).unwrap_or("office_scene.png".into()))
        .unwrap();
    println!("rendered");
}
