//! Acceptance probe for the capture half: run the real audio thread for a
//! few seconds while the caller plays (or doesn't play) sound, then print
//! the 12 band levels. Silence must read ~0 everywhere; a 440 Hz tone must
//! light the low-mid bands. Run:  cargo run --release -p hypr-viz --example
//! audio_probe -- <seconds>

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[path = "../src/audio.rs"]
mod audio;

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let levels = Arc::new(Mutex::new([0f32; audio::BANDS]));
    let stop = Arc::new(AtomicBool::new(false));
    audio::spawn(levels.clone(), stop.clone());
    std::thread::sleep(std::time::Duration::from_secs(secs));
    let lv = *levels.lock().unwrap();
    stop.store(true, Ordering::Relaxed);
    let peak = lv.iter().cloned().fold(0f32, f32::max);
    let line: Vec<String> = lv.iter().map(|v| format!("{v:.2}")).collect();
    println!("bands: [{}]  peak: {peak:.2}", line.join(", "));
}
