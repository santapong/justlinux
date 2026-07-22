import rclpy, time
from sensor_msgs.msg import JointState
rclpy.init(); n = rclpy.create_node("rec")
rows = []
JW = ["arm1_joint2", "arm2_joint2", "arm3_joint2", "arm1_joint1", "arm1_joint3"]
def cb(m):
    idx = {name: i for i, name in enumerate(m.name)}
    if all(j in idx for j in JW):
        rows.append([time.time()] + [m.position[idx[j]] for j in JW])
n.create_subscription(JointState, "/joint_states", cb, 50)
end = time.time() + 16
while time.time() < end and rclpy.ok():
    rclpy.spin_once(n, timeout_sec=0.05)
t0 = rows[0][0] if rows else 0
with open("/ros2_ws/joints.dat", "w") as f:
    for r in rows:
        f.write(" ".join(f"{r[0]-t0:.2f}" if i == 0 else f"{v:.4f}"
                          for i, v in enumerate(r)) + "\n")
print(f"recorded {len(rows)} samples of 5 joints over 16s")
