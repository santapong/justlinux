"""Multi-robot welding cell: 3 arms + car part, ros2_control, RViz, and the
welding coordinator + effects. Watch all three weld the car body at once."""
import os
from launch import LaunchDescription
from launch.actions import TimerAction, ExecuteProcess
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory


def generate_launch_description():
    share = get_package_share_directory("weld_cell")
    urdf = os.path.join(share, "urdf", "multi_weld_cell.urdf.xacro")
    robot_desc = os.popen(f"xacro {urdf}").read()
    controllers = os.path.join(share, "config", "controllers.yaml")
    rviz = os.path.join(share, "rviz", "cell.rviz")
    scripts = os.path.join(share, "scripts")

    rsp = Node(package="robot_state_publisher", executable="robot_state_publisher",
               parameters=[{"robot_description": robot_desc}])
    cm = Node(package="controller_manager", executable="ros2_control_node",
              parameters=[{"robot_description": robot_desc}, controllers],
              output="screen")

    def spawner(name):
        return Node(package="controller_manager", executable="spawner",
                    arguments=[name, "--controller-manager", "/controller_manager"])

    spawners = [spawner("joint_state_broadcaster"),
                spawner("arm1_controller"), spawner("arm2_controller"),
                spawner("arm3_controller")]
    rviz_node = Node(package="rviz2", executable="rviz2",
                     arguments=["-d", rviz], output="screen")
    coord = ExecuteProcess(
        cmd=["python3", os.path.join(scripts, "weld_coordinator.py")],
        output="screen")
    fx = ExecuteProcess(
        cmd=["python3", os.path.join(scripts, "cell_effects.py")], output="screen")

    return LaunchDescription([
        rsp, cm, rviz_node, *spawners,
        TimerAction(period=8.0, actions=[fx]),
        TimerAction(period=12.0, actions=[coord]),
    ])
