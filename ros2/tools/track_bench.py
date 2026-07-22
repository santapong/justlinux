"""U3 acceptance benchmark for arm_ik.solve_track — run inside ros2-arm:jazzy.
Sanity-checks fk_tool0 vs the 4x4 fk, then sweeps 200 targets circling the
workspace and reports per-call timing + position error."""
import math
import random
import statistics
import sys

sys.path.insert(0, "/ros2_ws/tools")
import arm_ik as ik

# --- 1. fk_tool0 must agree with the original 4x4 FK (tip - TORCH*z) -------
random.seed(7)
worst = 0.0
for _ in range(200):
    q = [random.uniform(-2.5, 2.5) for _ in range(6)]
    tip, za = ik.fk(q)
    ref = [tip[i] - ik.TORCH * za[i] for i in range(3)]
    p = ik.fk_tool0(q)
    worst = max(worst, max(abs(p[i] - ref[i]) for i in range(3)))
print(f"fk_tool0 vs fk(4x4): worst |delta| = {worst:.3e} m")
assert worst < 1e-12, "fast FK disagrees with reference FK"

# --- 2. continuous sweep: 200 steps circling the workspace -----------------
# tilted circle, radius 0.10, center forward of the base — all reachable
CX, CY, CZ, R = 0.24, 0.0, 0.30, 0.10
targets = []
for i in range(200):
    a = 2 * math.pi * i / 200
    targets.append((CX + R * math.cos(a),
                    CY + R * math.sin(a) * 0.8,
                    CZ + 0.06 * math.sin(2 * a)))

# cold-start the very first pose with the slow weld solver's style seed
q = list(ik.NOMINAL)
q, info0 = ik.solve_track(targets[0], q)      # settle onto the circle
times, errs, jumps, reseeds = [], [], 0, 0
for t in targets:
    q, info = ik.solve_track(t, q)
    times.append(info["time_ms"])
    errs.append(info["err_m"])
    jumps += info["jump"]
    reseeds += info["reseeded"]

times_s = sorted(times)
errs_s = sorted(errs)
print(f"sweep: 200 steps  median {statistics.median(times):.2f} ms  "
      f"p90 {times_s[179]:.2f} ms  max {times_s[-1]:.2f} ms")
print(f"pos err: median {statistics.median(errs)*1e3:.3f} mm  "
      f"p90 {errs_s[179]*1e3:.3f} mm  max {errs_s[-1]*1e3:.3f} mm")
print(f"reseeds={reseeds}  unresolved_jumps={jumps}")

ok = statistics.median(times) < 15.0 and errs_s[-1] < 5e-3
print("ACCEPT: median<15ms and err<5mm ->", "PASS" if ok else "FAIL")

# --- 3. unreachable target: must clamp to boundary and still solve ---------
far = (0.9, 0.4, 0.8)
q2, info = ik.solve_track(far, q)
ct, was = ik.clamp_target(far)
d = math.dist(ik.fk_tool0(q2), ct)
print(f"unreachable {far}: clamped={info['clamped']} "
      f"boundary_err={d*1e3:.2f} mm  time={info['time_ms']:.2f} ms")

# --- 4. branch-flip guard: teleport target across the workspace ------------
qa, _ = ik.solve_track((0.30, 0.15, 0.25), q)
qb, info = ik.solve_track((-0.25, -0.15, 0.30), qa)   # far side
mj = max(abs(a - b) for a, b in zip(qb, qa))
print(f"teleport: reseeded={info['reseeded']} jump={info['jump']} "
      f"max_dq={mj:.2f} rad  err={info['err_m']*1e3:.2f} mm  "
      f"time={info['time_ms']:.2f} ms")
