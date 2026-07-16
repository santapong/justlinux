#!/usr/bin/env python3
"""Benchmark driver: times a command N times, prints median/p95 JSON.

Modes:
  exec  — wall time of a full run (fork+exec .. exit), like a keybind firing
          a script that must finish (stash, status, screenshot).
  pty   — time from spawn to the FIRST BYTE the process writes to its pty
          (time-to-first-paint for TUIs). The child is killed afterwards.

Usage: measure.py --mode exec|pty --runs 20 --warmup 3 --name label [--env K=V ...] -- cmd args...
Environment for the child = current env + --env overrides.
"""
import argparse
import json
import os
import pty
import signal
import statistics
import subprocess
import sys
import time


def run_exec(cmd, env):
    t0 = time.monotonic()
    subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return (time.monotonic() - t0) * 1000.0


def run_pty(cmd, env):
    t0 = time.monotonic()
    pid, fd = pty.fork()
    if pid == 0:
        for k, v in env.items():
            os.environ[k] = v
        os.environ.setdefault("TERM", "xterm-256color")
        try:
            os.execvp(cmd[0], cmd)
        except OSError:
            os._exit(127)
    dt = None
    try:
        os.read(fd, 1)  # first byte the app writes to the terminal
        dt = (time.monotonic() - t0) * 1000.0
        # drain briefly so the child doesn't block on a full pty buffer
        os.set_blocking(fd, False)
        try:
            while os.read(fd, 65536):
                pass
        except (BlockingIOError, OSError):
            pass
    except OSError:
        pass
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
    os.close(fd)
    if dt is None:
        raise RuntimeError(f"no output from {cmd}")
    return dt


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["exec", "pty"], required=True)
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--warmup", type=int, default=3)
    ap.add_argument("--name", required=True)
    ap.add_argument("--env", action="append", default=[])
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    cmd = args.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    env = dict(os.environ)
    for kv in args.env:
        k, _, v = kv.partition("=")
        env[k] = v
    runner = run_exec if args.mode == "exec" else run_pty
    for _ in range(args.warmup):
        runner(cmd, env)
    samples = [runner(cmd, env) for _ in range(args.runs)]
    samples.sort()
    out = {
        "name": args.name,
        "mode": args.mode,
        "runs": args.runs,
        "median_ms": round(statistics.median(samples), 2),
        "p95_ms": round(samples[max(0, int(len(samples) * 0.95) - 1)], 2),
        "min_ms": round(samples[0], 2),
        "max_ms": round(samples[-1], 2),
        "cmd": cmd,
    }
    print(json.dumps(out))


if __name__ == "__main__":
    main()
