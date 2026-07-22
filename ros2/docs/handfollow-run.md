# Webcam Hand-Following Teleop — Run Guide

One command brings up the 6-joint weld arm in RViz (MoveIt2 demo,
`demo.launch.py`) plus `hand_follow.py`, which streams single-point
`JointTrajectory` commands to `/arm_controller/joint_trajectory` so the arm
follows your **left hand** in front of `/dev/video0`. See
`docs/handfollow-inception.md` for the design/theory writeup and
`src/robot_arm_moveit_config/scripts/hand_follow.py`'s own docstring for the
axis-mapping and behavior spec this guide summarizes.

## Prerequisites

- `ros2-arm:jazzy` image built (`docker build -t ros2-arm:jazzy ~/ros2_ws/.docker-arm/`)
  — it bakes the `/opt/mpvenv` mediapipe venv and
  `/opt/models/hand_landmarker.task` (7.8 MB) at build time, so no network
  is needed at run time.
- `/dev/video0` present and not held by another process (`fuser /dev/video0`
  should print nothing).
- No other `ros2-arm` containers already running (the launcher does not
  stack multiple arm demos on the same `arm_controller`); `ros2-arm stop`
  or `docker ps` + `docker rm -f <name>` to clear old ones first.
- Nothing else on the 4-core host is CPU-bound (RViz + move_group + mediapipe
  inference together want real cores). Stop other demo containers
  (`armloop`, `cellloop`, `fullsys`, `mtcdemo`, `arduino`) if the box is
  loaded.

## One-command start

```sh
ros2-arm handfollow              # camera + mediapipe, live hand tracking
ros2-arm handfollow synthetic    # no camera: deterministic 3D sine-sweep
                                  # target exercises the identical
                                  # filter -> slew -> IK -> publish path
```

Both bring up the full MoveIt2 demo (RViz + mock `ros2_control`) and
`hand_follow.py` in a **single container**, in one invocation, from cold.
Internally this runs
`ros2 launch .../launch/handfollow.launch.py [synthetic:=true]`: a
`TimerAction` gives `demo.launch.py` 22 s to bring up `move_group` +
`arm_controller` + RViz (the proven margin already used by `ros2-arm loop`),
then starts `hand_follow.py` under `/opt/mpvenv/bin/python` (the mediapipe
venv). `hand_follow.py` then runs its own preflight (camera + model file +
`/joint_states` with the exact 6 expected joint names, up to another 12 s),
so worst case first output is roughly 30-35 s after the command.

What "up" looks like in the container log:
```
[python-N] [INFO] [...] [hand_follow]: preflight OK: joints [...] live, seed q=[...] tool0=(...)
[python-N] [INFO] [...] [hand_follow]: LEFT-hand follow: conf>=0.6, box x(0.2, 0.32) y(-0.15, 0.15) z(0.16, 0.36), mirror mapping ...
```
RViz shows the arm; move your left hand in front of the camera (right hand
is ignored) and it follows, mirror-style (hand moves toward your right ->
arm moves to its +Y / screen-right).

To stop: `docker rm -f <container>` (Ctrl-C in the foreground terminal also
works — the container is `--rm`, so no cleanup needed after).

### Pause/resume without killing the node

```sh
# from a second ros2-arm container (same host, --net=host + --ipc=host
# shares the DDS graph):
ros2 service call /follow_enable std_srvs/srv/SetBool "{data: false}"   # pause, arm holds
ros2 service call /follow_enable std_srvs/srv/SetBool "{data: true}"    # resume — re-seeds from /joint_states, no jump
```

## Tuning knobs

All are `ros2 launch ... key:=value` overrides (append after `handfollow` /
`handfollow synthetic` on the `ros2-arm` command line) or, for ad-hoc
testing, `--ros-args -p key:=value` if invoking `hand_follow.py` directly.
Defaults are the values validated in U4/U5.

| Param | Default | Effect |
|---|---|---|
| `min_cutoff` | 1.0 | One-Euro filter: lower = smoother at rest but more lag on fast moves |
| `beta` | 0.5 | One-Euro filter: higher = less lag on fast moves but more jitter at rest |
| `rate_hz` | 20.0 | Control tick / command rate to `/arm_controller/joint_trajectory` |
| `time_from_start` | 0.12 | Seconds-ahead stamp on each streamed trajectory point |
| `max_step_m` | 0.03 | Per-tick target slew clamp while tracking (m/tick) — raise for snappier catch-up, lower to fight overshoot |
| `max_joint_step_rad` | 0.10 | Per-tick joint-space delta clamp (0.10 rad @ 20 Hz = 2 rad/s, under the 2.0 rad/s URDF velocity limit) |
| `decay_step_m` | 0.005 | Slew rate while decaying to HOME after a >2s hand loss |
| `loss_hold_sec` | 2.0 | How long to hold the last target after losing the hand before decaying home |
| `hand_conf` | 0.6 | Minimum handedness-classification confidence to accept a Left hand |
| `s_near` / `s_far` | 0.30 / 0.10 | Hand-size (wrist->middle-MCP / image height) range mapped to the near/far end of the X (depth) axis — recalibrate per-user if depth response feels off |

**Workspace box** (`BOX` in `hand_follow.py`, not a launch param — edit the
constant and restart): `x: 0.20..0.32  y: -0.15..0.15  z: 0.16..0.36` (m,
`base_link`), validated against `arm_ik` reachability (125-pt grid, 0
clamps, worst-case IK error 1.45 mm). `HOME = (0.26, 0.0, 0.26)`.

Example: snappier tracking, more tolerant of a shaky hand:
```sh
ros2-arm handfollow min_cutoff:=1.5 beta:=0.8
```

## Latency instrumentation (`latency_probe:=true`)

```sh
ros2-arm handfollow latency_probe:=true              # camera mode + probe
ros2-arm handfollow synthetic latency_probe:=true     # synthetic + probe
```

Every 3 s, `hand_follow.py` logs a rolling-median (last <=150 samples per
stage) report:
```
LATENCY (median, n_ik=812) capture=12.3ms infer=27.1ms ik=1.8ms publish=0.1ms -- pipeline(capture+infer+ik)=41.2ms
```
- **capture** — frame age when the vision thread picks it up off the
  threaded `cv2.VideoCapture` (queueing/scheduling delay, camera-mode only).
- **infer** — `HandLandmarker.detect_for_video()` wall time (camera-mode
  only; runs on every frame whether or not a hand is found, so this is
  measurable with no hand in view).
- **ik** — `arm_ik.solve_track()`'s own `time_ms` (warm-start descent),
  measured every control tick in **both** camera and synthetic mode.
- **publish** — the `JointTrajectory` `publish()` call itself (not part of
  the acceptance budget; included for completeness — DDS publish is
  fire-and-forget and should stay near 0).
- **pipeline** = capture + infer + ik. This is the U5 acceptance metric:
  median **< 70 ms in camera mode, no hand required** (the vision loop and
  control tick both run continuously and independently of hand presence).
  In synthetic mode capture/infer don't exist (no camera thread), so
  pipeline reads `n/a` there by design — synthetic mode only exercises ik
  and the downstream control path, not the vision pipeline.

capture and infer come from the async vision thread; ik/publish come from
the fixed-rate control tick. They are not causally chained per-frame (that
would need per-frame sequence tracking through both threads, out of scope
for this budget check) — this is a **stage-timing budget**, not a strict
per-frame trace. It answers "is any stage secretly slow," which is what the
acceptance gate needs.

### Measuring true glass-to-RViz latency (deferred, needs a human + camera)

The pipeline-budget log bounds the *processing* latency but not the full
human-perceived glass-to-RViz delay (which also includes the ~20 Hz
JointTrajectory interpolation period and RViz's own render/TF pipeline).
To measure it directly, since this needs a live hand and is explicitly
deferred to Operation:

1. Put the RViz window and your moving hand in the same phone camera frame
   (e.g. hand in front of the webcam, RViz visible on a monitor behind/beside
   it, both in frame).
2. Record in slow-motion (iPhone/Android 120-240 fps slow-mo mode).
3. Make a sharp, distinctive hand motion (a fast stop-start, not a slow
   sweep — you want one identifiable event, e.g. a snap-to-stop).
4. Scrub the slow-mo footage frame-by-frame: find the frame where the hand
   motion event happens, and the frame where the RViz arm visibly starts
   responding to it. The frame-count gap x (1/recorded fps) = glass-to-RViz
   latency.
5. Cross-check against the pipeline-budget log from the same run
   (`latency_probe:=true`): glass-to-RViz should be pipeline-budget +
   roughly one `time_from_start` period (0.12 s default) + RViz's own
   render latency (~1 render frame, usually <33 ms) — if the measured gap
   is much larger than that sum, something outside the instrumented stages
   (RViz, TF, or contention from other processes on the 4-core host) is
   adding delay.

## Troubleshooting

**Camera busy / fails to open**
```
camera /dev/video0 failed to open — is the container started by the
ros2-arm launcher (needs --device /dev/video0)?
```
Check `fuser /dev/video0` for another process (another `ros2-arm` container,
`cheese`, `zoom`, browser camera permission, etc.) and kill/close it. If it's
a stray previous `ros2-arm` container: `docker ps -a` then `docker rm -f
<name>`.

**Model file missing**
```
model file missing: /opt/models/hand_landmarker.task — bake it into the
image or download hand_landmarker.task (float16) from the mediapipe-models
GCS bucket
```
The image wasn't built with the current Dockerfile, or was built before the
model-bake step was added. Rebuild:
`docker build -t ros2-arm:jazzy ~/ros2_ws/.docker-arm/`. To sanity-check an
existing image without a full run: `docker run --rm ros2-arm:jazzy sha256sum
/opt/models/hand_landmarker.task` and compare against
`/opt/models/hand_landmarker.task.sha256` baked alongside it.

**Controller down / `/joint_states` mismatch**
```
no /joint_states within 12 s (and 0 subscriber(s) on
/arm_controller/joint_trajectory) — start the arm demo first: ros2-arm arm
```
```
/joint_states reports joints [...] but this node requires exactly
['joint1', ..., 'joint6'] — wrong robot or wrong controller running?
```
Under `ros2-arm handfollow` this means `demo.launch.py` didn't come up in
time (loaded host, check `docker logs <container>` for the `move_group`/
`spawner` lines) or crashed. Give it longer, check `docker logs` for
tracebacks, or run `ros2-arm arm` standalone first to confirm the demo
itself is healthy on this host before layering hand-following on top. If
you see this while running `hand_follow.py` by hand (not via
`handfollow.launch.py`) against a *different* arm/package, it means you
pointed it at the wrong robot.

**`/joint_states` goes stale mid-run**
```
/joint_states stale for 1.3 s — controller down?
```
Logged (throttled to once/5s) if the controller stops publishing — usually
means `arm_controller` crashed or was killed; check `docker logs` for the
`ros2_control_node` lines.

**Command rate too low / jerky motion**
Check CPU contention first (`docker stats`, `nproc` — this is a 4-core
host and RViz + move_group + mediapipe inference + the control tick all
compete). Stop other demo containers. If genuinely CPU-bound even alone,
lower `rate_hz` before loosening the IK/filter budget — the U3 warm-start
IK measures ~2 ms/solve so it is not usually the bottleneck; mediapipe
inference (measured ~28 ms/frame, 2 hands, i3-9100) is.

**IK branch-jump warnings**
```
IK branch jump at target (...) — slewing joints toward it
```
Expected occasionally on fast hand motion or right after (re-)acquisition —
U3's guard is doing its job (pausing the target slew while joints catch up
via the per-tick `max_joint_step_rad` clamp). Frequent/continuous jumps
during normal slow motion would indicate `max_step_m` is set too high for
the workspace box size, or a workspace-box edit made two adjacent target
regions correspond to different IK branches.

## Deferred: live-hand tuning procedure (Operation)

U5 validates the pipeline mechanically (synthetic-mode command rate,
camera-mode pipeline-budget with no hand). Tuning against a **real** hand —
confirming the mirror mapping feels intuitive, the workspace box covers a
comfortable range of motion, and `min_cutoff`/`beta` feel right for a human
hand's actual speed/jitter profile — needs a human in the loop and is
deferred to Operation. Suggested procedure when picking that up:

1. `ros2-arm handfollow` (camera mode, defaults).
2. Slowly move your left hand across the camera's field of view at a
   comfortable distance; confirm the mirror mapping matches intuition
   (hand right -> arm swings to its +Y / screen-right in RViz's default
   view; hand up -> arm up; hand closer to camera -> arm reaches forward/+X).
3. At rest (hand still), watch for jitter in RViz. If visible, lower
   `min_cutoff` (e.g. `0.7`).
4. Make a fast hand motion; watch for lag. If the arm feels sluggish to
   catch up, raise `beta` (e.g. `0.8`) before lowering `min_cutoff` further.
5. Test the edges of `BOX` — hold your hand at the frame edges / as
   close/far as comfortable — confirm no clamped-target snapping or
   IK branch-jump warnings under normal motion; if the box feels too small
   or too large for a natural range of motion, edit the `BOX` constant in
   `hand_follow.py` (re-validate against `arm_ik` reachability, per the
   inception doc's 125-pt-grid method, before trusting a resized box).
6. Test hand loss: move your hand out of frame, confirm a 2 s hold then a
   slow decay to `HOME`; move back in, confirm a slew-in (no teleport/snap).
7. Record final tuned values as the new launch-argument defaults (or a
   documented per-user override line here) once satisfied.
