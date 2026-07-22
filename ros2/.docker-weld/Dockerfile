# Welding / industrial-arm image: adds real 6-DOF robots + Cartesian planning
FROM ros2-arm:jazzy
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ros-jazzy-moveit-resources-fanuc-description \
    ros-jazzy-moveit-resources-fanuc-moveit-config \
    ros-jazzy-ur-description \
    ros-jazzy-ur-moveit-config \
    ros-jazzy-kinova-gen3-6dof-robotiq-2f-85-moveit-config \
    ros-jazzy-moveit-py \
    && rm -rf /var/lib/apt/lists/*
ENV RMW_IMPLEMENTATION=rmw_fastrtps_cpp
