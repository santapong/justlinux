#!/usr/bin/env python3
"""Welding visual effects — makes a moving arm look like it's welding.

Tracks the torch frame via TF. While /welding_active is True it:
  · deposits a glowing weld bead (orange spheres) along the torch path
  · throws bright sparks near the torch tip
Also publishes a static workpiece (two plates meeting at the weld seam).
Robot-agnostic: set torch_frame / base_frame params to any arm's tool frame.

  markers on /welding_markers  →  add a MarkerArray display in RViz
"""
import math
import rclpy
from rclpy.node import Node
from std_msgs.msg import Bool
from visualization_msgs.msg import Marker, MarkerArray
from geometry_msgs.msg import Point
from builtin_interfaces.msg import Duration
import tf2_ros

# a tiny deterministic PRNG (no Math.random-style nondeterminism needed,
# but we want varied sparks) — linear congruential
class Rng:
    def __init__(self, seed=12345):
        self.s = seed
    def f(self, lo, hi):
        self.s = (1103515245 * self.s + 12345) & 0x7fffffff
        return lo + (self.s / 0x7fffffff) * (hi - lo)


class Welder(Node):
    def __init__(self):
        super().__init__("welding_effects")
        self.declare_parameter("torch_frame", "tool0")
        self.declare_parameter("base_frame", "base_link")
        self.declare_parameter("torch_len", 0.14)   # flange → torch tip
        self.torch = self.get_parameter("torch_frame").value
        self.base = self.get_parameter("base_frame").value
        self.torch_len = self.get_parameter("torch_len").value
        self.pub = self.create_publisher(MarkerArray, "/welding_markers", 10)
        self.create_subscription(Bool, "/welding_active", self.on_active, 10)
        self.tf_buf = tf2_ros.Buffer()
        self.tf_listener = tf2_ros.TransformListener(self.tf_buf, self)
        self.active = False
        self.bead = []          # deposited bead points
        self.last = None
        self.rng = Rng()
        self.mid = 0
        self.create_timer(0.05, self.tick)
        self.create_timer(0.1, self.publish_torch)
        self.create_timer(1.0, self.publish_workpiece)

    def on_active(self, msg):
        self.active = msg.data

    def torch_pos(self):
        """World position of the TORCH TIP (flange pos, minus torch length
        straight down — the torch always points down while welding)."""
        try:
            t = self.tf_buf.lookup_transform(
                self.base, self.torch, rclpy.time.Time())
            return (t.transform.translation.x, t.transform.translation.y,
                    t.transform.translation.z - self.torch_len)
        except Exception:
            return None

    def tick(self):
        p = self.torch_pos()
        arr = MarkerArray()
        if self.active and p:
            # lay a bead sphere if the torch moved enough
            if self.last is None or math.dist(p, self.last) > 0.008:
                self.bead.append(p)
                self.last = p
            # sparks: short-lived bright points near the torch
            sparks = Marker()
            sparks.header.frame_id = self.base
            sparks.ns = "sparks"; sparks.id = 0
            sparks.type = Marker.POINTS; sparks.action = Marker.ADD
            sparks.scale.x = sparks.scale.y = 0.006
            sparks.color.r = 1.0; sparks.color.g = 0.85; sparks.color.b = 0.2
            sparks.color.a = 1.0
            sparks.lifetime = Duration(sec=0, nanosec=120_000_000)
            for _ in range(14):
                sparks.points.append(Point(
                    x=p[0] + self.rng.f(-0.04, 0.04),
                    y=p[1] + self.rng.f(-0.04, 0.04),
                    z=p[2] + self.rng.f(-0.02, 0.06)))
            arr.markers.append(sparks)
        # the growing weld bead (persistent)
        bead = Marker()
        bead.header.frame_id = self.base
        bead.ns = "bead"; bead.id = 1
        bead.type = Marker.SPHERE_LIST; bead.action = Marker.ADD
        bead.scale.x = bead.scale.y = bead.scale.z = 0.012
        bead.color.r = 1.0; bead.color.g = 0.45; bead.color.b = 0.1
        bead.color.a = 1.0
        bead.points = [Point(x=b[0], y=b[1], z=b[2]) for b in self.bead]
        arr.markers.append(bead)
        self.pub.publish(arr)

    def publish_torch(self):
        """The welding gun mounted on the flange — drawn in the tool0 frame
        so it rides with the arm. Torch points along local +Z (down while
        welding); tip glows when the arc is on."""
        arr = MarkerArray()
        L = self.torch_len

        def cyl(mid, zc, length, dia, rgb, a=1.0):
            m = Marker()
            m.header.frame_id = self.torch
            m.ns = "torch"; m.id = mid
            m.type = Marker.CYLINDER; m.action = Marker.ADD
            m.pose.position.z = zc; m.pose.orientation.w = 1.0
            m.scale.x = m.scale.y = dia; m.scale.z = length
            m.color.r, m.color.g, m.color.b = rgb; m.color.a = a
            return m

        arr.markers.append(cyl(0, L * 0.35, L * 0.7, 0.028,
                               (0.12, 0.12, 0.14)))     # dark neck
        arr.markers.append(cyl(1, L * 0.82, L * 0.30, 0.05,
                               (0.72, 0.45, 0.20)))      # copper gas nozzle
        # glowing arc tip
        tip = Marker()
        tip.header.frame_id = self.torch
        tip.ns = "torch"; tip.id = 2
        tip.type = Marker.SPHERE; tip.action = Marker.ADD
        tip.pose.position.z = L; tip.pose.orientation.w = 1.0
        s = 0.03 if self.active else 0.014
        tip.scale.x = tip.scale.y = tip.scale.z = s
        tip.color.r = 1.0
        tip.color.g = 0.9 if self.active else 0.5
        tip.color.b = 0.7 if self.active else 0.15
        tip.color.a = 1.0
        arr.markers.append(tip)
        self.pub.publish(arr)

    def publish_workpiece(self):
        """Two steel plates meeting at the weld seam (an L / fillet joint)."""
        arr = MarkerArray()
        for i, (px, py, pz, sx, sy, sz) in enumerate([
            (1.0, 0.0, 0.59, 0.34, 0.42, 0.02),      # base plate under the seam
            (1.0, 0.17, 0.67, 0.34, 0.02, 0.16),     # upright plate (fillet joint)
        ]):
            m = Marker()
            m.header.frame_id = self.base
            m.ns = "workpiece"; m.id = 10 + i
            m.type = Marker.CUBE; m.action = Marker.ADD
            m.pose.position.x = px; m.pose.position.y = py; m.pose.position.z = pz
            m.pose.orientation.w = 1.0
            m.scale.x = sx; m.scale.y = sy; m.scale.z = sz
            m.color.r = 0.55; m.color.g = 0.57; m.color.b = 0.6; m.color.a = 1.0
            arr.markers.append(m)
        self.pub.publish(arr)


def main():
    rclpy.init()
    try:
        rclpy.spin(Welder())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
