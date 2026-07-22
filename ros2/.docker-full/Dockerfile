FROM ros2-weld:jazzy
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ros-jazzy-rqt-plot ros-jazzy-rqt-topic ros-jazzy-rqt-graph \
    ros-jazzy-rqt-tf-tree ros-jazzy-rqt-controller-manager \
    && rm -rf /var/lib/apt/lists/*
ENV RMW_IMPLEMENTATION=rmw_fastrtps_cpp
