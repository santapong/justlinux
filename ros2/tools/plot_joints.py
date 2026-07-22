import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
rows = [l.split() for l in open("/ros2_ws/joints.dat") if l.strip()]
t = [float(r[0]) for r in rows]
labels = ["arm1 shoulder", "arm2 shoulder", "arm3 shoulder",
          "arm1 waist", "arm1 elbow"]
colors = ["#D97757", "#e0a030", "#8EC07C", "#88c0d0", "#bd93f9"]
plt.style.use("dark_background")
fig, ax = plt.subplots(figsize=(11, 5))
for k in range(5):
    y = [float(r[k + 1]) for r in rows]
    ax.plot(t, y, color=colors[k], lw=2, label=labels[k])
ax.set_title("Live joint positions — 3 arms welding (one cycle)",
             color="#eeeeee", fontsize=14)
ax.set_xlabel("time (s)"); ax.set_ylabel("joint angle (rad)")
ax.legend(loc="upper right", framealpha=0.3)
ax.grid(alpha=0.15)
fig.tight_layout()
fig.savefig("/ros2_ws/joints_plot.png", dpi=110, facecolor="#1e1e22")
print("plot saved")
