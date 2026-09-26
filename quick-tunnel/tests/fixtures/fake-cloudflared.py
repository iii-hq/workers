#!/usr/bin/python3
"""Deterministic child fixture: emits JSON only, never opens sockets."""
import json
import os
from pathlib import Path
import signal
import sys
import time

base = Path(__file__).parent
mode = (base / "mode").read_text().strip()
args = sys.argv[1:]
assert args[0] == "tunnel"
assert args[args.index("--output") + 1] == "json"
assert args[args.index("--metrics") + 1] == "127.0.0.1:0"
assert args[args.index("--url") + 1] == "http://127.0.0.1:3112"
assert "--no-autoupdate" in args
assert not any(k.startswith("TUNNEL_") or k.startswith("III_") for k in os.environ)
assert Path(args[args.index("--config") + 1]).read_text() == "{}\n"
with (base / "pids").open("a") as file:
    file.write(str(os.getpid()) + "\n")

def emit(**fields):
    print(json.dumps(fields), file=sys.stderr, flush=True)

def connected():
    emit(level="info", message="Registered tunnel connection", connIndex=0,
         connection="72ff9dc2-84c6-4bb5-a536-349cf6dfe71d", protocol="quic")

if mode == "crash":
    sys.exit(23)
if mode == "oversized":
    print("x" * 20000, file=sys.stderr, flush=True)
elif mode != "silent":
    emit(level="info", message=f"|  https://fake-{os.getpid()}.trycloudflare.com  |")
if mode in ("ready", "disconnect", "reconnect", "ignore-term"):
    connected()
if mode == "ignore-term":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
if mode in ("disconnect", "reconnect"):
    time.sleep(0.2)
    emit(level="error", message="Serve tunnel error", connIndex=0, error="fixture secret must never escape")
    if mode == "reconnect":
        time.sleep(0.05)
        connected()
while True:
    time.sleep(1)
