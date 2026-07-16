//! justlinux — Hyprland desktop tools as one multi-call binary.
//!
//! Installed as symlinks that keep the original script names
//! (hypr-tools.sh, wallpaper.sh, …) so hyprland.conf keybinds, waybar
//! hooks and `pgrep -f` contracts keep working unchanged. The applet is
//! chosen from argv[0]; `justlinux <applet> [args]` also works.

mod colors;
mod hypr;
mod proc;
mod util;

mod applets {
    pub mod autohide;
    pub mod av_status;
    pub mod bar_toggle;
    pub mod fw_status;
    pub mod kitty_img;
    pub mod launcher;
    pub mod screenshot;
    pub mod settings;
    pub mod tools;
    pub mod wallpaper;
}

use std::process::ExitCode;

const APPLETS: &[(&str, &str)] = &[
    ("hypr-tools", "desktop control menus / dispatcher (ALT+D …)"),
    ("hypr-settings", "settings panel TUI (ALT+X)"),
    ("hypr-launcher", "app/window/wallpaper/tools launcher TUI (ALT+R …)"),
    ("wallpaper", "set wallpaper + recolor the whole desktop"),
    ("screenshot", "region|screen|all screenshots"),
    ("bar-toggle", "show/hide the top bar (ALT+B)"),
    ("waybar-autohide", "auto-hide daemon for waybar"),
    ("av-status", "waybar JSON status for ClamAV"),
    ("fw-status", "waybar JSON status for ufw"),
];

/// "wallpaper.sh" / "hypr-settings" / "justlinux" -> applet key
fn applet_name(argv0: &str) -> String {
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    base.strip_suffix(".sh").unwrap_or(base).to_string()
}

fn usage() -> ExitCode {
    eprintln!("justlinux — Hyprland desktop tools (multi-call binary)\n");
    eprintln!("usage: justlinux <applet> [args…]   or symlink an applet name to this binary\n");
    for (name, desc) in APPLETS {
        eprintln!("  {name:<16} {desc}");
    }
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    let mut name = applet_name(&args[0]);
    if name == "justlinux" {
        if args.len() < 2 {
            return usage();
        }
        name = applet_name(&args[1]);
        args.drain(0..2);
    } else {
        args.drain(0..1);
    }
    let rest: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match name.as_str() {
        "hypr-tools" => applets::tools::run(&rest),
        "hypr-settings" => applets::settings::run(&rest),
        "hypr-launcher" => applets::launcher::run(&rest),
        "wallpaper" => applets::wallpaper::run(&rest),
        "screenshot" => applets::screenshot::run(&rest),
        "bar-toggle" => applets::bar_toggle::run(),
        "waybar-autohide" => applets::autohide::run(),
        "av-status" => applets::av_status::run(),
        "fw-status" => applets::fw_status::run(),
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::applet_name;

    #[test]
    fn applet_names_strip_paths_and_sh() {
        assert_eq!(applet_name("/home/u/.local/bin/wallpaper.sh"), "wallpaper");
        assert_eq!(applet_name("hypr-settings"), "hypr-settings");
        assert_eq!(applet_name("./bar-toggle.sh"), "bar-toggle");
        assert_eq!(applet_name("justlinux"), "justlinux");
    }
}
