# DravenIQ voice — "Hey Draven" and spoken commands

`hypr-voice` is the ear of the DravenIQ Meta Harness: an always-on wake word,
then one spoken command, all offline. Nothing leaves the machine.

```
mic ─ pw-record ─► openWakeWord (ONNX, 1 thread) ─ wake ─► record until you pause
                                                          └► faster-whisper base.en (child process)
                                                             └► grammar (voice_intents.py) ─► action
```

## Shortcuts

| keys | does |
|---|---|
| `ALT+CTRL+V` | voice console — meters, what was heard, controls |
| `ALT+CTRL+M` | push-to-talk: listen now, no wake word |

## Try it

```sh
hypr-voice --say "open three tabs"        # grammar + actions, no microphone
hypr-voice --test 10                       # print the wake score for 10 s
systemctl --user enable --now hypr-voice   # or the switch on Settings → DravenIQ
journalctl --user -u hypr-voice -f         # what it heard, and how long whisper took
```

Say the wake word, wait for the tone / "listening…" toast, then speak.

| you say | it does |
|---|---|
| "open Draven" · "come here" | focus / spawn the harness |
| "open three tabs" · "new codex tab" · "open two hermes tabs" | `--new-tab` × N |
| "show the plan" | opens the plan viewer beside the active tab |
| "close tab" · "next tab" · "previous tab" | tab management |
| "settings" | Hypr Settings on the DravenIQ page |
| "hide Draven" | moves the window to the hidden special workspace |
| "tell Claude *list the files here*" · any sentence of 4+ words | **types** it into the active tab — no Enter |
| "send" · "go" | presses Enter |
| "stop listening" / "Draven, listen" | pause / resume |

Everything typed is echoed in a toast first; a mis-hearing is visible before it runs.

## The wake word

Until a custom model exists the daemon listens for the pretrained **"hey
jarvis"** (bundled with openWakeWord 0.4). To get **"Hey Draven"**:

1. Train a model — needs a GPU, so use one of:
   - <https://openwakeword.com/train> (hosted, type the phrase, download the `.onnx`), or
   - the 2026 Colab notebook <https://github.com/alfiedennen/openwakeword-colab-2026> (75–90 min on Colab).
2. Save it in `~/.local/share/hyprdesk/voice/` (any name, e.g. `draven.onnx` — a single-word wake phrase works but false-wakes more than “hey draven”).
3. `systemctl --user restart hypr-voice`. The Settings card shows which model is live.

Tune `voice_threshold` (default 0.6) in `~/.config/conky/widgets.conf` — raise it
if it wakes on podcasts, lower it if it ignores you. `hypr-voice --test` shows the
score your voice reaches.

## Settings keys (`widgets.conf`)

| key | default | meaning |
|---|---|---|
| `voice_enabled` | `0` | install.sh enables the service when 1 |
| `voice_source` | (WirePlumber default) | `pw-record --target` — a node id or name from `wpctl status` |
| `voice_threshold` | `0.6` | wake score needed twice in a row |
| `voice_lang` | `en` | `en` → `base.en`; anything else → multilingual `base` (slower) |

## Why Python here (language policy exception)

Residents are Rust. openWakeWord's feature pipeline (melspectrogram →
embedding → classifier, three ONNX graphs with trained weights) has no Rust
port with a model ecosystem, so this daemon is a **Python pilot** with a
measured budget (see `perf/voice-baseline.md`). The transcriber's 480 MB lives
in a child that exits after 5 idle minutes; the daemon itself holds only the
wake model. If the idle cost ever matters, the Rust lever is `ort` with the
same three graphs.
