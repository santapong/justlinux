# Theory: ROS2 x LLM, and Webcam Hand-Following Teleoperation

## Part A — The ROS2 x LLM Landscape

LLMs slot into robot systems at three distinct tiers, ordered by how close they sit to actuation:

### Tier 1 — LLM as operator/inspector (natural-language interface to existing tools)
**NASA JPL ROSA** (Robot Operating System Agent, arXiv 2410.06472, pip `jpl-rosa`) is the canonical example: a LangChain-based agent that wraps existing ROS1/ROS2 introspection tools (topic echo, node list, TF lookups, rosbag queries) so a human can converse with the robot system. Critically, ROSA does **not** do low-level actuation or motion planning — it calls existing ROS tools and services. It is extensible via custom toolsets, making it the template for a supervisory layer: start/stop behaviors, diagnostics, parameter changes, all at human conversational cadence (seconds), never inside a control loop.

The **MCP-ROS2 bridge ecosystem** (lpigeon/ros-mcp-server, kakimochi/ros2-mcp-server, LCAS/ros2_mcp, and others) generalizes this: Model Context Protocol servers expose ROS2 topics/services/actions as MCP tools so any MCP client (Claude, GPT, Gemini) can introspect or command the robot. The ecosystem is fragmented and early-stage — no canonical server exists yet, and Jazzy compatibility must be checked per-repo — but the pattern (LLM issues low-frequency commands via tools; ROS executes) is stable.

### Tier 2 — LLM as task planner over classical executors
Community projects (Dacossti/LLM-Task-Planner, the ALRM paper arXiv 2601.19510, Politecnico di Torino thesis work) follow a consistent pattern: natural language → LLM decomposes into symbolic subtasks (pick/place/move-to-pose) → subtasks handed to **MoveIt2** (arm planning) or **Nav2** (navigation) for kinematic feasibility and execution, often with YOLO-class detectors for grounding. The ecosystem is fragmented (mostly Humble-era prototypes), but MoveIt2 itself is released for Jazzy, and the LLM layer sits outside ROS core APIs so porting friction is low.

The research frontier here addresses LLM planning failure modes: **closed-loop state feedback** (re-query the LLM with updated world state after each action, per "Grounding LLMs for Robot Task Planning Using Closed-Loop State Feedback"), **symbolic verification** (LLM proposes, a PDDL-style verifier checks feasibility before execution, per "Robot Planning via LLM Proposals and Symbolic Verification"), and hierarchical splits like **BrainBody-LLM** (high-level reasoning LLM + low-level control LLM). The consensus hybrid neuro-symbolic pattern: LLM proposes → symbolic/classical layer verifies → deterministic controller executes.

### Tier 3 — End-to-end Vision-Language-Action (VLA) policies
**OpenVLA** (arXiv 2406.09246; SigLIP+DINOv2 encoder → Llama-2-7B → tokenized actions, trained on 970k Open X-Embodiment demonstrations) is the strongest open baseline, outperforming the 55B **RT-2-X** with 7x fewer parameters. **pi0 and successors** (Physical Intelligence) replaced discrete action tokenization with flow matching for smooth continuous trajectories suited to contact-rich manipulation. **RT-2 / Open X-Embodiment** (Google DeepMind + 21-institution consortium, 1.4M episodes, 22 platforms) is the foundational data/model lineage. None of these ship ROS2 integration — each would need a custom wrapper node — and all need serious GPU compute. They bypass symbolic planning entirely: perception+language → action in one network.

### The grounding problem, and the design implication for this project
LLMs operate on tokens, not physical state. Every framework needs a translation layer: ROS state (TF frames, joint states, detections) → LLM-consumable text, and LLM output → valid, collision-free, kinematically feasible commands. Raw LLM planning hallucinates and reasons poorly about spatial/numeric constraints.

**Implication adopted in this design:** the 15–25 Hz hand-following loop is latency-sensitive and safety-relevant, so it stays purely classical and deterministic (CV → filter → IK → controller). Any LLM belongs in a Tier-1 supervisory role — natural-language mode switching, motion scaling, diagnostics — at seconds-scale cadence, exactly the ROSA/MCP pattern. This mirrors every mature system surveyed: LLM-as-operator-interface, never LLM-as-low-level-controller.

## Part B — Camera Hand-Following Teleop, End-to-End

The pipeline decomposes into six stages (per the mediapipe_ros2_suite reference architecture and the Lyon Industries teleop writeup):

### 1. Perception (capture + landmark extraction)
`/dev/video0` (UVC webcam, verified 640x480 @ ~25 fps) feeds **MediaPipe HandLandmarker** (Google AI Edge Tasks API — the legacy `mp.solutions` Hands API no longer exists in mediapipe 0.10.35). The pipeline is two-stage: a full-frame palm detector plus a per-hand landmark regressor on a cropped ROI, outputting **21 3D keypoints per hand**. It is explicitly CPU-optimized (BlazePalm + lightweight CNN): the OpenPose-vs-MediaPipe comparison (Saiwa AI) measured MediaPipe ~4x faster than OpenPose/MMPose on CPU, and empirical verification in the ros2-arm:jazzy container measured **35.4 FPS with two hands detected** (28.2 ms/frame, i3-9100, VIDEO mode) — comfortably real-time without a GPU. Because the camera read (40 ms) and inference (28 ms) run sequentially at ~14.7 Hz, a capture thread (or grab/retrieve with buffer size 1) is required to overlap them and hold the camera's 25 Hz.

### 2. Handedness filter
HandLandmarker outputs a handedness classification per hand (label `Left`/`Right` + confidence). **Gotcha (Google AI Edge Hand Landmarker guide):** MediaPipe assumes a mirrored/selfie-view input. A raw, non-mirrored `/dev/video0` feed inverts the labels — either `cv2.flip()` the frame before inference or swap the label check in code. The node keeps only the hand classified as the user's LEFT hand and ignores everything else.

### 3. Coordinate mapping
Two coordinate outputs exist: `hand_landmarks` (normalized image-space x,y in [0,1], relative unitless z) and `hand_world_landmarks` (metric meters, origin at the hand's geometric center) — the latter suits IK input. **Fundamental limit:** absolute distance from a single monocular RGB frame is not recoverable; depth is relative/heuristic. Research systems either accept relative-z with human visual feedback closing the loop, calibrate with hand-geometry priors, or move to RGB-D. For teleop of a sim arm with the user watching RViz, relative depth mapped through a scaled, clamped workspace box is adequate (the approach used by mediapipe_dual_arm_control, Multi-Arm-6-DOF teleop, and FrankaTeleop). The wrist landmark becomes the end-effector position target; a wrist-to-middle-finger vector can supply orientation later (per mernaahany/teleoperation-robot-arm).

### 4. Smoothing
Raw per-frame landmarks jitter; feeding them directly produces shaky arm motion (called out as a mandatory stage in the Lyon Industries pipeline). The **One-Euro filter** (Casiez, Roussel, Vogel — CHI 2012) is the standard for noisy real-time keypoint streams: adaptive cutoff gives heavy smoothing at low speeds and low latency on fast motion. It runs per-axis on the wrist position, upstream of any robot-side shaping. (In a MoveIt Servo design, Servo's own pluggable smoothing — Butterworth / AccelerationLimited / Ruckig — would additionally shape output for kinematic limits; the two layers address different noise sources and are not redundant.)

### 5. IK / servoing
Two control paths exist (per the MoveIt Realtime Servo tutorial and the ros2_control joint_trajectory_controller Jazzy userdoc):

- **MoveIt Servo** (`moveit_servo`, released for Jazzy, verified pre-installed 2.12.4 in ros2-arm:jazzy): a C++ realtime servoing loop taking JointJog/TwistStamped/PoseStamped input, with built-in joint-limit margins, singularity soft-scaling by Jacobian condition number (a known-imperfect mechanism — moveit2 issue #1370 proposes DLS regularization), and optional collision-proximity velocity scaling. The canonical ROS2 teleop pattern (FrankaTeleop, mediapipe_dual_arm_control).
- **Direct IK + JointTrajectory streaming:** solve IK per frame and publish single-point `JointTrajectory` messages to the controller's raw topic interface (`~/joint_trajectory`, verified present in the Jazzy JTC). The topic interface is fire-and-forget with no execution monitoring but far lower overhead than the FollowJointTrajectory action (which admits one active goal and replans on each new goal — a stop-and-replan pattern, wrong for continuous streaming).

For an RViz-only sim arm, direct IK streaming is simpler and lower-latency; Servo's safety machinery becomes valuable at the hardware step. Note the verified constraint that shaped this design: the existing `arm_ik.py` full multi-seed solve measured 680–770 ms/call — unusable at frame rate — while a warm-started single descent (small per-frame target deltas) is the bounded fix.

### 6. Controller / visualization
The published `JointTrajectory` lands on `/arm_controller/joint_trajectory` (ros2_control JointTrajectoryController, 6 joints, existing config), which interpolates and updates `/joint_states`; `robot_state_publisher` broadcasts TF and RViz renders the arm following the hand. End-to-end glass-to-RViz budget: ~40 ms capture + ~28 ms inference + ~5–15 ms filter+warm-IK + 50–66 ms publish period/interpolation ≈ **100–150 ms**, which reads as "live" to a human operator.

### Sources cited
NASA JPL ROSA; MCP-ROS2 bridge servers (lpigeon/ros-mcp-server et al.); LLM task planners over MoveIt2/Nav2 (Dacossti, ALRM arXiv 2601.19510); OpenVLA (arXiv 2406.09246); pi0/Physical Intelligence (arXiv 2410.24164); RT-2 / RT-X / Open X-Embodiment; closed-loop & symbolically-verified LLM planning literature (incl. BrainBody-LLM); Google AI Edge Hand Landmarker guide; MediaPipe Hands legacy docs; Saiwa AI OpenPose-vs-MediaPipe benchmark; Lyon Industries hand-tracking teleop writeup; One-Euro filter (Casiez et al., CHI 2012); MoveIt Servo Realtime Servo tutorial + Jazzy changelog; moveit2 issue #1370; ros2_control joint_trajectory_controller Jazzy userdoc; reference implementations mediapipe_dual_arm_control, Multi-Arm-6-DOF-Robot-Tele-operation, FrankaTeleop, mediapipe_ros2_suite, mernaahany/teleoperation-robot-arm, gesturebot, husarion/v4l2-camera-docker, cv_bridge.

---

# Architecture: In-Container Hand Tracker → Tracking-Mode IK → JointTrajectory Streaming

**Chosen: Candidate A (max-reuse, minimal), amended with every fix from the three verify lenses.** All lenses returned feasible=true; each refuted specific claims as-written and supplied verified fixes, all adopted below. Candidate B (MoveIt Servo) is the designated hardware-upgrade path — moveit_servo 2.12.4 is already in the image, so no rebuild is needed to switch later. Candidate C's LLM supervisory layer is an optional final unit, kept strictly out of the control loop.

## Data flow (text diagram)

```
/dev/video0 (UVC "Web Camera", 640x480 @ ~25 fps, major 81)
    |  docker: --device /dev/video0            [FIX: existing -v /dev:/dev + --group-add video
    v         added to bin/ros2-arm             is NOT enough — cgroup rules only allow majors
[ros2-arm:jazzy container]                      166/188; empirically confirmed EPERM without it]
    |
[hand_follow node — /opt/mpvenv/bin/python]    [FIX: image has no pip; PEP 668; mediapipe deps
    |                                           need numpy 2.x. Verified recipe: apt python3-pip
    |                                           python3-venv; venv --system-site-packages
    |                                           /opt/mpvenv; pip install --ignore-installed
    |                                           mediapipe. rclpy + trajectory_msgs import OK
    |                                           from the venv. Baked into the image Dockerfile.]
    |-- capture thread: cv2.VideoCapture(0, CAP_V4L2), CAP_PROP_BUFFERSIZE=1
    |      [FIX: sequential read(40ms)+infer(28ms) = 14.7 Hz; threaded overlap holds ~25 Hz]
    |-- MediaPipe HandLandmarker, Tasks API, RunningMode.VIDEO, num_hands=2
    |      [FIX: mp.solutions is GONE in 0.10.35; requires hand_landmarker.task model file
    |       (7.8 MB, storage.googleapis.com) baked into the image at build time]
    |      [measured in-container: 35.4 FPS with 2 hands; 52.7 FPS no-hand]
    |-- handedness filter: keep LEFT hand only
    |      [mirror caveat: raw /dev/video0 is not selfie-view — cv2.flip frame or swap label]
    |-- One-Euro filter (per-axis) on wrist hand_world_landmarks
    |-- workspace map: scaled+clamped box, image/hand space -> base_link XYZ
    |      (monocular z is relative-only: scaled/clamped estimate, operator closes the loop via RViz)
    |-- tracking-mode IK (arm_ik.py variant)
    |      [FIX: stock solve() = 680-770 ms — unusable. Warm-start single _descend from the
    |       previous solution, capped iterations, small initial step; drop the weld-specific
    |       torch-down orientation term and TORCH=0.16 tool offset; target tool0/wrist directly.
    |       CHAIN geometry already verified to match robot_arm.urdf.xacro.]
    |-- publish trajectory_msgs/JointTrajectory (1 point, ~0.1 s time_from_start) @ 15-25 Hz
    v
/arm_controller/joint_trajectory  (raw topic, fire-and-forget)
    [FIX: weld_replay.py uses the ACTION interface, not this topic — it is a reference for
     message construction only; the publisher is written fresh. `~/joint_trajectory` verified
     present in the installed Jazzy libjoint_trajectory_controller.so; arm_controller is a
     JTC spawned by demo.launch.py, so the topic will exist.]
    v
ros2_control JointTrajectoryController (arm_controller, 6 joints: joint1..6)
    v
/joint_states -> robot_state_publisher -> TF -> RViz (arm follows the left hand)

Latency budget: ~40 capture + ~28 infer + ~5-15 filter+warm-IK + 50-66 publish/interp
             ≈ 100-150 ms glass-to-RViz.
```

Hygiene fixes also adopted: call `HandLandmarker.close()` explicitly (harmless `__del__` throw in 0.10.35); ignore the dual-matplotlib Axes3D UserWarning; ignore `/dev/video1` (same UVC device's metadata node).

## Reuse map onto the existing stack

| Existing asset | Location | Role in this design |
|---|---|---|
| `ros2-arm` launcher | `/home/santapong/hyprland-dots/bin/ros2-arm` | Container bring-up; gains one `--device /dev/video0` flag (line ~36, next to `--device /dev/dri`) |
| ros2-arm:jazzy image + Dockerfile | `/home/santapong/ros2_ws/.docker-arm/Dockerfile` | Rebuilt once with pip/venv/mediapipe + model file baked in (launcher runs `--rm`, so live installs don't persist) |
| `arm_ik.py` | `/home/santapong/ros2_ws/tools/arm_ik.py` | FK + IK engine; gains a tracking-mode solve variant (warm-start, de-welded cost). FK doubles as the TF-validation oracle |
| `robot_arm_description` URDF | `~/ros2_ws/src/robot_arm_description/urdf/` | Robot model, unchanged; CHAIN geometry already verified to match |
| `robot_arm_moveit_config` (demo.launch.py, ros2_controllers.yaml) | `~/ros2_ws/src/robot_arm_moveit_config/` | Spawns arm_controller + RViz, unchanged |
| `weld_replay.py` | `~/ros2_ws/src/robot_arm_moveit_config/scripts/weld_replay.py` | Reference for JointTrajectory message construction and joint naming (NOT the topic-streaming template — it uses the action interface) |
| `/arm_controller/joint_trajectory` topic | JTC raw interface | The command sink; no controller config change |
| moveit_servo 2.12.4 (pre-installed) | in ros2-arm:jazzy | Dormant; the Candidate-B upgrade path for collision/singularity-aware control, zero image change needed |
| ROSA / MCP-ROS2 (theory) | external, optional | Phase-2 supervisory layer: `/follow_enable` service + parameters exposed as LLM tools; never in the 25 Hz loop |

## Why A over B and C (verdict-aware)
- All three lenses confirmed A feasible once the launcher, install, API, and IK-rate fixes land — and all four fixes are verified working end-to-end in throwaway containers, so residual bring-up risk is low.
- For an RViz-only sim arm, B's collision checking and singularity scaling buy little, while servo.yaml bring-up is the known friction point (1–2 days). The dominant noise source (landmark jitter) is handled upstream by One-Euro in every candidate.
- A has the lowest latency and is debuggable in a single process; it matches the user's recorded inline-first / minimal-moving-parts / cost-conscious preferences.
- Upgrade paths are preserved, not burned: Servo is already installed for the hardware step; the LLM layer bolts on later as a supervisory sidecar per the ROSA pattern.

---

# Construction units (as synthesized at the gate)

## U1-camera-passthrough
**Scope:** Add --device /dev/video0 (or --device-cgroup-rule='c 81:* rmw') to the docker run in the ros2-arm launcher so V4L2 (major 81) clears the container's device-cgroup filter; leave /dev/video1 (metadata node) alone.

**Files:** /home/santapong/hyprland-dots/bin/ros2-arm

**Acceptance:** Inside a launcher-started container, cv2.VideoCapture(0, CAP_V4L2) opens and delivers 640x480 BGR frames at ~25 fps.

## U2-image-bake
**Scope:** Bake the verified install into the image: apt python3-pip python3-venv; python3 -m venv --system-site-packages /opt/mpvenv; /opt/mpvenv/bin/pip install --ignore-installed mediapipe; wget hand_landmarker.task (7.8 MB float16) to a fixed path. Rebuild ros2-arm:jazzy.

**Files:** /home/santapong/ros2_ws/.docker-arm/Dockerfile

**Acceptance:** /opt/mpvenv/bin/python imports mediapipe 0.10.35, rclpy, and trajectory_msgs together, and instantiates HandLandmarker from the baked model file with no network access.

## U3-tracking-ik
**Scope:** Add a tracking-mode solve to arm_ik.py: warm-start single _descend from the previous frame's solution (small initial step ~0.05, capped iterations), drop the torch-down orientation term and TORCH=0.16 offset, target tool0/wrist position directly; add reachability clamp and branch-consistency guard; validate FK against robot_state_publisher TF at sample poses.

**Files:** /home/santapong/ros2_ws/tools/arm_ik.py

**Acceptance:** Warm-start solve median <15 ms with position error <5 mm on a continuous target sweep, and FK matches TF within 1 mm at 5+ sampled poses.

## U4-hand-follow-node
**Scope:** New rclpy node under /opt/mpvenv/bin/python: threaded capture (BUFFERSIZE=1), HandLandmarker VIDEO mode, LEFT-hand filter with mirror-flip handling, per-axis One-Euro filter on wrist world landmarks, scaled+clamped workspace map to base_link, U3 IK, publish single-point JointTrajectory @ 15-25 Hz to /arm_controller/joint_trajectory; hold-position on hand loss; explicit landmarker.close() on shutdown.

**Files:** /home/santapong/ros2_ws/src/robot_arm_moveit_config/scripts/hand_follow.py

**Acceptance:** With demo.launch.py running, the RViz arm tracks the user's left hand at >=15 Hz command rate and does not respond to the right hand.

## U5-integration-calibration
**Scope:** One-command bring-up (launcher arg or wrapper script that starts demo.launch.py + hand_follow.py), tune workspace box scale/offsets and One-Euro parameters (min_cutoff/beta) against real hand motion, verify end-to-end latency budget, document run steps in the repo.

**Files:** /home/santapong/hyprland-dots/bin/ros2-arm, /home/santapong/ros2_ws/src/robot_arm_moveit_config/scripts/hand_follow.py

**Acceptance:** Single command from cold start to a live-following arm; motion is smooth (no visible jitter at rest, no overshoot on fast moves) with glass-to-RViz latency ~100-150 ms.

## U6-llm-supervisor (optional)
**Scope:** Supervisory LLM layer per the ROSA/MCP pattern: expose /follow_enable service, motion-scale parameter, home pose, and follower diagnostics as tools (jpl-rosa custom toolset or a minimal MCP-ROS2 server) so natural-language commands gate the follower at seconds-scale cadence; LLM never enters the 25 Hz loop. Defer unless the user asks — API cost vs. recorded cost-conscious preference.

**Files:** /home/santapong/ros2_ws/src/robot_arm_moveit_config/scripts/hand_follow.py, /home/santapong/ros2_ws/tools/

**Acceptance:** A natural-language 'stop following' / 'follow at half speed' round-trips through the LLM toolset and visibly changes follower behavior without perturbing control-loop timing.


---

# Risks
- IK tracking robustness: the warm-start single-descent fix is bounded (~1-2 h estimate) but unproven at frame rate over the whole workspace — fast hand motion enlarges per-frame target deltas, risking lag, local-minimum stalls, or elbow-branch flips; mitigation is capped step size, reachability clamping, and falling back to a short multi-seed re-solve on divergence (U3 acceptance gates this).
- arm_ik.py frame conventions were built for the weld-arm torch path; although CHAIN geometry was verified against robot_arm.urdf.xacro, the de-welded cost function must be validated against TF at multiple poses before trusting it across the workspace (folded into U3 acceptance).
- Monocular depth is fundamentally relative: MediaPipe z is a heuristic, so Z tracking will be the least faithful axis — set expectations (operator closes the loop visually via RViz), clamp Z hard, and note RGB-D as the upgrade if precision Z matters.
- Handedness label inversion: MediaPipe assumes a mirrored selfie view; if the flip/label-swap is wrong the node follows the RIGHT hand — verify with a single-hand test on day one.
- Environment fragility in the venv: mediapipe pulls numpy 2.5.1 + opencv-contrib 5.0 which shadow the system numpy 1.26.4 / cv2 4.6.0 inside /opt/mpvenv; any future in-venv use of ROS Python packages with compiled numpy-1.x bindings (beyond the verified rclpy/trajectory_msgs path) may break — keep the node's import surface minimal.
- Image rebuild coupling: the launcher runs --rm containers, so all deps must be baked (U2); the model-file wget needs network at build time, and a mediapipe version drift on a future rebuild could reintroduce the resolver failures the lenses already hit — pin mediapipe==0.10.35 in the Dockerfile.
- No collision, joint-limit-velocity, or singularity handling in the direct-IK path: fine for the RViz-only sim, categorically insufficient for real hardware — the MoveIt Servo upgrade (already installed, 2.12.4) is mandatory before any physical arm, and Servo's own singularity handling has known limitations (moveit2 #1370).
- Frame-rate ceiling: the camera caps the loop at ~25 fps and a naive sequential loop lands at ~14.7 Hz, below target — the capture-thread fix is required (U4), and CPU contention from RViz + inference on the 4-core host could still erode the rate.
- Fire-and-forget topic streaming has no execution monitoring: a dying controller or dropped messages fail silently — watch /joint_states staleness in the node as a cheap health check.
- Scope creep on the LLM layer: the researched ROS2+LLM value here is supervisory only; pulling an LLM toward the control loop adds cost and latency for negative benefit and contradicts the user's recorded cost-conscious workflow preference — U6 stays optional and gated.


---

# Critic gaps (to fold into Construction)
- LOAD-BEARING UNVERIFIED CLAIM (U4): the plan tracks 'wrist world landmarks', but MediaPipe hand_world_landmarks are expressed relative to the hand's own geometric center — the wrist world coordinate barely changes when the hand translates across the frame, so it cannot drive absolute position tracking. The camera lens verified world-landmark VALUES on a static reference image but never verified they vary with hand position. The node must use normalized image landmarks (x,y) plus MediaPipe's heuristic z (or bounding-box scale as a depth proxy); no lens tested that path, and U4's design as written would produce a near-stationary arm.
- Camera-to-robot axis mapping is not designed: no convention for which camera axis maps to which base_link axis, no handedness/mirror decision for intuitive motion (hand moves left → arm moves which way?), and U5's 'tune scale/offsets' has no procedure. U4 acceptance only checks left-vs-right hand filtering, not that the mapping direction is ergonomic — a fully inverted mapping would pass all stated acceptance criteria.
- Re-acquisition jump handling is missing: hold-position on hand loss is specified, but when the hand is re-detected far from the held pose the target teleports and the arm snaps. No slew-rate limit on the target, no per-frame joint-delta clamp, and no hand-loss timeout/decay policy — the risks list notes absent velocity limiting but no unit owns even a cheap joint-space delta clamp for the sim.
- JointTrajectory streaming semantics un-researched: what time_from_start to stamp on the single point, and how joint_trajectory_controller behaves under 15-25 Hz trajectory replacement (preemption, interpolation, open_loop_control setting) determines smoothness. The control-path lens confirmed the topic exists but nobody researched or tested rapid-replacement behavior; jerky motion here would fail U5's smoothness acceptance for reasons unrelated to the One-Euro filter.
- Joint name/order correctness unverified for a fresh topic publisher: weld_replay.py used the action interface, so there is no in-repo example of the raw topic message; U4 has no acceptance that published joint names match the controller's configured joint list (JTC silently rejects or misbehaves on mismatch).
- U2 contradicts the risks list: scope says 'pip install --ignore-installed mediapipe' UNPINNED while the risks demand pinning mediapipe==0.10.35; the model wget uses the 'latest' URL path with no checksum, so an image rebuild is non-reproducible on two axes. Also two verified install recipes exist (venv --ignore-installed with numpy 2.5.1 vs --break-system-packages with numpy==1.26.4 pin, which avoids the numpy-shadowing landmine entirely) and no rationale is recorded for choosing the venv variant.
- Frame-rate numbers across lenses are unreconciled and none were measured under real load: 20.3 fps sustained capture+inference (camera lens) vs 35.4 fps inference-only (vision lens) vs the 25 Hz camera cap, and no benchmark ran with RViz + robot_state_publisher + the IK loop competing for the 4 cores. The capture-thread fix is asserted, not measured — U4's >=15 Hz acceptance rests on an untested budget.
- The stated goal 'research ROS2+LLM theory' has no deliverable: no cited survey of ROS2+LLM integration patterns exists anywhere in the units — U6 name-drops ROSA/MCP but nothing verifies jpl-rosa supports Jazzy/Python 3.12, that any MCP-ROS2 server actually exists and works, that the container has network access for LLM API calls, or what the API cost would be (directly relevant to the recorded cost-conscious preference).
- U5's latency acceptance (~100-150 ms glass-to-RViz) has no measurement method: no instrumentation plan (e.g., LED/phone-timer in frame vs RViz motion, or per-stage timestamps), so the acceptance criterion is currently unfalsifiable.
- Runtime enable/disable is gated behind the optional LLM unit: without U6 there is no /follow_enable service, deadman gesture, or keyboard toggle — the follower runs unconditionally from launch until the node is killed. A cheap non-LLM enable/disable belongs in U4/U5 regardless of whether U6 ships (it is also the substrate U6 would need anyway).
- The /joint_states staleness health check is named in the risks as the mitigation for silent controller death but is assigned to no unit and appears in no acceptance criterion — it will not get built.
- Detection-policy details unspecified: num_hands setting, min detection/tracking confidence thresholds, and the selection rule when two left hands (two people) or a false-positive left hand appear — no tie-break policy and no acceptance on detection reliability.
- Lighting robustness unverified: UVC webcams halve frame rate under auto-exposure in dim light and MediaPipe detection degrades; all measurements were done at one (unstated) lighting condition, and Z-clamp bounds plus the workspace box are given no concrete numbers anywhere.
- U1 has no non-regression acceptance: the launcher edit touches the shared /home/santapong/hyprland-dots/bin/ros2-arm used for existing workflows (serial majors 166/188, weld demo) — nothing checks those still work after adding the video device rule.
- One-Euro filter is underspecified: no source decided (hand-rolled vs dependency — nothing in the venv provides it), and the filter's operating space is ambiguous (normalized image coords, mapped meters, or joint space), which interacts with the world-vs-image landmark gap; filtering in the wrong space changes the meaning of the min_cutoff/beta tuning U5 depends on.
- Shutdown/exit behavior beyond landmarker.close() is unspecified: whether the arm parks/homes or freezes mid-pose on node exit, and what the one-command bring-up (U5) does on partial failure (camera busy, model file missing, controller not up) — no startup preflight checks are scoped.
