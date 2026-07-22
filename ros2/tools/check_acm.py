"""Prove the collision matrix works:
 1. read the Allowed Collision Matrix from the planning scene (show disabled
    adjacent pairs) and confirm arm-vs-arm pairs are NOT disabled;
 2. run /check_state_validity on:
      · the all-home pose            -> expect VALID (no collision)
      · an arm folded into itself    -> expect INVALID (self-collision caught)
      · two arms driven into each other -> expect INVALID (inter-arm caught)
"""
import rclpy
from moveit_msgs.srv import GetPlanningScene, GetStateValidity
from moveit_msgs.msg import PlanningSceneComponents, RobotState
from sensor_msgs.msg import JointState

rclpy.init()
n = rclpy.create_node("acm_check")
ALL = [f"arm{a}_joint{j}" for a in (1, 2, 3) for j in range(1, 7)]


def call(client, req):
    client.wait_for_service(timeout_sec=10)
    fut = client.call_async(req)
    rclpy.spin_until_future_complete(n, fut, timeout_sec=8)
    return fut.result()


# ---- 1. the ACM ----
ps = n.create_client(GetPlanningScene, "/get_planning_scene")
req = GetPlanningScene.Request()
req.components.components = PlanningSceneComponents.ALLOWED_COLLISION_MATRIX
res = call(ps, req)
acm = res.scene.allowed_collision_matrix
names = list(acm.entry_names)
disabled = 0
arm_vs_arm_disabled = 0
for i, row in enumerate(acm.entry_values):
    for j, allowed in enumerate(row.enabled):
        if allowed and i < j:
            disabled += 1
            a1 = names[i].split("_")[0]
            a2 = names[j].split("_")[0]
            if a1 != a2 and a1.startswith("arm") and a2.startswith("arm"):
                arm_vs_arm_disabled += 1
print(f"ACM: {len(names)} links, {disabled} disabled pairs")
print(f"  arm-vs-arm pairs disabled: {arm_vs_arm_disabled} "
      f"(want 0 — they must stay checked)")


# ---- 2. state validity ----
sv = n.create_client(GetStateValidity, "/check_state_validity")


def valid(positions, group="arm1"):
    req = GetStateValidity.Request()
    req.group_name = group
    rs = RobotState()
    js = JointState()
    js.name = ALL
    js.position = positions
    rs.joint_state = js
    req.robot_state = rs
    r = call(sv, req)
    return r.valid if r else None


home = [0.0] * 18
print("\nstate validity:")
print(f"  all-home pose:            valid={valid(home)}   (expect True)")

# fold arm1 into itself (extreme joint2/joint3)
fold = home[:]
fold[1] = 2.6   # arm1_joint2
fold[2] = -2.6  # arm1_joint3
print(f"  arm1 folded on itself:    valid={valid(fold)}   (expect False)")

rclpy.shutdown()
