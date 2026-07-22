# Lean robot-arm image: ROS2 Jazzy + MoveIt2 + ros2_control
FROM osrf/ros:jazzy-desktop

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ros-jazzy-moveit \
    ros-jazzy-moveit-resources \
    ros-jazzy-moveit-setup-assistant \
    ros-jazzy-moveit-servo \
    ros-jazzy-moveit-visual-tools \
    ros-jazzy-joint-state-publisher-gui \
    ros-jazzy-ros2-control \
    ros-jazzy-ros2-controllers \
    ros-jazzy-controller-manager \
    ros-jazzy-xacro \
    && rm -rf /var/lib/apt/lists/*

# Fast DDS — CycloneDDS discovery breaks on this multi-NIC host
ENV RMW_IMPLEMENTATION=rmw_fastrtps_cpp
RUN echo 'source /opt/ros/jazzy/setup.bash' >> /root/.bashrc

# MediaPipe hand tracking, isolated in a venv (system numpy/cv2 stay untouched;
# PEP-668 blocks pip on the system python otherwise). --system-site-packages
# lets rclpy / trajectory_msgs still import from inside the venv.
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3-pip \
    python3-venv \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv --system-site-packages /opt/mpvenv \
    && /opt/mpvenv/bin/pip install --no-cache-dir --ignore-installed mediapipe==0.10.35

# Bake the hand landmarker model at build time so runtime needs no network.
RUN mkdir -p /opt/models \
    && curl -fL -o /opt/models/hand_landmarker.task \
       https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/latest/hand_landmarker.task \
    && sha256sum /opt/models/hand_landmarker.task > /opt/models/hand_landmarker.task.sha256 \
    && cat /opt/models/hand_landmarker.task.sha256
