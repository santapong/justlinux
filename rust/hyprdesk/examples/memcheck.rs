fn rss() -> usize {
    std::fs::read_to_string("/proc/self/statm").ok()
        .and_then(|s| s.split_whitespace().nth(1).map(|p| p.parse::<usize>().unwrap_or(0)))
        .unwrap_or(0) * 4096 / 1048576
}
fn main() {
    let big = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    println!("start           rss={} MB", rss());
    let t = hyprdesk::session_title(&big);
    println!("after title     rss={} MB  (title={:.20})", rss(), t);
    let (c, p) = hyprdesk::session_meta(&big);
    println!("after meta      rss={} MB  (cwd={:.15} prev={:.15})", rss(), c, p);
    let _ = hyprdesk::recent_transcripts(30);
    println!("after recents   rss={} MB", rss());
    let _ = hyprdesk::active_subagents(20.0);
    println!("after subagents rss={} MB", rss());
}
