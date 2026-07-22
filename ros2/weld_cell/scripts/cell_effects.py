#!/usr/bin/env python3
"""Multi-arm welding effects: for each arm, track its torch tip via TF and,
while /armN/welding is True, lay a glowing bead + throw sparks. One
MarkerArray on /cell_markers covers all three arms."""
import math
import rclpy
from rclpy.node import Node
from std_msgs.msg import Bool
from visualization_msgs.msg import Marker, MarkerArray
from geometry_msgs.msg import Point
from builtin_interfaces.msg import Duration
import tf2_ros

ARMS = ["arm1_", "arm2_", "arm3_"]
TORCH_LEN = 0.16


class Rng:
    def __init__(self, seed=7):
        self.s = seed
    def f(self, lo, hi):
        self.s = (1103515245 * self.s + 12345) & 0x7fffffff
        return lo + (self.s / 0x7fffffff) * (hi - lo)


class CellFX(Node):
    def __init__(self):
        super().__init__("cell_effects")
        self.pub = self.create_publisher(MarkerArray, "/cell_markers", 10)
        self.buf = tf2_ros.Buffer()
        self.listener = tf2_ros.TransformListener(self.buf, self)
        self.active = {a: False for a in ARMS}
        self.bead = {a: [] for a in ARMS}
        self.last = {a: None for a in ARMS}
        self.rng = Rng()
        for a in ARMS:
            self.create_subscription(Bool, f"/{a}welding",
                                     lambda m, arm=a: self.on_flag(arm, m), 10)
        self.create_timer(0.05, self.tick)

    def on_flag(self, arm, msg):
        self.active[arm] = msg.data

    def tip(self, arm):
        try:
            t = self.buf.lookup_transform("world", f"{arm}tool0",
                                          rclpy.time.Time())
            return (t.transform.translation.x, t.transform.translation.y,
                    t.transform.translation.z - TORCH_LEN)
        except Exception:
            return None

    def tick(self):
        arr = MarkerArray()
        for idx, a in enumerate(ARMS):
            p = self.tip(a)
            if self.active[a] and p:
                if self.last[a] is None or math.dist(p, self.last[a]) > 0.006:
                    self.bead[a].append(p)
                    self.last[a] = p
                sp = Marker()
                sp.header.frame_id = "world"
                sp.ns = f"sparks{idx}"; sp.id = idx
                sp.type = Marker.POINTS; sp.action = Marker.ADD
                sp.scale.x = sp.scale.y = 0.006
                sp.color.r = 1.0; sp.color.g = 0.85; sp.color.b = 0.2; sp.color.a = 1.0
                sp.lifetime = Duration(sec=0, nanosec=120_000_000)
                for _ in range(12):
                    sp.points.append(Point(x=p[0] + self.rng.f(-0.04, 0.04),
                                           y=p[1] + self.rng.f(-0.04, 0.04),
                                           z=p[2] + self.rng.f(-0.02, 0.06)))
                arr.markers.append(sp)
            bd = Marker()
            bd.header.frame_id = "world"
            bd.ns = f"bead{idx}"; bd.id = 100 + idx
            bd.type = Marker.SPHERE_LIST; bd.action = Marker.ADD
            bd.scale.x = bd.scale.y = bd.scale.z = 0.012
            bd.color.r = 1.0; bd.color.g = 0.45; bd.color.b = 0.1; bd.color.a = 1.0
            bd.points = [Point(x=b[0], y=b[1], z=b[2]) for b in self.bead[a]]
            arr.markers.append(bd)
        self.pub.publish(arr)


def main():
    rclpy.init()
    try:
        rclpy.spin(CellFX())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
