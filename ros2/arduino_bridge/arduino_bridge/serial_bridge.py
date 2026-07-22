#!/usr/bin/env python3
"""ROS2 <-> Arduino Uno R3 serial bridge.

Speaks the line protocol in firmware/arduino_uno_bridge.ino over a USB serial
port and exposes it as ROS2 topics:

  published (Arduino -> ROS):
    /arduino/analog0   std_msgs/Int32   A0 reading (0-1023)
    /arduino/button    std_msgs/Bool    D2 button (true = pressed)
    /arduino/connected std_msgs/Bool    latched link status

  subscribed (ROS -> Arduino):
    /arduino/led       std_msgs/Bool    onboard D13 LED
    /arduino/servo     std_msgs/Int32   servo angle on D9 (0-180)

Parameters:
    port  (string, default /dev/ttyACM0)   Uno usually enumerates as ttyACM0;
                                           clones with CH340 show as ttyUSB0
    baud  (int,    default 115200)

Robust to unplug/replug: if the port drops it keeps retrying to reopen.
Runs with `ros2 run arduino_bridge serial_bridge` or directly with python3.
"""
import threading
import time

import rclpy
from rclpy.node import Node
from rclpy.qos import QoSProfile, DurabilityPolicy
from std_msgs.msg import Int32, Bool

try:
    import serial  # pyserial
except ImportError:  # pragma: no cover
    raise SystemExit("pyserial not installed — `apt-get install python3-serial` "
                     "or `pip install pyserial`")


class ArduinoBridge(Node):
    def __init__(self):
        super().__init__("arduino_bridge")
        self.declare_parameter("port", "/dev/ttyACM0")
        self.declare_parameter("baud", 115200)
        self.port = self.get_parameter("port").value
        self.baud = int(self.get_parameter("baud").value)

        # latched status so late subscribers still see the current link state
        latched = QoSProfile(depth=1, durability=DurabilityPolicy.TRANSIENT_LOCAL)
        self.pub_a0 = self.create_publisher(Int32, "/arduino/analog0", 10)
        self.pub_btn = self.create_publisher(Bool, "/arduino/button", 10)
        self.pub_conn = self.create_publisher(Bool, "/arduino/connected", latched)

        self.create_subscription(Bool, "/arduino/led", self.on_led, 10)
        self.create_subscription(Int32, "/arduino/servo", self.on_servo, 10)

        self.ser = None
        self._lock = threading.Lock()
        self._publish_connected(False)

        # serial I/O lives on its own thread; rclpy spins the callbacks
        self._stop = False
        self._reader = threading.Thread(target=self._serial_loop, daemon=True)
        self._reader.start()
        self.get_logger().info(
            f"arduino_bridge up — port={self.port} baud={self.baud}")

    # ---- ROS -> Arduino ----
    def _write(self, line: str):
        with self._lock:
            if self.ser and self.ser.is_open:
                try:
                    self.ser.write((line + "\n").encode())
                except serial.SerialException as e:
                    self.get_logger().warn(f"write failed: {e}")

    def on_led(self, msg: Bool):
        self._write(f"LED:{1 if msg.data else 0}")

    def on_servo(self, msg: Int32):
        angle = max(0, min(180, int(msg.data)))
        self._write(f"SERVO:{angle}")

    # ---- Arduino -> ROS ----
    def _publish_connected(self, ok: bool):
        m = Bool()
        m.data = ok
        self.pub_conn.publish(m)

    def _handle_line(self, line: str):
        if ":" not in line:
            if line == "PONG":
                self.get_logger().debug("PONG")
            elif line == "READY":
                self.get_logger().info("Arduino booted (READY)")
            return
        key, _, val = line.partition(":")
        try:
            if key == "A0":
                self.pub_a0.publish(Int32(data=int(val)))
            elif key == "BTN":
                self.pub_btn.publish(Bool(data=(val.strip() == "1")))
        except ValueError:
            pass  # ignore malformed lines / serial noise

    def _serial_loop(self):
        """Open the port, read lines, reopen on failure — forever."""
        while not self._stop:
            try:
                with self._lock:
                    self.ser = serial.Serial(self.port, self.baud, timeout=1.0)
                self.get_logger().info(f"opened {self.port}")
                self._publish_connected(True)
                self._write("PING")
                while not self._stop:
                    raw = self.ser.readline()
                    if raw:
                        self._handle_line(raw.decode(errors="ignore").strip())
            except (serial.SerialException, OSError) as e:
                self.get_logger().warn(
                    f"{self.port} unavailable ({e}); retrying in 2s")
                self._publish_connected(False)
                with self._lock:
                    if self.ser:
                        try:
                            self.ser.close()
                        except Exception:
                            pass
                    self.ser = None
                time.sleep(2.0)

    def destroy_node(self):
        self._stop = True
        with self._lock:
            if self.ser:
                try:
                    self.ser.close()
                except Exception:
                    pass
        super().destroy_node()


def main(args=None):
    rclpy.init(args=args)
    node = ArduinoBridge()
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        node.destroy_node()
        rclpy.shutdown()


if __name__ == "__main__":
    main()
