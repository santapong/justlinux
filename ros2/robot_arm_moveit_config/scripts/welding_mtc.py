#!/usr/bin/env python3
"""MoveIt Task Constructor — plan a WELD as a multi-stage task.

MTC treats the weld as a task tree: each stage is planned on its own (with
alternatives) and MTC stitches the best collision-free combination into one
solution. Stages:

  current → move to ready → approach the seam (Cartesian down) →
  weld along the seam (Cartesian) → retract (Cartesian up) → home

Launch against move_group (robot_arm_moveit_config); the MTC RViz panel
"Motion Planning Tasks" shows the tree and each stage's solution count.
Uses the MTC python bindings, which need an rclcpp node (not rclpy).
"""
import time
from std_msgs.msg import Header
from geometry_msgs.msg import Vector3Stamped, Vector3, PoseStamped
from moveit.task_constructor import core, stages
import rclcpp

GROUP = "arm"

rclcpp.init()
node = rclcpp.Node("mtc_welding")     # name matches mtc_params.yaml

# planners
cartesian = core.CartesianPath()
cartesian.max_velocity_scaling_factor = 0.2
jointspace = core.JointInterpolationPlanner()

task = core.Task()
task.name = "welding"
task.loadRobotModel(node)

H = Header(frame_id="base_link")


def rel(name, x, y, z):
    m = stages.MoveRelative(name, cartesian)
    m.group = GROUP
    m.ik_frame = PoseStamped(header=Header(frame_id="tool0"))  # torch tip
    m.setDirection(Vector3Stamped(header=H, vector=Vector3(x=x, y=y, z=z)))
    return m


# 1. current state
task.add(stages.CurrentState("current"))

# 2. move to the "ready" pose (named SRDF state above the workpiece)
ready = stages.MoveTo("move to ready", jointspace)
ready.group = GROUP
ready.setGoal("ready")
task.add(ready)

# 3. approach: torch straight DOWN onto the seam
task.add(rel("approach seam ↓", 0.0, 0.0, -0.08))
# 4. THE WELD: trace the seam sideways at welding speed
task.add(rel("weld seam →", 0.0, 0.15, 0.0))
# 5. retract: lift the torch back off
task.add(rel("retract ↑", 0.0, 0.0, 0.08))

# 6. home
home = stages.MoveTo("move home", jointspace)
home.group = GROUP
home.setGoal("home")
task.add(home)

# ---- plan ----
print("planning the welding task (MTC)...")
if task.plan(10):
    print(f"SUCCESS — {len(task.solutions)} full-task solution(s)")
    for i, s in enumerate(task.solutions):
        print(f"  solution {i}: cost={s.cost:.3f}")
    task.publish(task.solutions[0])
    print("published best solution to the MTC RViz panel")
    # NOTE: task.execute() is unreliable through the container (its internal
    # executor can't connect to the execute_task_solution action). The arm is
    # animated by weld_replay.py streaming to /arm_controller instead. Keep the
    # node alive so the published solution stays visible in the panel.
    while True:
        time.sleep(5)
else:
    print("no full solution (see per-stage failures in the RViz panel)")
    time.sleep(30)
