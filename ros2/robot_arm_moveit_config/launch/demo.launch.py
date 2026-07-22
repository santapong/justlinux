"""Full MoveIt2 demo for the arm: move_group planning server + RViz with
the MotionPlanning panel + mock ros2_control hardware. Drag the marker,
hit Plan & Execute — the planned trajectory runs on the (simulated) arm.

  ros2-arm ros2 launch robot_arm_moveit_config demo.launch.py
"""
from moveit_configs_utils import MoveItConfigsBuilder
from moveit_configs_utils.launches import generate_demo_launch


def generate_launch_description():
    moveit_config = (
        MoveItConfigsBuilder("robot_arm", package_name="robot_arm_moveit_config")
        .robot_description(file_path="config/robot_arm.urdf.xacro")
        .robot_description_semantic(file_path="config/robot_arm.srdf")
        .trajectory_execution(file_path="config/moveit_controllers.yaml")
        .to_moveit_configs()
    )
    return generate_demo_launch(moveit_config)
