# MoveIt Task Constructor built from source (not packaged for Jazzy)
FROM ros2-cell-full:jazzy
ENV DEBIAN_FRONTEND=noninteractive
RUN mkdir -p /mtc_ws/src && cd /mtc_ws/src && \
    git clone --depth 1 -b ros2 https://github.com/moveit/moveit_task_constructor.git
RUN apt-get update && cd /mtc_ws && \
    (rosdep update 2>/dev/null || true) && \
    (rosdep install --from-paths src --ignore-src -r -y 2>/dev/null || true) && \
    rm -rf /var/lib/apt/lists/*
RUN cd /mtc_ws && . /opt/ros/jazzy/setup.sh && \
    colcon build --cmake-args -DCMAKE_BUILD_TYPE=Release --parallel-workers 4
RUN echo 'source /mtc_ws/install/setup.bash' >> /root/.bashrc
ENV RMW_IMPLEMENTATION=rmw_fastrtps_cpp
