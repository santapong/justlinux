"""FULL weld-cell system in one launch:
  robot_state_publisher · ros2_control (3 controllers) · move_group (with the
  SRDF collision matrix) · the weld coordinator · welding effects · RViz.
Bring up rqt_graph / rqt_plot alongside to watch the data flow."""
import os
from launch import LaunchDescription
from launch.actions import TimerAction, ExecuteProcess
from launch_ros.actions import Node
from moveit_configs_utils import MoveItConfigsBuilder
from ament_index_python.packages import get_package_share_directory


def generate_launch_description():
    share = get_package_share_directory("weld_cell")
    moveit_config = (
        MoveItConfigsBuilder("weld_cell", package_name="weld_cell")
        .robot_description(
            file_path=os.path.join(share, "urdf", "multi_weld_cell.urdf.xacro"))
        .robot_description_semantic(file_path="config/weld_cell.srdf")
        .robot_description_kinematics(file_path="config/kinematics.yaml")
        .joint_limits(file_path="config/joint_limits.yaml")
        .to_moveit_configs()
    )
    controllers = os.path.join(share, "config", "controllers.yaml")
    scripts = os.path.join(share, "scripts")
    rviz = os.path.join(share, "rviz", "cell.rviz")

    rsp = Node(package="robot_state_publisher", executable="robot_state_publisher",
               parameters=[moveit_config.robot_description])
    cm = Node(package="controller_manager", executable="ros2_control_node",
              parameters=[moveit_config.robot_description, controllers],
              output="screen")
    move_group = Node(package="moveit_ros_move_group", executable="move_group",
                      output="screen", parameters=[moveit_config.to_dict()])
    rviz_node = Node(package="rviz2", executable="rviz2", arguments=["-d", rviz])

    def spawner(name):
        return Node(package="controller_manager", executable="spawner",
                    arguments=[name, "--controller-manager", "/controller_manager"])

    spawners = [spawner(n) for n in ("joint_state_broadcaster",
                                     "arm1_controller", "arm2_controller",
                                     "arm3_controller")]
    coord = ExecuteProcess(
        cmd=["python3", os.path.join(scripts, "weld_coordinator.py")],
        output="screen")
    fx = ExecuteProcess(
        cmd=["python3", os.path.join(scripts, "cell_effects.py")], output="screen")

    return LaunchDescription([
        rsp, cm, move_group, rviz_node,
        TimerAction(period=4.0, actions=spawners),
        TimerAction(period=9.0, actions=[fx]),
        TimerAction(period=13.0, actions=[coord]),
    ])
