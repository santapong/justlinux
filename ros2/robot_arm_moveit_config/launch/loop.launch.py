"""Pick-and-place LOOP demo: the full MoveIt demo (RViz + ros2_control) plus
the pick_place node that cycles the arm forever. Watch it in RViz.

  ros2-arm loop
"""
import os
from launch import LaunchDescription
from launch.actions import IncludeLaunchDescription, TimerAction, ExecuteProcess
from launch.launch_description_sources import PythonLaunchDescriptionSource
from ament_index_python.packages import get_package_share_directory


def generate_launch_description():
    share = get_package_share_directory("robot_arm_moveit_config")
    demo = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(
            os.path.join(share, "launch", "demo.launch.py")))
    # give move_group + controllers time to come up, then start the loop
    loop = TimerAction(period=22.0, actions=[
        ExecuteProcess(
            cmd=["python3", os.path.join(share, "scripts", "arm_pick_place.py")],
            output="screen"),
    ])
    return LaunchDescription([demo, loop])
