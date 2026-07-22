# Lean robot-arm image: ROS2 Jazzy + MoveIt2 + ros2_control
FROM osrf/ros:jazzy-desktop

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ros-jazzy-moveit \
    ros-jazzy-moveit-resources \
    ros-jazzy-moveit-setup-assistant \
    ros-jazzy-moveit-servo \
    ros-jazzy-ros2-control \
    ros-jazzy-ros2-controllers \
    ros-jazzy-controller-manager \
    ros-jazzy-xacro \
    && rm -rf /var/lib/apt/lists/*

# Fast DDS — CycloneDDS discovery breaks on this multi-NIC host
ENV RMW_IMPLEMENTATION=rmw_fastrtps_cpp
RUN echo 'source /opt/ros/jazzy/setup.bash' >> /root/.bashrc
