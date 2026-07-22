"""Collision-check the WHOLE welding trajectory of all 3 arms.

The coordinator drives the 3 arms through the same waypoints simultaneously,
so at every step all 18 joints are set (arm1|arm2|arm3 at the same relative
pose). We interpolate between the waypoints and, at each interpolated state,
ask move_group /check_state_validity for each arm group — which checks that
arm against its own links (minus the ACM), the OTHER arms, and the car part.
Reports any collision along the path."""
import rclpy
from moveit_msgs.srv import GetStateValidity
from moveit_msgs.msg import RobotState
from sensor_msgs.msg import JointState

rclpy.init()
n = rclpy.create_node("traj_check")
sv = n.create_client(GetStateValidity, "/check_state_validity")
sv.wait_for_service(timeout_sec=10)

ALL = [f"arm{a}_joint{j}" for a in (1, 2, 3) for j in range(1, 7)]

# the coordinator's cycle (per-arm 6-joint waypoints), in order
WAYPOINTS = [
    ("HOME",       [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("APPROACH_L", [-0.5, 0.53, 1.138, 0.0, 1.474, 0.0]),
    ("WELD_L",     [-0.5, 0.795, 1.391, 0.0, 0.956, 0.0]),
    ("WELD_R",     [0.5, 0.795, 1.391, 0.0, 0.956, 0.0]),
    ("APPROACH_R", [0.5, 0.53, 1.138, 0.0, 1.474, 0.0]),
    ("HOME",       [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
]
STEPS = 10   # interpolation steps per segment


def full_state(perarm):
    return list(perarm) * 3   # same pose on all 3 arms


def check(positions):
    """True if EVERY arm group is collision-free at this full state."""
    for a in (1, 2, 3):
        req = GetStateValidity.Request()
        req.group_name = f"arm{a}"
        rs = RobotState()
        js = JointState()
        js.name = ALL
        js.position = positions
        rs.joint_state = js
        req.robot_state = rs
        fut = sv.call_async(req)
        rclpy.spin_until_future_complete(n, fut, timeout_sec=8)
        r = fut.result()
        if r is None or not r.valid:
            return False, a
    return True, None


def lerp(a, b, t):
    return [a[i] + (b[i] - a[i]) * t for i in range(len(a))]


print(f"checking {len(WAYPOINTS) - 1} segments x {STEPS} steps "
      f"(all 3 arms, self + inter-arm + car-part)...\n")
collisions = 0
checked = 0
for (n1, w1), (n2, w2) in zip(WAYPOINTS, WAYPOINTS[1:]):
    seg_ok = True
    for s in range(STEPS + 1):
        t = s / STEPS
        state = full_state(lerp(w1, w2, t))
        ok, bad = check(state)
        checked += 1
        if not ok:
            seg_ok = False
            collisions += 1
            if collisions <= 5:
                print(f"  ✗ COLLISION on {n1}→{n2} at t={t:.1f} (arm{bad})")
    if seg_ok:
        print(f"  ✓ {n1} → {n2}: clear")

print(f"\n{checked} states checked, {collisions} in collision")
print("RESULT:", "ALL TRAJECTORIES COLLISION-FREE ✓" if collisions == 0
      else f"⚠ {collisions} colliding states found")
rclpy.shutdown()
