#!/usr/bin/env python3
"""Pick-and-place loop — a real robot-arm task.

Cycles forever through taught waypoints (home → reach to the pick spot →
grasp → lift → swing to the place spot → set down → return home) by sending
trajectories to the arm's joint_trajectory_controller — the same ros2_control
path a real arm uses. Watch it move in RViz.

  ros2-arm loop        # convenience wrapper launches demo + this node
"""
import rclpy
from rclpy.action import ActionClient
from rclpy.node import Node
from control_msgs.action import FollowJointTrajectory
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint
from builtin_interfaces.msg import Duration

JOINTS = ["joint1", "joint2", "joint3", "joint4", "joint5", "joint6"]

# taught poses: [waist, shoulder, elbow, wrist-roll, wrist-pitch, wrist-roll]
WAYPOINTS = [
    ("home",         [0.0,  0.0,  0.0, 0.0, 0.0, 0.0]),
    ("above pick",   [0.9, -0.5,  1.0, 0.0, 0.8, 0.0]),   # swing right, reach
    ("↓ pick",       [0.9, -0.15, 1.35, 0.0, 0.5, 0.0]),  # lower onto object
    ("grasp+lift",   [0.9, -0.5,  1.0, 0.0, 0.8, 0.0]),   # (close gripper) lift
    ("above place",  [-0.9, -0.5, 1.0, 0.0, 0.8, 0.0]),   # swing left
    ("↓ place",      [-0.9, -0.15, 1.35, 0.0, 0.5, 0.0]),  # set down
    ("release+lift", [-0.9, -0.5, 1.0, 0.0, 0.8, 0.0]),   # (open gripper) lift
    ("home",         [0.0,  0.0,  0.0, 0.0, 0.0, 0.0]),
]
SEG_TIME = 2.0   # seconds between waypoints


class PickPlace(Node):
    def __init__(self):
        super().__init__("arm_pick_place")
        self.client = ActionClient(
            self, FollowJointTrajectory,
            "/arm_controller/follow_joint_trajectory")
        self.get_logger().info("waiting for arm_controller...")
        self.client.wait_for_server()
        self.cycle = 0
        self.next_cycle()

    def next_cycle(self):
        self.cycle += 1
        self.get_logger().info(f"═══ pick & place — cycle {self.cycle} ═══")
        traj = JointTrajectory()
        traj.joint_names = JOINTS
        t = 0.0
        for _name, pos in WAYPOINTS:
            t += SEG_TIME
            pt = JointTrajectoryPoint()
            pt.positions = [float(p) for p in pos]
            pt.time_from_start = Duration(
                sec=int(t), nanosec=int((t % 1) * 1e9))
            traj.points.append(pt)
        goal = FollowJointTrajectory.Goal()
        goal.trajectory = traj
        self.client.send_goal_async(goal).add_done_callback(self._accepted)

    def _accepted(self, future):
        handle = future.result()
        if not handle.accepted:
            self.get_logger().warn("goal rejected — retrying in a moment")
            self.create_timer(1.0, self._retry_once)
            return
        handle.get_result_async().add_done_callback(lambda _f: self.next_cycle())

    def _retry_once(self):
        self.next_cycle()


def main():
    rclpy.init()
    try:
        rclpy.spin(PickPlace())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
