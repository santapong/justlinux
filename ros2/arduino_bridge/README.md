# arduino_bridge — ROS2 ⇄ Arduino Uno R3

The Uno R3's ATmega328P has only 2 KB of RAM, so it **can't run micro-ROS**
(a real ROS2 node on-chip needs an ESP32 / Teensy / Due / Portenta). Instead
the Uno speaks a tiny **line protocol over USB serial**, and a ROS2 Python node
(`serial_bridge.py`) turns that into topics.

```
  Uno (arduino_uno_bridge.ino)  ──USB serial──▶  serial_bridge.py  ──▶  ROS2 topics
     A0, D2 button, D13 LED, D9 servo            (pyserial)             /arduino/*
```

## Topics

| Direction | Topic | Type | Meaning |
|---|---|---|---|
| Uno → ROS | `/arduino/analog0` | `std_msgs/Int32` | A0 reading, 0–1023 |
| Uno → ROS | `/arduino/button`  | `std_msgs/Bool` | D2 button (pull-up, true = pressed) |
| Uno → ROS | `/arduino/connected` | `std_msgs/Bool` | latched link status |
| ROS → Uno | `/arduino/led`   | `std_msgs/Bool` | onboard D13 LED |
| ROS → Uno | `/arduino/servo` | `std_msgs/Int32` | servo angle on D9 (0–180) |

## 1. Flash the Uno

Needs `arduino-cli` (or the Arduino IDE) on the **host** — the sketch can't be
flashed from inside a container easily.

```bash
# one-time: install arduino-cli + AVR core
curl -fsSL https://raw.githubusercontent.com/arduino/arduino-cli/master/install.sh | sh
arduino-cli core update-index
arduino-cli core install arduino:avr

# flash (adjust the port)
cd ~/ros2_ws/src/arduino_bridge/firmware
arduino-cli compile -b arduino:avr:uno arduino_uno_bridge
arduino-cli upload  -b arduino:avr:uno -p /dev/ttyACM0 arduino_uno_bridge
```

Zero external parts needed for the basic demo: **A0** and the **D13 LED** are
on-board. Optional: potentiometer → A0, button D2→GND, servo signal → D9.

## 2. Build the container image (once)

```bash
docker build -t ros2-arduino:jazzy ~/ros2_ws/.docker-arduino
```

## 3. Run

```bash
ros2-arduino run          # auto-detects ttyACM0/ttyUSB0, starts the bridge
ros2-arduino topics       # watch /arduino/analog0 move, check link status
ros2-arduino led on       # onboard LED on  (ROS → Uno)
ros2-arduino led off
ros2-arduino servo 45     # move the servo
ros2-arduino stop
```

## No hardware yet? Test the whole pipeline

`socat` makes two linked virtual serial ports; a **fake Arduino** drives one
end and the real bridge reads the other — so every ROS2 topic works exactly as
it will with the board:

```bash
ros2-arduino test         # fake Arduino ⇄ bridge, no board
ros2-arduino topics       # /arduino/analog0 sweeps as a sine
```

## Notes

- Genuine Uno → `/dev/ttyACM0`; CH340 clone → `/dev/ttyUSB0`. Pass explicitly:
  `ros2-arduino run /dev/ttyUSB0`.
- You must be in the `dialout` group: `sudo usermod -aG dialout $USER` (re-login).
- The container gets the port via `--device`; the bridge auto-reconnects on
  unplug/replug.
