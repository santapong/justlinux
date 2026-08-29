# hypr-voice — cost baseline (29 Aug 2026)

Machine: 4 cores, no GPU. Measured with `hypr-voice --test 20` (prints its own
steady-state CPU after model load) on the MCHOSE V9 PRO headset mic.

| stage | cost | notes |
|---|---|---|
| wake model inference alone (bench, random audio) | 2.6 % of one core | openWakeWord 0.4, `ncpu=1`, any chunk size 80–320 ms |
| daemon steady state, 160 ms chunks, pw-record `--latency 160ms` | **4.8 % of one core ≈ 1.2 % of the machine** | the rest is pipe wakeups + numpy; 100 ms latency → 5.6 %, 320 ms chunks → no gain |
| daemon RSS | **228 MB** | onnxruntime + numpy floor; nothing else loaded |
| whisper `base.en` int8 (child process) | 1.5 s for 3 s of audio · 480 MB while alive | loaded on first command, exits after 5 idle minutes |
| wake → "listening…" toast | < 200 ms | two consecutive frames above threshold |

Budget written in the plan was ≤ 2 % / ≤ 120 MB; the Python pilot lands at
~5 % of one core / 228 MB. Accepted as the pilot cost (see docs/voice.md,
"Why Python here"); the lever if it matters is a Rust `ort` port of the same
three ONNX graphs, and a lower-rate pipe (`pw-record` cannot deliver
> 320 ms buffers without latency the wake word feels).

Things that did **not** help, measured: disabling ORT thread spinning (the
sessions already run one thread), bigger inference chunks.

Re-measure: `HYPR_VOICE_FRAMES=2 HYPR_VOICE_LATENCY=160ms hypr-voice --test 20`.
