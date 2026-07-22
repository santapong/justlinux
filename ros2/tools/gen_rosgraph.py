#!/usr/bin/env python3
"""Render the live ROS2 graph (nodes + topics + who publishes/subscribes)
to a PNG with graphviz — the data-flow picture of the whole system."""
import subprocess
import re

SKIP_T = {"/parameter_events", "/rosout", "/rosout_agg"}


def sh(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True,
                          text=True).stdout


nodes = [n.strip() for n in sh("ros2 node list").splitlines() if n.strip()]
pubs, subs = {}, {}   # topic -> set(nodes)
for n in nodes:
    info = sh(f"ros2 node info {n}")
    section = None
    for line in info.splitlines():
        s = line.strip()
        if s.startswith("Publishers:"):
            section = "pub"
        elif s.startswith("Subscribers:"):
            section = "sub"
        elif s.startswith(("Service", "Action")):
            section = None
        elif section and ":" in s:
            topic = s.split(":")[0].strip()
            if topic in SKIP_T or not topic.startswith("/"):
                continue
            (pubs if section == "pub" else subs).setdefault(
                topic, set()).add(n)

topics = set(pubs) | set(subs)
dot = ['digraph ros {', '  rankdir=LR;', '  bgcolor="#1e1e22";',
       '  node [fontname="JetBrains Mono", fontsize=11];',
       '  edge [color="#888888"];']
for n in nodes:
    label = n.strip("/")
    dot.append(f'  "{n}" [shape=ellipse, style=filled, '
               f'fillcolor="#D97757", fontcolor="#111111", label="{label}"];')
for t in sorted(topics):
    dot.append(f'  "{t}" [shape=box, style=filled, '
               f'fillcolor="#3a3a40", fontcolor="#e0e0e0", label="{t}"];')
for t, ns in pubs.items():
    for n in ns:
        dot.append(f'  "{n}" -> "{t}";')
for t, ns in subs.items():
    for n in ns:
        dot.append(f'  "{t}" -> "{n}";')
dot.append("}")
open("/tmp/rosgraph.dot", "w").write("\n".join(dot))
sh("dot -Tpng /tmp/rosgraph.dot -o /ros2_ws/rosgraph.png")
print(f"{len(nodes)} nodes, {len(topics)} topics -> /ros2_ws/rosgraph.png")
