//! Dump session_rows for the python-parity diff.
fn main() {
    for r in hyprdesk::session_rows() {
        println!("{}|{}|{}|{}", r.kind, r.label, r.sid, r.dir);
    }
}
