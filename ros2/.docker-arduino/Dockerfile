# ros2-arduino:jazzy — ROS2 base + pyserial + socat (for the no-hardware test).
# Built on the existing cell image so it already has rclpy/std_msgs.
FROM ros2-cell-full:jazzy

RUN apt-get update && apt-get install -y --no-install-recommends \
        python3-serial socat \
    && rm -rf /var/lib/apt/lists/*
