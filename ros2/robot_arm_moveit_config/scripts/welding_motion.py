#!/usr/bin/env python3
"""Welding motion — trace a weld seam with a 6-DOF arm (Fanuc M-10iA).

Uses MoveIt IK (/compute_ik) to keep the torch pointing DOWN while it:
  approach → descend to the seam → WELD along the seam (slow, welding speed,
  /welding_active=True) → retract → home, then repeats.
Pair with welding_effects.py for the bead + sparks.
"""
import rclpy
from rclpy.action import ActionClient
from rclpy.node import Node
from moveit_msgs.srv import GetPositionIK
from control_msgs.action import FollowJointTrajectory
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint
from std_msgs.msg import Bool
from builtin_interfaces.msg import Duration

JOINTS = [f"joint_{i}" for i in range(1, 7)]
DOWN = (1.0, 0.0, 0.0, 0.0)     # torch points -Z (down at the workpiece)
X = 1.0                          # seam plane (reachable, IK-verified)
# tool0 (flange) sits a torch-length ABOVE the seam so the torch reaches down
Z_WELD = 0.74                    # flange height while welding
Z_UP = 0.88                      # approach/retract height
Y0, Y1 = -0.15, 0.15             # seam runs 30 cm along Y


class WeldMotion(Node):
    def __init__(self):
        super().__init__("welding_motion")
        self.ik = self.create_client(GetPositionIK, "/compute_ik")
        self.ac = ActionClient(
            self, FollowJointTrajectory,
            "/fanuc_controller/follow_joint_trajectory")
        self.active = self.create_publisher(Bool, "/welding_active", 10)
        self.get_logger().info("waiting for IK service + controller...")
        self.ik.wait_for_service()
        self.ac.wait_for_server()
        self.build_path()
        self.loop()

    def solve(self, x, y, z, seed):
        req = GetPositionIK.Request()
        r = req.ik_request
        r.group_name = "manipulator"
        r.ik_link_name = "tool0"
        r.robot_state.joint_state.name = JOINTS
        r.robot_state.joint_state.position = seed
        r.pose_stamped.header.frame_id = "base_link"
        p = r.pose_stamped.pose
        p.position.x, p.position.y, p.position.z = x, y, z
        (p.orientation.x, p.orientation.y,
         p.orientation.z, p.orientation.w) = DOWN
        r.timeout.sec = 2
        fut = self.ik.call_async(req)
        rclpy.spin_until_future_complete(self, fut, timeout_sec=5)
        res = fut.result()
        if res and res.error_code.val == 1:
            return list(res.solution.joint_state.position[:6])
        self.get_logger().warn(f"IK failed at ({x},{y},{z})")
        return seed

    def build_path(self):
        seed = [0.0] * 6
        self.approach = []
        s = self.solve(X, Y0, Z_UP, seed); seed = s
        self.approach.append((s, 2.5))
        s = self.solve(X, Y0, Z_WELD, seed); seed = s
        self.approach.append((s, 4.5))            # descend to the seam
        self.seam, t = [], 0.0
        for i in range(13):                        # slow pass along the seam
            y = Y0 + (Y1 - Y0) * i / 12
            s = self.solve(X, y, Z_WELD, seed); seed = s
            t += 0.7
            self.seam.append((s, t))
        self.retract = []
        s = self.solve(X, Y1, Z_UP, seed); seed = s
        self.retract.append((s, 2.5))
        self.retract.append(([0.0] * 6, 5.0))     # home
        self.get_logger().info("weld path computed (torch-down, seam traced)")

    def send(self, pts):
        traj = JointTrajectory()
        traj.joint_names = JOINTS
        for pos, t in pts:
            pt = JointTrajectoryPoint()
            pt.positions = [float(v) for v in pos]
            pt.time_from_start = Duration(sec=int(t), nanosec=int((t % 1) * 1e9))
            traj.points.append(pt)
        goal = FollowJointTrajectory.Goal()
        goal.trajectory = traj
        fut = self.ac.send_goal_async(goal)
        rclpy.spin_until_future_complete(self, fut)
        gh = fut.result()
        if not gh or not gh.accepted:
            return
        rf = gh.get_result_async()
        rclpy.spin_until_future_complete(self, rf)

    def set_active(self, v):
        for _ in range(4):
            self.active.publish(Bool(data=v))
            rclpy.spin_once(self, timeout_sec=0.05)

    def loop(self):
        cyc = 0
        while rclpy.ok():
            cyc += 1
            self.get_logger().info(f"═══ weld pass {cyc} — striking arc ═══")
            self.set_active(False)
            self.send(self.approach)
            self.set_active(True)                  # arc on
            self.send(self.seam)
            self.set_active(False)                 # arc off
            self.send(self.retract)


def main():
    rclpy.init()
    try:
        WeldMotion()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
