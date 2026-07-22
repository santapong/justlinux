"""Launch the ROS2 <-> Arduino Uno serial bridge.

    ros2 launch arduino_bridge arduino_bridge.launch.py port:=/dev/ttyACM0
"""
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node


def generate_launch_description():
    port = LaunchConfiguration("port")
    baud = LaunchConfiguration("baud")
    return LaunchDescription([
        DeclareLaunchArgument("port", default_value="/dev/ttyACM0",
                              description="serial device (ttyACM0 Uno / ttyUSB0 clone)"),
        DeclareLaunchArgument("baud", default_value="115200"),
        Node(
            package="arduino_bridge",
            executable="serial_bridge",
            name="arduino_bridge",
            output="screen",
            parameters=[{"port": port, "baud": baud}],
        ),
    ])
