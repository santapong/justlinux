"""View the arm in RViz with joint sliders (jog each joint by hand).
  ros2-arm ros2 launch robot_arm_description display.launch.py
"""
import os
from launch import LaunchDescription
from launch.actions import ExecuteProcess
from launch_ros.actions import Node

URDF = "/ros2_ws/src/robot_arm_description/urdf/robot_arm.urdf.xacro"


def generate_launch_description():
    robot_desc = os.popen(f"xacro {URDF}").read()
    return LaunchDescription([
        Node(package="robot_state_publisher", executable="robot_state_publisher",
             parameters=[{"robot_description": robot_desc}]),
        Node(package="joint_state_publisher_gui",
             executable="joint_state_publisher_gui"),
        Node(package="rviz2", executable="rviz2", output="screen",
             arguments=["-d", "/ros2_ws/src/robot_arm_description/rviz/arm.rviz"]),
    ])
