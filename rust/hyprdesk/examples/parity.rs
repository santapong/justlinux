fn main() {
    let procs = hyprdesk::claude_procs();
    println!("procs={}", procs.len());
    for p in &procs { println!("  pid={} interactive={} sid={} cwd={}", p.pid, p.interactive, &p.argv_sid[..8.min(p.argv_sid.len())], p.cwd); }
    let txs = hyprdesk::recent_transcripts(5);
    println!("transcripts={}", txs.len());
    for (mt, f) in &txs {
        let (cwd, prev) = hyprdesk::session_meta(f);
        let title = hyprdesk::session_title(f);
        println!("  {} | {} | {:.20} | {:.30}", hyprdesk::ago(*mt), cwd.rsplit('/').next().unwrap_or(""), title, prev);
    }
    let ag = hyprdesk::active_subagents(20.0);
    println!("active_subagents={:?}", ag);
    let pal = hyprdesk::colors();
    println!("palette bg={:?} accent={:?} sub={:?} good={:?}", pal.bg, pal.accent, pal.sub, pal.good);
}
