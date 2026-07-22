#!/opt/mpvenv/bin/python
"""hand_follow.py — webcam hand-following teleop for the 6-joint weld arm.

Runs under /opt/mpvenv/bin/python inside ros2-arm:jazzy (mediapipe venv with
--system-site-packages so rclpy/trajectory_msgs resolve). Streams single-point
JointTrajectory commands to /arm_controller/joint_trajectory at ~20 Hz using
the warm-start IK in /ros2_ws/tools/arm_ik.solve_track (U3).

AXIS MAPPING (fixed convention — mirror behavior)
  The raw /dev/video0 image is NOT mirrored, so every frame is first
  cv2.flip(frame, 1)'d into a selfie/mirror view. MediaPipe handedness labels
  assume exactly this mirrored view, so after the flip the labels are
  correct-as-seen. All mapping below is in the FLIPPED image:

    image u = wrist.x in [0,1], 0 = left edge of mirror view
    image v = wrist.y in [0,1], 0 = top edge
    scale s = wrist->middle-MCP pixel distance / image height (depth proxy;
              NORMALIZED IMAGE landmarks are used — hand_world_landmarks are
              hand-centered and do not translate with the hand)

    u (hand toward user's RIGHT, appears right in mirror) -> base_link +Y
        (viewed in RViz from the front, i.e. camera side looking at the arm
        along -X — and approximately in the default 45-deg orbit view —
        +Y is screen-right, so the tool moves to the same side of the screen
        as the hand: mirror behavior)
    v (hand UP, v decreasing)                              -> base_link +Z up
    s (hand TOWARD camera, bigger)                         -> base_link +X forward

  Workspace box (validated against arm_ik reachability, 125-pt grid: 0 clamps,
  worst IK err 1.45 mm; shoulder shell R_MAX = 0.4378 m about z = 0.155):
    x: 0.20 .. 0.32   y: -0.15 .. 0.15   z: 0.16 .. 0.36   (meters, base_link)
  Home target (loss decay): (0.26, 0.0, 0.26).

BEHAVIOR
  * LEFT hand only (label 'Left', score >= 0.6, after the flip); if two, the
    larger (closer) wins; Right hands ignored.
  * One-Euro filter (Casiez et al. 2012) per axis on the mapped target [m].
  * Loss: hold last target; after loss_hold_sec (2 s) decay slowly to home.
    (Re-)acquisition slews: per-tick target step clamp max_step_m — never
    teleports. Per-tick joint deltas always clamped to max_joint_step_rad;
    when solve_track's branch guard (U3) flags a far solution (elbow flip /
    catch-up) the target slew pauses while the joints close the gap.
  * /follow_enable (std_srvs/SetBool) pauses/resumes streaming (default ON);
    while paused the whole command state (cmd_target/q_cmd) is frozen so the
    arm truly holds — including through shutdown's final hold — and on resume
    the command state is re-seeded from /joint_states (no jump).
  * Preflight verifies camera, model file, /joint_states names == joint1..6
    (abort on mismatch), and a live controller. /joint_states staleness > 1 s
    is warned. Shutdown closes the landmarker and publishes a final hold.

TEST MODE
  --ros-args -p synthetic:=true replaces camera+mediapipe with a slow 3D sine
  sweep inside the box, exercising the identical filter->slew->IK->publish
  path (U4 acceptance).

LATENCY PROBE (U5)
  --ros-args -p latency_probe:=true logs a rolling-median timing report every
  3 s: capture (frame age when the vision thread picks it up), infer
  (mediapipe detect_for_video wall time), ik (arm_ik.solve_track's own
  info["time_ms"], measured every control tick in both modes), and publish
  (JointTrajectory publish() call). "pipeline" = capture+infer+ik, the
  budget named in the U5 acceptance gate (< 70 ms median, camera mode, no
  hand needed to see it — the vision thread still runs mediapipe on every
  frame regardless of whether a hand is found). capture/infer are camera-only
  (empty/"n/a" in synthetic mode, which has no vision thread). See
  docs/handfollow-run.md for how to correlate this with glass-to-RViz
  latency externally (phone slow-mo of hand + screen).
"""
import math
import os
import signal
import sys
import threading
import time
from collections import deque

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState
from std_srvs.srv import SetBool
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint

sys.path.insert(0, "/ros2_ws/tools")
import arm_ik

JOINTS = ["joint1", "joint2", "joint3", "joint4", "joint5", "joint6"]
JLIM = 2.85                      # URDF limit is +-2.9 — keep a margin
BOX = {"x": (0.20, 0.32), "y": (-0.15, 0.15), "z": (0.16, 0.36)}
HOME = (0.26, 0.0, 0.26)
TRAJ_TOPIC = "/arm_controller/joint_trajectory"
WRIST, MID_MCP = 0, 9            # mediapipe hand landmark indices


def check_joint_names(names):
    """None if `names` is exactly joint1..joint6 (any order), else an error
    string. Split out so the mismatch guard is unit-checkable (acceptance c)."""
    if sorted(names) != sorted(JOINTS):
        return ("/joint_states reports joints {} but this node requires "
                "exactly {} — wrong robot or wrong controller running? "
                "Expected the robot_arm_moveit_config demo.launch.py arm."
                .format(sorted(names), JOINTS))
    return None


class OneEuro:
    """One-Euro filter, Casiez et al. CHI 2012. Timestamps in seconds."""

    def __init__(self, min_cutoff=1.0, beta=0.0, d_cutoff=1.0):
        self.min_cutoff, self.beta, self.d_cutoff = min_cutoff, beta, d_cutoff
        self.x_prev = self.dx_prev = self.t_prev = None

    @staticmethod
    def _alpha(cutoff, dt):
        tau = 1.0 / (2.0 * math.pi * cutoff)
        return 1.0 / (1.0 + tau / dt)

    def reset(self):
        self.x_prev = self.dx_prev = self.t_prev = None

    def __call__(self, x, t):
        if self.t_prev is None:
            self.x_prev, self.dx_prev, self.t_prev = x, 0.0, t
            return x
        dt = max(t - self.t_prev, 1e-4)
        dx = (x - self.x_prev) / dt
        a_d = self._alpha(self.d_cutoff, dt)
        dx_hat = a_d * dx + (1.0 - a_d) * self.dx_prev
        a = self._alpha(self.min_cutoff + self.beta * abs(dx_hat), dt)
        x_hat = a * x + (1.0 - a) * self.x_prev
        self.x_prev, self.dx_prev, self.t_prev = x_hat, dx_hat, t
        return x_hat


class ThreadedCapture:
    """Camera reader on its own thread (latest-frame only) so the ~40 ms
    v4l2 read overlaps mediapipe inference. CAP_PROP_BUFFERSIZE=1 keeps the
    driver queue from serving stale frames."""

    def __init__(self, device):
        self.cap = cv2.VideoCapture(device, cv2.CAP_V4L2)
        if not self.cap.isOpened():
            raise RuntimeError(
                "camera {} failed to open — is the container started by the "
                "ros2-arm launcher (needs --device /dev/video0)?".format(device))
        self.cap.set(cv2.CAP_PROP_BUFFERSIZE, 1)
        ok, _ = self.cap.read()
        if not ok:
            self.cap.release()
            raise RuntimeError("camera {} opened but read() failed — device "
                               "busy or wrong node?".format(device))
        self.cond = threading.Condition()
        self.frame, self.t, self.seq = None, 0.0, 0
        self.running = True
        self.thread = threading.Thread(target=self._loop, daemon=True,
                                       name="capture")
        self.thread.start()

    def _loop(self):
        while self.running:
            ok, f = self.cap.read()
            if not ok:
                time.sleep(0.05)
                continue
            with self.cond:
                self.frame, self.t, self.seq = f, time.monotonic(), self.seq + 1
                self.cond.notify_all()

    def get(self, last_seq, timeout=0.5):
        """Newest (seq, frame, t) newer than last_seq, or None on timeout."""
        with self.cond:
            if self.seq == last_seq:
                self.cond.wait(timeout)
            if self.seq == last_seq or self.frame is None:
                return None
            return self.seq, self.frame, self.t

    def stop(self):
        self.running = False
        self.thread.join(timeout=2.0)
        self.cap.release()


class HandFollow(Node):
    def __init__(self):
        super().__init__("hand_follow")
        p = self.declare_parameter
        self.synthetic = p("synthetic", False).value
        self.camera = p("camera", "/dev/video0").value
        self.model_path = p("model_path", "/opt/models/hand_landmarker.task").value
        self.rate_hz = p("rate_hz", 20.0).value
        self.tfs = p("time_from_start", 0.12).value
        self.min_cutoff = p("min_cutoff", 1.0).value
        self.beta = p("beta", 0.5).value
        self.max_step = p("max_step_m", 0.03).value          # slew, m/tick
        self.decay_step = p("decay_step_m", 0.005).value     # home decay, m/tick
        self.loss_hold = p("loss_hold_sec", 2.0).value
        self.max_dq = p("max_joint_step_rad", 0.10).value    # 2 rad/s at 20 Hz
        self.hand_conf = p("hand_conf", 0.6).value
        self.s_near = p("s_near", 0.30).value                # scale -> x map
        self.s_far = p("s_far", 0.10).value
        self.latency_probe = p("latency_probe", False).value
        self.preview = p("preview", False).value   # live webcam window with
                                                   # detections drawn (X11)

        self.pub = self.create_publisher(JointTrajectory, TRAJ_TOPIC, 10)
        self.sub_js = self.create_subscription(JointState, "/joint_states",
                                               self._on_js, 10)
        self.srv = self.create_service(SetBool, "/follow_enable", self._on_enable)

        self.lock = threading.Lock()
        self.js_msg = None                 # latest JointState
        self.js_time = None                # monotonic stamp of it
        self.enabled = True
        self.running = True
        self.hand_target = None            # filtered mapped target (vision thread)
        self.last_seen = None              # monotonic time of last accepted hand
        self.cmd_target = None             # slewed target actually sent to IK
        self.q_cmd = None                  # last commanded joints
        self.track_state = "init"          # init/tracking/lost/homing
        self.ik_frozen = False             # last tick's IK flagged a jump
        self.capture = None
        self.landmarker = None
        self.filters = [OneEuro(self.min_cutoff, self.beta) for _ in range(3)]
        self.t0 = time.monotonic()
        self.timer = None
        self.n_pub = 0

        # latency probe (U5): rolling windows of per-stage timings in ms
        self.lat_capture = deque(maxlen=150)   # frame age at vision pickup
        self.lat_infer = deque(maxlen=150)     # detect_for_video wall time
        self.lat_ik = deque(maxlen=150)        # arm_ik.solve_track time_ms
        self.lat_publish = deque(maxlen=150)   # pub.publish() wall time
        self.lat_timer = None

    # ------------------------------------------------------------- callbacks
    def _on_js(self, msg):
        with self.lock:
            self.js_msg = msg
            self.js_time = time.monotonic()

    def _on_enable(self, req, resp):
        with self.lock:
            was, self.enabled = self.enabled, req.data
        if req.data and not was:
            self._reseed_from_js()         # never teleport after a pause
            self.get_logger().info("follow ENABLED (re-seeded from /joint_states)")
        elif not req.data and was:
            self.get_logger().info("follow DISABLED — arm holds, streaming paused")
        resp.success = True
        resp.message = "following enabled" if req.data else "following disabled"
        return resp

    def _reseed_from_js(self):
        with self.lock:
            msg = self.js_msg
        if msg is None:
            return
        pos = dict(zip(msg.name, msg.position))
        q = [pos[j] for j in JOINTS]
        with self.lock:
            self.q_cmd = q
            self.cmd_target = list(arm_ik.fk_tool0(q))

    # ------------------------------------------------------------- preflight
    def preflight(self):
        log = self.get_logger()
        if not self.synthetic:
            if not os.path.isfile(self.model_path):
                log.fatal("model file missing: {} — bake it into the image or "
                          "download hand_landmarker.task (float16) from the "
                          "mediapipe-models GCS bucket".format(self.model_path))
                return False
            try:
                self.capture = ThreadedCapture(self.camera)
            except RuntimeError as e:
                log.fatal(str(e))
                return False
            log.info("camera {} OK, model {} OK".format(self.camera,
                                                        self.model_path))
        # /joint_states (needed for names + seed) or at least a live controller
        deadline = time.monotonic() + 12.0
        while time.monotonic() < deadline and self.js_msg is None:
            rclpy.spin_once(self, timeout_sec=0.2)
        if self.js_msg is None:
            n_sub = self.pub.get_subscription_count()
            log.fatal("no /joint_states within 12 s (and {} subscriber(s) on "
                      "{}) — start the arm demo first: ros2-arm arm"
                      .format(n_sub, TRAJ_TOPIC))
            return False
        err = check_joint_names(list(self.js_msg.name))
        if err:
            log.fatal(err)
            return False
        if self.pub.get_subscription_count() == 0:
            log.warning("{} has no subscriber yet — arm_controller not up? "
                        "streaming anyway".format(TRAJ_TOPIC))
        self._reseed_from_js()
        log.info("preflight OK: joints {} live, seed q={} tool0=({:.3f},{:.3f},"
                 "{:.3f})".format(JOINTS, [round(v, 3) for v in self.q_cmd],
                                  *self.cmd_target))
        return True

    # ------------------------------------------------------------- vision
    def start(self):
        if not self.synthetic:
            base = mp_python.BaseOptions(model_asset_path=self.model_path)
            self.landmarker = mp_vision.HandLandmarker.create_from_options(
                mp_vision.HandLandmarkerOptions(
                    base_options=base,
                    running_mode=mp_vision.RunningMode.VIDEO,
                    num_hands=2,
                    min_hand_detection_confidence=0.5,
                    min_tracking_confidence=0.5))
            self.vision_thread = threading.Thread(target=self._vision_loop,
                                                  daemon=True, name="vision")
            self.vision_thread.start()
            self.get_logger().info(
                "LEFT-hand follow: conf>={}, box x{} y{} z{}, mirror mapping "
                "(hand right->+Y, up->+Z, closer->+X)".format(
                    self.hand_conf, BOX["x"], BOX["y"], BOX["z"]))
        else:
            self.get_logger().info("SYNTHETIC mode: 3D sine sweep in box "
                                   "x{} y{} z{}".format(BOX["x"], BOX["y"],
                                                        BOX["z"]))
        self.timer = self.create_timer(1.0 / self.rate_hz, self._tick)
        self.health_timer = self.create_timer(1.0, self._health)
        if self.latency_probe:
            self.lat_timer = self.create_timer(3.0, self._latency_report)
            self.get_logger().info("latency probe ON: reporting every 3s")

    def _vision_loop(self):
        seq, ts_prev = 0, -1
        while self.running and rclpy.ok():
            got = self.capture.get(seq)
            if got is None:
                continue
            seq, frame, t_frame = got
            if self.latency_probe:
                self.lat_capture.append((time.monotonic() - t_frame) * 1e3)
            frame = cv2.flip(frame, 1)          # mirror view (see header)
            rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            h, w = rgb.shape[:2]
            ts = max(int((t_frame - self.t0) * 1000), ts_prev + 1)
            ts_prev = ts
            t_infer0 = time.perf_counter() if self.latency_probe else None
            res = self.landmarker.detect_for_video(
                mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb), ts)
            if self.latency_probe:
                self.lat_infer.append((time.perf_counter() - t_infer0) * 1e3)
            best = None                          # (scale, wrist_u, wrist_v)
            for lms, handed in zip(res.hand_landmarks, res.handedness):
                cat = handed[0]
                du = (lms[MID_MCP].x - lms[WRIST].x) * w
                dv = (lms[MID_MCP].y - lms[WRIST].y) * h
                s = math.hypot(du, dv) / h       # depth proxy: hand size
                ok_hand = (cat.category_name == "Left"
                           and cat.score >= self.hand_conf)
                if ok_hand and (best is None or s > best[0]):
                    best = (s, lms[WRIST].x, lms[WRIST].y)  # two Lefts -> larger
                if self.preview:
                    self._draw_hand(frame, lms, cat, ok_hand, s, w, h)
            if self.preview:
                self._show_preview(frame, res, best)
            if best is None:
                if res.handedness:               # seen but rejected — say why
                    self.get_logger().info(
                        "hand(s) seen but none accepted (need Left >= {:.2f}):"
                        " {}".format(self.hand_conf, ", ".join(
                            "{} {:.2f}".format(hd[0].category_name,
                                               hd[0].score)
                            for hd in res.handedness)),
                        throttle_duration_sec=2.0)
                continue
            s, u, v = best
            u, v = min(max(u, 0.0), 1.0), min(max(v, 0.0), 1.0)
            k = min(max((s - self.s_far) / (self.s_near - self.s_far), 0.0), 1.0)
            raw = (BOX["x"][0] + k * (BOX["x"][1] - BOX["x"][0]),
                   BOX["y"][0] + u * (BOX["y"][1] - BOX["y"][0]),
                   BOX["z"][1] - v * (BOX["z"][1] - BOX["z"][0]))
            now = time.monotonic()
            with self.lock:
                gap = None if self.last_seen is None else now - self.last_seen
            if gap is None or gap > 0.5:         # (re-)acquired after a gap
                for f in self.filters:
                    f.reset()
                self.get_logger().info(
                    "hand ACQUIRED (scale {:.3f}) -> target ({:.3f},{:.3f},"
                    "{:.3f}) — slewing".format(s, *raw))
            filt = tuple(f(x, t_frame) for f, x in zip(self.filters, raw))
            with self.lock:
                self.hand_target = filt
                self.last_seen = now

    # ------------------------------------------------------------- preview
    def _draw_hand(self, frame, lms, cat, ok_hand, s, w, h):
        """Overlay one detected hand: landmarks + wrist->MCP line + verdict."""
        color = ((0, 220, 0) if ok_hand else            # green  = tracked
                 (0, 200, 255) if cat.category_name == "Left"
                 else (80, 80, 255))                    # yellow = low conf,
        for lm in lms:                                  # red    = Right/ignored
            cv2.circle(frame, (int(lm.x * w), int(lm.y * h)), 3, color, -1)
        wx, wy = int(lms[WRIST].x * w), int(lms[WRIST].y * h)
        mx, my = int(lms[MID_MCP].x * w), int(lms[MID_MCP].y * h)
        cv2.line(frame, (wx, wy), (mx, my), color, 2)
        tag = ("{} {:.2f} TRACKED s={:.2f}".format(cat.category_name,
                                                   cat.score, s)
               if ok_hand else
               "{} {:.2f} LOW CONF (<{:.2f})".format(cat.category_name,
                                                     cat.score, self.hand_conf)
               if cat.category_name == "Left" else
               "{} {:.2f} ignored".format(cat.category_name, cat.score))
        cv2.putText(frame, tag, (max(wx - 60, 2), max(wy - 12, 14)),
                    cv2.FONT_HERSHEY_SIMPLEX, 0.55, color, 2)

    def _show_preview(self, frame, res, best):
        n = len(res.hand_landmarks)
        status = ("TRACKING  scale {:.2f}".format(best[0]) if best else
                  "{} hand(s) seen, none accepted".format(n) if n else
                  "no hand in view — raise your LEFT hand")
        cv2.rectangle(frame, (0, 0), (frame.shape[1], 26), (30, 30, 30), -1)
        cv2.putText(frame, "[{}] {}".format(self.track_state, status), (8, 18),
                    cv2.FONT_HERSHEY_SIMPLEX, 0.55,
                    (0, 220, 0) if best else (0, 200, 255), 1)
        cv2.imshow("hand_follow — webcam (press q to close)", frame)
        if cv2.waitKey(1) & 0xFF == ord("q"):
            self.preview = False
            cv2.destroyAllWindows()

    # ------------------------------------------------------------- control
    def _desired(self, now):
        """(target, step_limit, state) for this tick, from mode + loss state."""
        if self.synthetic:
            t = now - self.t0
            return ((0.26 + 0.05 * math.sin(2 * math.pi * t / 13.0),
                     0.13 * math.sin(2 * math.pi * t / 9.0),
                     0.26 + 0.08 * math.sin(2 * math.pi * t / 7.0)),
                    self.max_step, "tracking")
        with self.lock:
            tgt, seen = self.hand_target, self.last_seen
        if seen is None:                       # never saw a hand
            ref = self.t0
        else:
            ref = seen
        lost_for = now - ref
        if tgt is not None and lost_for <= self.loss_hold:
            return tgt, self.max_step, "tracking"
        if lost_for <= self.loss_hold:         # startup grace: hold seed pose
            return None, 0.0, "init"
        return HOME, self.decay_step, "homing"  # lost > 2 s -> slow decay home

    def _tick(self):
        with self.lock:
            enabled = self.enabled
        if not enabled:
            return          # paused: freeze cmd_target/q_cmd so the arm truly
                            # holds — resume re-seeds from /joint_states and
                            # shutdown's final hold republishes the held pose
        now = time.monotonic()
        des, step_lim, state = self._desired(now)
        if state != self.track_state:
            if state == "homing":
                self.get_logger().warning(
                    "hand lost > {:.0f} s — decaying to home {}"
                    .format(self.loss_hold, HOME))
            elif state == "tracking" and self.track_state == "homing":
                self.get_logger().info("tracking resumed")
            self.track_state = state
        if des is not None and not self.ik_frozen:
            dx = [d - c for d, c in zip(des, self.cmd_target)]
            n = math.sqrt(sum(v * v for v in dx))
            if n > step_lim:                   # slew — never teleport
                dx = [v * step_lim / n for v in dx]
            self.cmd_target = [c + v for c, v in zip(self.cmd_target, dx)]
        q, info = arm_ik.solve_track(tuple(self.cmd_target), self.q_cmd)
        if self.latency_probe:
            self.lat_ik.append(info["time_ms"])
        if info["jump"]:                       # U3 branch guard tripped: the
            self.get_logger().warning(         # solution is far — pause the
                "IK branch jump at target ({:.3f},{:.3f},{:.3f}) — slewing "
                "joints toward it".format(*self.cmd_target),
                throttle_duration_sec=2.0)     # target and let joints catch up
        self.ik_frozen = info["jump"]
        # per-tick joint delta clamp (always): <= max_dq rad toward the IK
        # solution -> 2 rad/s at 20 Hz, under the 2.0 rad/s URDF velocity limit
        q = [max(-JLIM, min(JLIM,
             p + max(-self.max_dq, min(self.max_dq, v - p))))
             for p, v in zip(self.q_cmd, q)]
        self.q_cmd = q
        self._publish(q, self.tfs)
        self.n_pub += 1
        true_err = math.dist(arm_ik.fk_tool0(q), self.cmd_target)
        self.get_logger().info(
            "[{}] target ({:.3f},{:.3f},{:.3f}) err {:.1f} mm ik {:.2f} ms "
            "sent #{}".format(self.track_state, *self.cmd_target,
                              true_err * 1e3, info["time_ms"], self.n_pub),
            throttle_duration_sec=5.0)

    def _publish(self, q, tfs):
        t_pub0 = time.perf_counter() if self.latency_probe else None
        msg = JointTrajectory()
        msg.joint_names = list(JOINTS)
        pt = JointTrajectoryPoint()
        pt.positions = [float(v) for v in q]
        pt.time_from_start.sec = int(tfs)
        pt.time_from_start.nanosec = int((tfs - int(tfs)) * 1e9)
        msg.points.append(pt)
        self.pub.publish(msg)
        if self.latency_probe:
            self.lat_publish.append((time.perf_counter() - t_pub0) * 1e3)

    @staticmethod
    def _median(dq):
        if not dq:
            return None
        s = sorted(dq)
        return s[len(s) // 2]

    def _latency_report(self):
        c, i, k, u = (self._median(self.lat_capture), self._median(self.lat_infer),
                      self._median(self.lat_ik), self._median(self.lat_publish))
        fmt = lambda v: "{:.1f}ms".format(v) if v is not None else "n/a"
        pipeline = ("{:.1f}ms".format(c + i + k) if None not in (c, i, k)
                    else "n/a (no camera data yet)" if self.synthetic
                    else "n/a (waiting for first camera frame)")
        self.get_logger().info(
            "LATENCY (median, n_ik={}) capture={} infer={} ik={} publish={} "
            "-- pipeline(capture+infer+ik)={}"
            .format(len(self.lat_ik), fmt(c), fmt(i), fmt(k), fmt(u), pipeline))

    def _health(self):
        with self.lock:
            t_js = self.js_time
        if t_js is not None and time.monotonic() - t_js > 1.0:
            self.get_logger().warning(
                "/joint_states stale for {:.1f} s — controller down?"
                .format(time.monotonic() - t_js), throttle_duration_sec=5.0)

    # ------------------------------------------------------------- shutdown
    def shutdown(self):
        self.running = False
        if self.timer is not None:
            self.timer.cancel()
        if self.q_cmd is not None and rclpy.ok():
            self._publish(self.q_cmd, 0.3)     # final hold
            self.get_logger().info("published final hold, shutting down")
        if self.landmarker is not None:
            self.landmarker.close()
        if self.capture is not None:
            self.capture.stop()
            cv2.destroyAllWindows()            # no-op unless preview was on


def main():
    global cv2, mp, mp_python, mp_vision
    rclpy.init()
    node = HandFollow()
    if not node.synthetic:                     # lazy imports: synthetic mode
        import cv2                             # must run without camera stack
        import mediapipe as mp
        from mediapipe.tasks import python as mp_python
        from mediapipe.tasks.python import vision as mp_vision
    def _sigterm(*_):
        raise KeyboardInterrupt          # docker stop -> clean shutdown path
    signal.signal(signal.SIGTERM, _sigterm)
    try:
        if not node.preflight():
            node.destroy_node()
            rclpy.shutdown()
            sys.exit(1)
        node.start()
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        node.shutdown()
        node.destroy_node()
        if rclpy.ok():
            rclpy.shutdown()


if __name__ == "__main__":
    main()
