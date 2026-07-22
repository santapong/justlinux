#!/usr/bin/env python3
"""Animate the welding motion on the real controller so it's VISIBLE in RViz.

MTC plans the weld task (see welding_mtc.py) and shows it in the Motion
Planning Tasks panel, but its ExecuteTaskSolution plumbing is flaky through
the container. The motion itself is what we want to watch, so we stream the
weld cycle straight to the joint_trajectory_controller, which is active and
feeds /joint_states -> RobotModel. The arm visibly:

    home -> ready -> dip onto the seam -> sweep the torch L<->R along the
    seam (the actual weld) -> retract -> home,  looping.
"""
import rclpy
from rclpy.node import Node
from rclpy.action import ActionClient
from control_msgs.action import FollowJointTrajectory
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint

JOINTS = [f"joint{i}" for i in range(1, 7)]

# (label, joint positions, seconds-from-start-of-this-cycle)
CYCLE = [
    ("home",        [0.0,  0.0,  0.0, 0.0, 0.0,  0.0],  2.0),
    ("ready",       [0.0, -0.6,  1.2, 0.0, 0.9,  0.0],  5.0),
    ("dip to seam", [0.0, -0.35, 1.4, 0.0, 0.85, 0.0],  8.0),
    ("weld -> L",   [-0.4, -0.35, 1.4, 0.0, 0.85, 0.0], 11.0),
    ("weld -> R",   [0.4, -0.35, 1.4, 0.0, 0.85, 0.0],  15.0),
    ("weld -> mid", [0.0, -0.35, 1.4, 0.0, 0.85, 0.0],  18.0),
    ("retract",     [0.0, -0.6,  1.2, 0.0, 0.9,  0.0],  21.0),
    ("home",        [0.0,  0.0,  0.0, 0.0, 0.0,  0.0],  24.0),
]


def main():
    rclpy.init()
    node = Node("weld_replay")
    ac = ActionClient(node, FollowJointTrajectory,
                      "/arm_controller/follow_joint_trajectory")
    node.get_logger().info("waiting for /arm_controller ...")
    ac.wait_for_server()
    node.get_logger().info("connected — welding on loop (Ctrl-C to stop)")

    cyc = 0
    while rclpy.ok():
        cyc += 1
        traj = JointTrajectory()
        traj.joint_names = JOINTS
        for label, pos, t in CYCLE:
            p = JointTrajectoryPoint()
            p.positions = pos
            p.time_from_start.sec = int(t)
            p.time_from_start.nanosec = int((t - int(t)) * 1e9)
            traj.points.append(p)
        goal = FollowJointTrajectory.Goal()
        goal.trajectory = traj

        node.get_logger().info(f"weld pass {cyc}: streaming {len(CYCLE)} waypoints")
        send = ac.send_goal_async(goal)
        rclpy.spin_until_future_complete(node, send)
        gh = send.result()
        if not gh.accepted:
            node.get_logger().error("goal rejected")
            break
        res = gh.get_result_async()
        rclpy.spin_until_future_complete(node, res)
        node.get_logger().info(f"weld pass {cyc} done")

    rclpy.shutdown()


if __name__ == "__main__":
    main()
