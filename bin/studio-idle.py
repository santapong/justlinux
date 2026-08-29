#!/usr/bin/env python3
"""Idle CPU of every hypr-claude-studio --sidebar over N seconds (default 30)."""
import glob, sys, time
def pids():
    out = []
    for d in glob.glob("/proc/[0-9]*"):
        try:
            c = open(d + "/cmdline", "rb").read().split(b"\0")
        except OSError:
            continue
        if c and c[0].endswith(b"hypr-claude-studio") and b"--sidebar" in c:
            out.append(int(d[6:]))
    return sorted(out)
def ticks(p):
    f = open(f"/proc/{p}/stat").read().rsplit(")", 1)[1].split()
    return int(f[11]) + int(f[12]), int(f[13]) + int(f[14])
n = int(sys.argv[1]) if len(sys.argv) > 1 else 30
ps = pids(); a = {p: ticks(p) for p in ps}; time.sleep(n); tot = 0
for p in ps:
    o, c = ticks(p); tot += o - a[p][0] + c - a[p][1]
    print(p, "own", o - a[p][0], "children", c - a[p][1])
print(f"TOTAL {len(ps)} sidebars: {tot / n:.2f}% CPU over {n}s")
