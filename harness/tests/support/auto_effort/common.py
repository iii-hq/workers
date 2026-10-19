"""Shared constants and helpers for the auto-effort phase 0 scripts."""

import json
import os
import subprocess
from pathlib import Path

LEVELS = ("low", "medium", "high", "xhigh")
WEIGHTS = {"input": 1, "cache_write": 1.25, "cache_read": 0.1, "output": 5}
# USD per million input tokens (provider-anthropic/src/curated.rs): G3 pools models in dollars (R16)
INPUT_USD = {"claude-opus-5-5": 4.0, "claude-sonnet-5-5": 2.0, "claude-fable-5-1": 10.0, "claude-haiku-5-5": 0.10}


class TriggerError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


def units(usage: dict | None) -> float:
    """Uncached-input-token equivalents of one usage record."""
    usage = usage or {}
    return float(sum(weight * (usage.get(key) or 0) for key, weight in WEIGHTS.items()))


def trigger(fn: str, payload: dict, timeout_s: float) -> dict:
    timeout_ms = str(round(timeout_s * 1000))
    argv = ["iii", "trigger", "-n", "my-project", "--timeout-ms", timeout_ms, fn, "--json", json.dumps(payload)]
    done = subprocess.run(argv, capture_output=True, encoding="utf-8")
    if done.returncode == 0:
        return json.loads(done.stdout) if done.stdout.strip() else {}
    stderr = done.stderr.strip()
    try:
        error = json.loads(stderr.removeprefix("Error: "))
    except ValueError:
        error = None
    if done.returncode == 1 and isinstance(error, dict) and "code" in error:
        raise TriggerError(error["code"], error.get("message", ""))
    raise TriggerError("transport", stderr)


def read_jsonl(path: Path) -> list[dict]:
    with open(path, encoding="utf-8") as f:
        return [json.loads(raw) for raw in f if raw.strip()]


def write_jsonl(path: Path, rows: list[dict]) -> None:
    """Rewrites path whole, through a temporary file that replaces it: a stop mid-write leaves the old file intact."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        f.writelines(json.dumps(row, ensure_ascii=False) + "\n" for row in rows)
    os.replace(tmp, path)
