//! The capture half, unchanged in shape from the Python: pw-record on the
//! default sink's monitor, 12 log-spaced Goertzel bands, decay max-hold.
//! PipeWire suspends idle sinks and ends the stream — reconnect with a
//! backoff instead of dying (the python v1 bug, kept fixed here).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const BANDS: usize = 12;
const RATE: f64 = 44100.0;
const CHUNK: usize = 1024;
const DECAY: f32 = 0.72;

pub fn spawn(levels: Arc<Mutex<[f32; BANDS]>>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let freqs: Vec<f64> = (0..BANDS)
            .map(|i| 50.0 * (12000.0f64 / 50.0).powf(i as f64 / (BANDS - 1) as f64))
            .collect();
        let coeffs: Vec<f64> = freqs
            .iter()
            .map(|f| 2.0 * (2.0 * std::f64::consts::PI * f / RATE).cos())
            .collect();
        let mut buf = vec![0u8; CHUNK * 2];
        while !stop.load(Ordering::Relaxed) {
            let child = Command::new("pw-record")
                .args([
                    "-P",
                    "{ stream.capture.sink = true }",
                    "--rate",
                    "44100",
                    "--channels",
                    "1",
                    "--format",
                    "s16",
                    "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                std::thread::sleep(Duration::from_millis(1500));
                continue;
            };
            let mut out = child.stdout.take().unwrap();
            loop {
                if stop.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    return;
                }
                if out.read_exact(&mut buf).is_err() {
                    break; // stream ended: reconnect
                }
                let samples: Vec<f64> = buf
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]) as f64)
                    .collect();
                let mut new = [0f32; BANDS];
                for (i, &c) in coeffs.iter().enumerate() {
                    let (mut s1, mut s2) = (0f64, 0f64);
                    for &x in &samples {
                        let s = c * s1 - s2 + x;
                        s2 = s1;
                        s1 = s;
                    }
                    let p = s1 * s1 + s2 * s2 - c * s1 * s2;
                    let full = (CHUNK as f64 * 32768.0).powi(2);
                    let db = 10.0 * (p / full + 1e-12).log10();
                    new[i] = (((db + 55.0) / 50.0) as f32).clamp(0.0, 1.0);
                }
                let mut lv = levels.lock().unwrap();
                for i in 0..BANDS {
                    lv[i] = new[i].max(lv[i] * DECAY);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            *levels.lock().unwrap() = [0.0; BANDS]; // flat bars while silent
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
}
