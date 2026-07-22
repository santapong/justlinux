"""Numeric FK + IK for the weld_arm — pure math, no ROS/numpy needed.
Finds joint angles so the torch tip reaches a target with the torch pointing
straight down. The Y-axis joints (2,3,5) work in the arm's forward/up plane;
joint1 sets azimuth. Prints poses for the coordinator."""
import math

# joint origin translations along local +Z, and axis ('z' or 'y')
CHAIN = [(0.075, "z"), (0.08, "y"), (0.18, "y"),
         (0.15, "z"), (0.06, "y"), (0.05, "z")]
TORCH = 0.16   # tool0 -> torch tip along local +Z


def mat_mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)]
            for i in range(4)]


def trans(z):
    return [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, z], [0, 0, 0, 1]]


def rot(axis, t):
    c, s = math.cos(t), math.sin(t)
    if axis == "z":
        return [[c, -s, 0, 0], [s, c, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]
    return [[c, 0, s, 0], [0, 1, 0, 0], [-s, 0, c, 0], [0, 0, 0, 1]]   # y


def fk(joints):
    m = [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]
    for (z, ax), q in zip(CHAIN, joints):
        m = mat_mul(m, trans(z))
        m = mat_mul(m, rot(ax, q))
    tip = [m[i][0] * 0 + m[i][1] * 0 + m[i][2] * TORCH + m[i][3]
           for i in range(3)]
    z_axis = [m[i][2] for i in range(3)]     # tool0 local +Z in world
    return tip, z_axis


def cost(joints, target):
    tip, za = fk(joints)
    dp = sum((tip[i] - target[i]) ** 2 for i in range(3))
    # torch pointing down: local +Z should equal world -Z
    do = (za[0] ** 2 + za[1] ** 2 + (za[2] + 1) ** 2)
    return dp + 0.5 * do


def _descend(q, target):
    step = 0.4
    for _ in range(500):
        improved = False
        for j in range(6):
            for d in (step, -step):
                cand = q[:]
                cand[j] += d
                if cost(cand, target) < cost(q, target) - 1e-12:
                    q = cand
                    improved = True
        if not improved:
            step *= 0.5
            if step < 1e-5:
                break
    return q, cost(q, target)


def solve(target, start=None):
    """coordinate descent with several restarts — avoids local minima."""
    seeds = [start] if start else []
    seeds += [[0.0, 0.6, 1.0, 0.0, 1.5, 0.0],
              [0.0, 0.9, 0.7, 0.0, 1.5, 0.0],
              [0.0, 1.1, 1.1, 0.0, 0.9, 0.0],
              [0.0, 0.4, 1.4, 0.0, 1.3, 0.0]]
    best, bc = None, 1e9
    for s in seeds:
        q, c = _descend(s[:], target)
        if c < bc:
            best, bc = q, c
    return best, bc


if __name__ == "__main__":
    # weld target in the arm's local frame: forward(x), up(z)
    FWD, UP = 0.28, 0.28
    center, c = solve([FWD, 0.0, UP])
    tip, za = fk(center)
    print(f"WELD_C target=({FWD},0,{UP})  cost={c:.5f}")
    print(f"  reached tip = ({tip[0]:.3f},{tip[1]:.3f},{tip[2]:.3f})  "
          f"torch_z=({za[0]:.2f},{za[1]:.2f},{za[2]:.2f})")
    print(f"  joints = {[round(v,3) for v in center]}")
    # sweep the seam via joint1 (waist) — keeps torch down
    for tag, dq1 in [("WELD_L", -0.45), ("WELD_R", 0.45)]:
        q = center[:]
        q[0] = dq1
        tip, _ = fk(q)
        print(f"{tag}: joint1={dq1}  tip=({tip[0]:.3f},{tip[1]:.3f},{tip[2]:.3f})")
    # approach = same but higher
    ap, _ = solve([FWD, 0.0, UP + 0.14], start=center)
    print(f"APPROACH joints = {[round(v,3) for v in ap]}")
