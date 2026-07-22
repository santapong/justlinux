"""U4 acceptance meter — run inside ros2-arm:jazzy while hand_follow streams.
Records /joint_states + /arm_controller/joint_trajectory for DURATION s and
prints: per-joint amplitude, max consecutive-sample position delta (smoothness)
and both message rates.  Usage: python3 hand_accept.py [duration_s]"""
import sys
import time

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState
from trajectory_msgs.msg import JointTrajectory

DUR = float(sys.argv[1]) if len(sys.argv) > 1 else 20.0
JOINTS = [f"joint{i}" for i in range(1, 7)]


class Meter(Node):
    def __init__(self):
        super().__init__("hand_accept_meter")
        self.js = []          # (t, [pos joint1..6])
        self.traj_t = []
        self.create_subscription(JointState, "/joint_states", self.on_js, 100)
        self.create_subscription(JointTrajectory,
                                 "/arm_controller/joint_trajectory",
                                 self.on_traj, 100)

    def on_js(self, m):
        idx = {n: i for i, n in enumerate(m.name)}
        if all(j in idx for j in JOINTS):
            self.js.append((time.monotonic(),
                            [m.position[idx[j]] for j in JOINTS]))

    def on_traj(self, m):
        self.traj_t.append(time.monotonic())


def main():
    rclpy.init()
    n = Meter()
    t_end = time.monotonic() + DUR
    while time.monotonic() < t_end:
        rclpy.spin_once(n, timeout_sec=0.1)
    js, tt = n.js, n.traj_t
    print(f"samples: joint_states={len(js)}  traj_msgs={len(tt)}  dur={DUR}s")
    if len(js) < 10:
        print("FAIL: too few joint_states samples")
        return
    for j in range(6):
        vals = [p[1][j] for p in js]
        print(f"  joint{j+1}: min={min(vals):+.3f} max={max(vals):+.3f} "
              f"amplitude={max(vals)-min(vals):.3f} rad")
    max_d, max_dj = 0.0, -1
    for a, b in zip(js, js[1:]):
        for j in range(6):
            d = abs(b[1][j] - a[1][j])
            if d > max_d:
                max_d, max_dj = d, j
    span_js = js[-1][0] - js[0][0]
    print(f"max consecutive /joint_states delta: {max_d:.4f} rad "
          f"(joint{max_dj+1})  [accept: <= 0.15]")
    print(f"/joint_states rate: {(len(js)-1)/span_js:.1f} Hz")
    if len(tt) > 1:
        span = tt[-1] - tt[0]
        gaps = [b - a for a, b in zip(tt, tt[1:])]
        print(f"traj cmd rate: {(len(tt)-1)/span:.2f} Hz  "
              f"(max gap {max(gaps)*1e3:.0f} ms)  [accept: >= 15 Hz]")
    else:
        print("traj cmd rate: NO MESSAGES  [accept: >= 15 Hz] FAIL")
    amp1 = max(p[1][0] for p in js) - min(p[1][0] for p in js)
    ok = amp1 > 0.2 and max_d <= 0.15 and len(tt) > 1 and \
        (len(tt) - 1) / (tt[-1] - tt[0]) >= 15.0
    print("ACCEPT(a):", "PASS" if ok else "check-individual-lines")
    n.destroy_node()
    rclpy.shutdown()


if __name__ == "__main__":
    main()
