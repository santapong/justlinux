#!/usr/bin/env python3
"""A software stand-in for the Uno running arduino_uno_bridge.ino.

Lets you test the whole ROS2 bridge with NO hardware. Pair it with the real
bridge over two linked virtual serial ports (socat):

    socat -d -d pty,raw,echo=0 pty,raw,echo=0     # prints /dev/pts/X and /Y
    python3 fake_arduino.py /dev/pts/X            # the "Arduino" end
    ros2 run arduino_bridge serial_bridge --ros-args -p port:=/dev/pts/Y

It speaks the exact same line protocol as the firmware: emits A0/BTN at 10 Hz
(A0 as a slow sine so /arduino/analog0 visibly moves), and obeys LED/SERVO/PING.
"""
import math
import sys
import time

import serial

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/pts/3"


def main():
    ser = serial.Serial(PORT, 115200, timeout=0.05)
    ser.write(b"READY\n")
    print(f"fake Arduino on {PORT} — emitting A0/BTN at 10 Hz")
    t0 = time.time()
    led = 0
    servo = 90
    while True:
        # obey any incoming commands
        line = ser.readline().decode(errors="ignore").strip()
        if line:
            if line == "PING":
                ser.write(b"PONG\n")
            elif line.startswith("LED:"):
                led = int(line.split(":")[1] or 0)
                print(f"  LED -> {led}")
            elif line.startswith("SERVO:"):
                servo = int(line.split(":")[1] or 90)
                print(f"  SERVO -> {servo}")
        # 10 Hz sensor report: A0 as a sine 0..1023 so the topic clearly moves
        t = time.time() - t0
        a0 = int(512 + 500 * math.sin(t))
        btn = 1 if (int(t) % 4 == 0) else 0   # "pressed" 1s out of every 4
        ser.write(f"A0:{a0}\n".encode())
        ser.write(f"BTN:{btn}\n".encode())
        time.sleep(0.1)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
