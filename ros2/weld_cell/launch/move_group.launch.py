"""move_group for the cell — loads the URDF, the SRDF collision matrix, and
kinematics for all 3 arm groups. Also runs robot_state_publisher + the
ros2_control controllers so the planning scene has live joint states."""
import os
from launch import LaunchDescription
from launch.actions import TimerAction
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

    move_group = Node(
        package="moveit_ros_move_group", executable="move_group",
        output="screen", parameters=[moveit_config.to_dict()])
    rsp = Node(package="robot_state_publisher", executable="robot_state_publisher",
               parameters=[moveit_config.robot_description])
    controllers = os.path.join(share, "config", "controllers.yaml")
    cm = Node(package="controller_manager", executable="ros2_control_node",
              parameters=[moveit_config.robot_description, controllers],
              output="screen")

    def spawner(name):
        return Node(package="controller_manager", executable="spawner",
                    arguments=[name, "--controller-manager", "/controller_manager"])

    spawners = [spawner(n) for n in ("joint_state_broadcaster",
                                     "arm1_controller", "arm2_controller",
                                     "arm3_controller")]
    return LaunchDescription([rsp, cm, move_group,
                              TimerAction(period=4.0, actions=spawners)])
