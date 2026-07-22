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


# a natural elbow-bent posture — spreading the bend across shoulder/elbow/
# wrist keeps the torch from folding back into the forearm (a self-collision)
NOMINAL = [0.0, 1.0, 1.0, 0.0, 1.14, 0.0]


def cost(joints, target):
    tip, za = fk(joints)
    dp = sum((tip[i] - target[i]) ** 2 for i in range(3))
    # torch pointing down: local +Z should equal world -Z
    do = (za[0] ** 2 + za[1] ** 2 + (za[2] + 1) ** 2)
    # regularize toward the natural posture (esp. keep wrist j5 moderate)
    reg = sum((joints[i] - NOMINAL[i]) ** 2 for i in (1, 2, 4)) * 0.02
    reg += max(0.0, abs(joints[4]) - 1.5) ** 2 * 0.2   # punish a cranked wrist
    return dp + 0.5 * do + reg


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


# --------------------------------------------------------------------------
# Tracking mode — fast warm-start IK for streaming targets (hand following).
# Solves for the *tool0* position only (no torch offset, no orientation
# term), single coordinate descent from the previous solution.  Meant to be
# called every camera frame with q_prev = last returned joints.
# --------------------------------------------------------------------------
from math import cos as _cos, sin as _sin, sqrt as _sqrt
from time import perf_counter as _clk

# joint2 origin (the "shoulder") sits on the base z-axis: 0.075 + 0.08.
# joint1 (azimuth) can't move it, so the reachable set is a spherical shell
# around it with outer radius = sum of the remaining link lengths.
_SHOULDER_Z = 0.075 + 0.08
_R_MAX = (0.18 + 0.15 + 0.06 + 0.05) * 0.995   # stay just inside the boundary
_R_MIN = 0.03

_TRACK_TOL2 = (1e-4) ** 2      # stop when within 0.1 mm
_JUMP_LIMIT = 0.5              # rad — max per-joint jump accepted from q_prev
_TRACK_JLIM = 2.85             # rad — URDF limit is +-2.9; match the caller's
                               # (hand_follow) clamp so descent never proposes
                               # a solution the streamer would clip anyway
# Position-only 5-DOF IK has a 2-dim null space; without a posture term the
# redundant wrist DOFs wander until joint4/joint5 ride the +-2.85 rails and
# every small target move becomes a branch flip. A tiny pull toward NOMINAL
# on the posture joints resolves the null space; weight is small enough that
# the position term still dominates to well under a millimeter.
_TRACK_REG_W = 2e-4
_TRACK_REG_J = (1, 2, 3, 4)    # shoulder/elbow/wrist — j0 (azimuth) is fully
                               # determined by the target, leave it free


def _track_reg(q):
    return sum((q[i] - NOMINAL[i]) ** 2 for i in _TRACK_REG_J)


def fk_tool0(q):
    """Position of tool0 (NOT the torch tip) — fast scalar FK."""
    r00, r01, r02 = 1.0, 0.0, 0.0
    r10, r11, r12 = 0.0, 1.0, 0.0
    r20, r21, r22 = 0.0, 0.0, 1.0
    px = py = pz = 0.0
    for (z, ax), t in zip(CHAIN, q):
        px += r02 * z; py += r12 * z; pz += r22 * z
        c, s = _cos(t), _sin(t)
        if ax == "z":                      # R = R @ Rz(t)
            a, b = r00, r01; r00 = c * a + s * b; r01 = c * b - s * a
            a, b = r10, r11; r10 = c * a + s * b; r11 = c * b - s * a
            a, b = r20, r21; r20 = c * a + s * b; r21 = c * b - s * a
        else:                              # R = R @ Ry(t)
            a, b = r00, r02; r00 = c * a - s * b; r02 = s * a + c * b
            a, b = r10, r12; r10 = c * a - s * b; r12 = s * a + c * b
            a, b = r20, r22; r20 = c * a - s * b; r22 = s * a + c * b
    return px, py, pz


def clamp_target(t):
    """Project an unreachable target onto the workspace boundary.
    Returns (clamped_xyz, was_clamped)."""
    dx, dy, dz = t[0], t[1], t[2] - _SHOULDER_Z
    d = _sqrt(dx * dx + dy * dy + dz * dz)
    if d > _R_MAX:
        k = _R_MAX / d
    elif d < _R_MIN:
        if d < 1e-9:                       # degenerate: pick +x
            return (_R_MIN, 0.0, _SHOULDER_Z), True
        k = _R_MIN / d
    else:
        return (t[0], t[1], t[2]), False
    return (dx * k, dy * k, _SHOULDER_Z + dz * k), True


def _err2(q, target):
    px, py, pz = fk_tool0(q)
    dx, dy, dz = px - target[0], py - target[1], pz - target[2]
    return dx * dx + dy * dy + dz * dz


def _cd(q, target, step, max_sweeps, reg_w):
    """Cached-cost coordinate descent on position error (+ optional posture
    regularizer), in place on q, joints clamped to +-_TRACK_JLIM.
    Returns (err2, sweeps) — err2 is the pure position error."""
    e2 = _err2(q, target)
    cur = e2 + reg_w * _track_reg(q) if reg_w else e2
    sweeps = 0
    while sweeps < max_sweeps and e2 > _TRACK_TOL2:
        sweeps += 1
        improved = False
        for j in range(5):                 # joint6 (last, about tool z) can't
            qj = q[j]                      # move the tool0 point — skip it
            for d in (step, -step):
                cand = max(-_TRACK_JLIM, min(_TRACK_JLIM, qj + d))
                if cand == qj:
                    continue               # pinned at a limit — no move
                q[j] = cand
                ce2 = _err2(q, target)
                c = ce2 + reg_w * _track_reg(q) if reg_w else ce2
                if c < cur - 1e-16:
                    cur, e2 = c, ce2
                    improved = True
                    break
                q[j] = qj
        if not improved:
            step *= 0.5
            if step < 5e-5:
                break
    return e2, sweeps


def _track_descend(q, target, step=0.05, max_sweeps=40):
    """Regularized descent (resolves the null space, keeps posture near
    NOMINAL) followed by a short position-only polish that recovers the
    sub-mm the posture term traded away — starting that close to the
    regularized posture, the polish cannot re-wind the null space.
    Returns (q, err2, sweeps)."""
    q = [max(-_TRACK_JLIM, min(_TRACK_JLIM, v)) for v in q]
    e2, sweeps = _cd(q, target, step, max_sweeps, _TRACK_REG_W)
    if e2 > _TRACK_TOL2:
        e2, _ = _cd(q, target, 0.01, 12, 0.0)
    return q, e2, sweeps


# fallback seeds for when the warm start diverges (elbow-branch flip etc.)
_TRACK_SEEDS = [NOMINAL,
                [0.0, 0.6, 1.0, 0.0, 1.5, 0.0],
                [0.0, 1.1, 1.1, 0.0, 0.9, 0.0],
                [0.0, 0.4, 1.4, 0.0, 1.3, 0.0]]


def solve_track(target_xyz, q_prev):
    """Tracking-mode IK: warm-start position-only solve for tool0.

    Returns (q, info) where info = dict(err_m, time_ms, clamped, reseeded,
    jump, sweeps).  `jump` True means even the fallback couldn't stay within
    _JUMP_LIMIT rad of q_prev — caller may want to interpolate/slew."""
    t0 = _clk()
    target, clamped = clamp_target(target_xyz)
    q, e2, sweeps = _track_descend(q_prev, target)
    reseeded = False
    jump = any(abs(a - b) > _JUMP_LIMIT for a, b in zip(q, q_prev))
    if jump:                               # branch flip — short multi-seed redo
        reseeded = True
        az = math.atan2(target[1], target[0])
        cands = [(q, e2)]
        for s in _TRACK_SEEDS:
            s = list(s); s[0] = az         # pre-aim the waist at the target
            cands.append(_track_descend(s, target, step=0.3, max_sweeps=60)[:2])
        ok = [c for c in cands
              if c[1] < (0.002) ** 2 and not any(
                  abs(a - b) > _JUMP_LIMIT for a, b in zip(c[0], q_prev))]
        if ok:
            q, e2 = min(ok, key=lambda c: c[1])
            jump = False
        else:                              # accept best error, flag the jump
            q, e2 = min(cands, key=lambda c: c[1])
            jump = any(abs(a - b) > _JUMP_LIMIT for a, b in zip(q, q_prev))
    return q, {"err_m": _sqrt(e2), "time_ms": (_clk() - t0) * 1e3,
               "clamped": clamped, "reseeded": reseeded, "jump": jump,
               "sweeps": sweeps}


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
