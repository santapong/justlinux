"""U3: validate arm_ik FK against live TF.  Run inside a ros2-arm container
while the robot_arm demo is up.  Drives the arm through several poses via
/arm_controller/joint_trajectory, then compares fk_tool0(/joint_states) with
TF base_link->tool0."""
import math
import sys
import time

import rclpy
from rclpy.node import Node
from rclpy.time import Time
from sensor_msgs.msg import JointState
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint
from tf2_ros import Buffer, TransformListener

sys.path.insert(0, "/ros2_ws/tools")
import arm_ik as ik

JOINTS = [f"joint{i}" for i in range(1, 7)]
POSES = [
    [0.0, 0.8, 0.9, 0.0, 1.0, 0.0],
    [0.5, 0.5, 1.2, 0.3, 0.8, 0.4],
    [-0.6, 1.1, 0.6, -0.4, 1.3, -0.5],
    [0.9, 0.3, 0.4, 0.8, -0.6, 1.0],
]


class Check(Node):
    def __init__(self):
        super().__init__("u3_fk_tf_check")
        self.js = None
        self.sub = self.create_subscription(JointState, "/joint_states",
                                            self._cb, 10)
        self.pub = self.create_publisher(JointTrajectory,
                                         "/arm_controller/joint_trajectory", 10)
        self.buf = Buffer()
        self.tfl = TransformListener(self.buf, self)

    def _cb(self, msg):
        self.js = msg

    def q_now(self):
        m = dict(zip(self.js.name, self.js.position))
        return [m[j] for j in JOINTS]

    def goto(self, q, dur=2.5):
        msg = JointTrajectory()
        msg.joint_names = JOINTS
        pt = JointTrajectoryPoint()
        pt.positions = [float(v) for v in q]
        pt.time_from_start.sec = int(dur)
        pt.time_from_start.nanosec = int((dur % 1) * 1e9)
        msg.points = [pt]
        self.pub.publish(msg)


def spin_for(node, sec):
    end = time.monotonic() + sec
    while time.monotonic() < end:
        rclpy.spin_once(node, timeout_sec=0.05)


def main():
    rclpy.init()
    node = Check()
    spin_for(node, 3.0)                      # discovery + first joint_states
    if node.js is None:
        print("FAIL: no /joint_states"); return
    worst = 0.0
    for i, pose in enumerate(POSES):
        node.goto(pose)
        spin_for(node, 4.0)                  # let the controller settle
        q = node.q_now()
        tr = node.buf.lookup_transform("base_link", "tool0", Time())
        t = tr.transform.translation
        p = ik.fk_tool0(q)
        d = math.dist(p, (t.x, t.y, t.z)) * 1e3
        worst = max(worst, d)
        print(f"pose{i+1}: q={[round(v,3) for v in q]}")
        print(f"  fk  = ({p[0]:+.5f}, {p[1]:+.5f}, {p[2]:+.5f})")
        print(f"  tf  = ({t.x:+.5f}, {t.y:+.5f}, {t.z:+.5f})   |d| = {d:.3f} mm")
    print(f"WORST fk-vs-tf disagreement: {worst:.3f} mm -> "
          + ("PASS <1mm" if worst < 1.0 else
             ("OK <5mm" if worst < 5.0 else "FAIL")))
    node.destroy_node()
    rclpy.shutdown()


if __name__ == "__main__":
    main()
