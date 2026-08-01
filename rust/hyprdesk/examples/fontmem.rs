fn rss() -> usize {
    std::fs::read_to_string("/proc/self/statm").ok()
        .and_then(|s| s.split_whitespace().nth(1).map(|p| p.parse::<usize>().unwrap_or(0)))
        .unwrap_or(0) * 4096 / 1048576
}
fn main() {
    println!("start rss={} MB", rss());
    let bytes = std::fs::read(hyprdesk::home()
        .join(".local/share/fonts/JetBrainsMonoNerd/JetBrainsMonoNerdFont-Regular.ttf")).unwrap();
    println!("after read ({} MB file) rss={} MB", bytes.len()/1048576, rss());
    let f = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).unwrap();
    println!("after Font::from_bytes rss={} MB  glyphs={}", rss(), f.glyph_count());
}
