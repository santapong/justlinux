"""One-command hand-following bring-up: the full MoveIt2 demo (RViz +
mock ros2_control) plus hand_follow.py streaming JointTrajectory commands
to /arm_controller/joint_trajectory (U5 integration).

  ros2-arm handfollow                       camera + mediapipe, live tracking
  ros2-arm handfollow synthetic             camera/mediapipe replaced by a
                                             deterministic 3D sine sweep
  ros2-arm handfollow latency_probe:=true   either mode + per-stage timing
                                             log every 3 s (see hand_follow.py
                                             header + docs/handfollow-run.md)

hand_follow.py is run straight from its mounted src path (not the colcon
install tree) under /opt/mpvenv/bin/python, the mediapipe venv baked into
ros2-arm:jazzy — so edits to it take effect on the next launch with no
rebuild. Only this launch file's own resolution of demo.launch.py goes
through the installed package share dir (same as loop.launch.py).
"""
import os
from launch import LaunchDescription
from launch.actions import (DeclareLaunchArgument, IncludeLaunchDescription,
                             TimerAction, ExecuteProcess)
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration
from ament_index_python.packages import get_package_share_directory

HAND_FOLLOW_PY = "/ros2_ws/src/robot_arm_moveit_config/scripts/hand_follow.py"
MPVENV_PYTHON = "/opt/mpvenv/bin/python"


def generate_launch_description():
    share = get_package_share_directory("robot_arm_moveit_config")
    demo = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(
            os.path.join(share, "launch", "demo.launch.py")))

    # every hand_follow.py parameter is forwarded as a launch argument so the
    # documented `ros2-arm handfollow key:=value` tuning overrides work
    # (docs/handfollow-run.md "Tuning knobs"). Defaults mirror the node's.
    args = [
        ("synthetic", "false",
         "true: 3D sine-sweep test target instead of camera/mediapipe"),
        ("latency_probe", "false",
         "true: log per-stage (capture/infer/IK/publish) timing every 3s"),
        ("preview", "false",
         "true: live webcam window with hand detections drawn (X11)"),
        ("camera", "/dev/video0", "video capture device"),
        ("model_path", "/opt/models/hand_landmarker.task",
         "mediapipe hand_landmarker .task model file"),
        ("rate_hz", "20.0", "control tick / command rate"),
        ("time_from_start", "0.12", "seconds-ahead stamp on each traj point"),
        ("min_cutoff", "1.0", "One-Euro filter min cutoff (lower = smoother)"),
        ("beta", "0.5", "One-Euro filter beta (higher = less lag, more jitter)"),
        ("max_step_m", "0.03", "per-tick target slew clamp while tracking"),
        ("decay_step_m", "0.005", "per-tick slew while decaying to HOME"),
        ("loss_hold_sec", "2.0", "hold time after hand loss before homing"),
        ("max_joint_step_rad", "0.10", "per-tick joint-space delta clamp"),
        ("hand_conf", "0.6", "min handedness confidence for a Left hand"),
        ("s_near", "0.30", "hand-size scale mapped to near end of X (depth)"),
        ("s_far", "0.10", "hand-size scale mapped to far end of X (depth)"),
    ]
    declares = [DeclareLaunchArgument(n, default_value=d, description=desc)
                for n, d, desc in args]
    param_cmd = []
    for n, _, _ in args:
        param_cmd += ["-p", [n + ":=", LaunchConfiguration(n)]]

    # give move_group + controllers time to come up first — same 22s margin
    # proven by loop.launch.py on this host; hand_follow.py's own preflight()
    # then waits up to another 12s for /joint_states before giving up.
    follow = TimerAction(period=22.0, actions=[
        ExecuteProcess(
            cmd=[MPVENV_PYTHON, HAND_FOLLOW_PY, "--ros-args"] + param_cmd,
            output="screen"),
    ])
    return LaunchDescription(declares + [demo, follow])
