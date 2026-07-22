#!/usr/bin/env python3
"""Weld coordinator — drives all 3 arms of the cell SIMULTANEOUSLY through a
welding cycle (approach → weld pass sweeping the seam → retract → home), then
loops. Each arm welds its own seam on the car part. Publishes /armN/welding
so the effects node knows when each arc is on.

Taught joint poses (the arms are placed symmetrically, so they share a pose);
tune WELD/APPROACH below if a torch doesn't sit on its seam.
"""
import rclpy
from rclpy.action import ActionClient
from rclpy.node import Node
from control_msgs.action import FollowJointTrajectory
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint
from std_msgs.msg import Bool
from builtin_interfaces.msg import Duration

ARMS = ["arm1_", "arm2_", "arm3_"]
# [waist, shoulder, elbow, wrist-roll, wrist-pitch, wrist-roll]
# IK-derived (tools/arm_ik.py): torch reaches ~0.25 forward/up pointing DOWN;
# joint1 (waist) sweeps the seam. The arms are placed 0.25 in front of each
# seam in the cell URDF so these land the torch on the workpiece.
# arms on 0.30 m pedestals reach DOWN-forward to the seam — natural posture
# (elbow bent, moderate wrist) so the torch never folds into the forearm.
# IK-solved + collision-verified (tools/arm_ik.py, check_trajectory.py).
HOME = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
APPROACH_L = [-0.5, 0.53, 1.138, 0.0, 1.474, 0.0]  # above seam start
WELD_L = [-0.5, 0.795, 1.391, 0.0, 0.956, 0.0]     # seam start (on part)
WELD_R = [0.5, 0.795, 1.391, 0.0, 0.956, 0.0]      # seam end (swept +waist)
APPROACH_R = [0.5, 0.53, 1.138, 0.0, 1.474, 0.0]   # retract up


class Coordinator(Node):
    def __init__(self):
        super().__init__("weld_coordinator")
        self.acs = {}
        self.flags = {}
        for a in ARMS:
            self.acs[a] = ActionClient(
                self, FollowJointTrajectory,
                f"/{a}controller/follow_joint_trajectory")
            self.flags[a] = self.create_publisher(Bool, f"/{a}welding", 10)
        for a in ARMS:
            self.get_logger().info(f"waiting for {a}controller...")
            self.acs[a].wait_for_server()
        self.loop()

    def joints(self, a):
        return [f"{a}joint{j}" for j in range(1, 7)]

    def send_all(self, waypoints):
        """waypoints: list of (positions, t). Sends the SAME motion to every
        arm at once and waits for all to finish."""
        futures = []
        for a in ARMS:
            traj = JointTrajectory()
            traj.joint_names = self.joints(a)
            for pos, t in waypoints:
                pt = JointTrajectoryPoint()
                pt.positions = [float(v) for v in pos]
                pt.time_from_start = Duration(sec=int(t),
                                              nanosec=int((t % 1) * 1e9))
                traj.points.append(pt)
            goal = FollowJointTrajectory.Goal()
            goal.trajectory = traj
            futures.append(self.acs[a].send_goal_async(goal))
        handles = []
        for f in futures:
            rclpy.spin_until_future_complete(self, f)
            gh = f.result()
            if gh and gh.accepted:
                handles.append(gh.get_result_async())
        for h in handles:
            rclpy.spin_until_future_complete(self, h)

    def set_arcs(self, on):
        for _ in range(4):
            for a in ARMS:
                self.flags[a].publish(Bool(data=on))
            rclpy.spin_once(self, timeout_sec=0.05)

    def loop(self):
        cyc = 0
        while rclpy.ok():
            cyc += 1
            self.get_logger().info(f"═══ cell weld cycle {cyc} (3 arms) ═══")
            self.set_arcs(False)
            self.send_all([(APPROACH_L, 3.0), (WELD_L, 5.0)])  # to seam start
            self.set_arcs(True)                                 # all arcs on
            self.send_all([(WELD_R, 6.0)])                      # weld the seam
            self.set_arcs(False)
            self.send_all([(APPROACH_R, 2.0), (HOME, 4.0)])     # retract + home


def main():
    rclpy.init()
    try:
        Coordinator()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
