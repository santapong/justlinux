/*
 * arduino_uno_bridge — a tiny serial protocol so a ROS2 node can talk to an
 * Arduino Uno R3 over USB. The Uno's ATmega328P (2 KB RAM) is too small for
 * micro-ROS, so we speak a simple line protocol instead and let the ROS2 side
 * (serial_bridge.py) translate it to/from topics.
 *
 * Baud: 115200
 *
 * Arduino -> host  (sent ~10 Hz):
 *     A0:<0-1023>     analog reading on pin A0  (pot / sensor / floating)
 *     BTN:<0|1>       button on D2 (INPUT_PULLUP, so 1 = pressed)
 *     PONG            reply to a PING
 *
 * host -> Arduino  (one command per line, '\n' terminated):
 *     LED:1 / LED:0   onboard LED on D13            (no extra parts needed)
 *     SERVO:<0-180>   servo angle on D9            (optional, needs a servo)
 *     PING            health check -> Arduino replies PONG
 *
 * Wiring for the zero-parts demo: nothing — D13 LED and A0 are on-board.
 * Optional: potentiometer wiper -> A0, button D2->GND, servo signal -> D9.
 */
#include <Servo.h>

const uint8_t PIN_LED   = 13;
const uint8_t PIN_BTN   = 2;
const uint8_t PIN_SERVO = 9;
const unsigned long REPORT_MS = 100;   // 10 Hz sensor reporting

Servo servo;
char  buf[24];
uint8_t idx = 0;
unsigned long lastReport = 0;

void applyCommand(char *line) {
  // split on ':'
  char *sep = strchr(line, ':');
  if (sep == NULL) {
    if (strcmp(line, "PING") == 0) Serial.println("PONG");
    return;
  }
  *sep = '\0';
  const char *key = line;
  int val = atoi(sep + 1);

  if (strcmp(key, "LED") == 0) {
    digitalWrite(PIN_LED, val ? HIGH : LOW);
  } else if (strcmp(key, "SERVO") == 0) {
    val = constrain(val, 0, 180);
    servo.write(val);
  }
}

void setup() {
  Serial.begin(115200);
  pinMode(PIN_LED, OUTPUT);
  pinMode(PIN_BTN, INPUT_PULLUP);
  servo.attach(PIN_SERVO);
  servo.write(90);
  Serial.println("READY");   // handshake so the host knows we booted
}

void loop() {
  // read incoming commands, line by line
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\n' || c == '\r') {
      if (idx > 0) { buf[idx] = '\0'; applyCommand(buf); idx = 0; }
    } else if (idx < sizeof(buf) - 1) {
      buf[idx++] = c;
    }
  }

  // periodic sensor report
  unsigned long now = millis();
  if (now - lastReport >= REPORT_MS) {
    lastReport = now;
    Serial.print("A0:");
    Serial.println(analogRead(A0));
    Serial.print("BTN:");
    Serial.println(digitalRead(PIN_BTN) == LOW ? 1 : 0);  // pullup: LOW=pressed
  }
}
