"""Subprocess wrappers over the Hermes CLI.

Grounded in the documented commands:
- ``hermes -z "<prompt>"``        one-shot turn, final response text on stdout
- ``hermes send --to <platform>`` outbound message (body on stdin)
- ``hermes sessions list``        enumerate sessions
- ``HERMES_INFERENCE_MODEL``      per-run model override

Hermes runs the loop in-process and gates auth on a provisioned ``~/.hermes/.env``
(or ``hermes auth add``); these wrappers do not manage credentials.
"""

from __future__ import annotations

import asyncio
import os
import sqlite3
from pathlib import Path
from typing import Any


def _state_db() -> Path:
    home = os.environ.get("HERMES_HOME") or os.path.join(os.path.expanduser("~"), ".hermes")
    return Path(home) / "state.db"


def read_latest_usage() -> dict[str, Any] | None:
    """Read usage + cost for the most recently updated Hermes session.

    Hermes records per-session token counts and cost in its SQLite session
    store; ``hermes -z`` does not print them, so the worker reads them back
    here. Runs are serialized per session, so the latest row is this turn.
    """
    db = _state_db()
    if not db.exists():
        return None
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=2.0)
        try:
            row = con.execute(
                "SELECT input_tokens, output_tokens, cache_read_tokens, "
                "cache_write_tokens, reasoning_tokens, estimated_cost_usd, "
                "actual_cost_usd, cost_source FROM sessions ORDER BY rowid DESC LIMIT 1"
            ).fetchone()
        finally:
            con.close()
    except sqlite3.Error:
        return None
    if not row:
        return None
    inp, out, cr, cw, rea, est, act, src = row
    cost = act if act not in (None, 0) else est
    return {
        "usage": {
            "input_tokens": inp or 0,
            "output_tokens": out or 0,
            "cache_read_tokens": cr or 0,
            "cache_write_tokens": cw or 0,
            "reasoning_tokens": rea or 0,
        },
        "total_cost_usd": cost,
        "cost_source": src,
    }


async def _run(
    argv: list[str], *, cwd: str | None, env: dict[str, str] | None, stdin: str | None
) -> tuple[int, str, str]:
    proc = await asyncio.create_subprocess_exec(
        *argv,
        stdin=asyncio.subprocess.PIPE if stdin is not None else None,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=cwd or None,
        env={**os.environ, **(env or {})},
    )
    out, err = await proc.communicate(stdin.encode() if stdin is not None else None)
    return proc.returncode or 0, out.decode(errors="replace"), err.decode(errors="replace")


def _model_env(model: str) -> dict[str, str]:
    return {"HERMES_INFERENCE_MODEL": model} if model else {}


async def run_turn(
    hermes: str, prompt: str, *, cwd: str = "", model: str = "", session: str = "", toolsets: str = ""
) -> tuple[str, str]:
    """One-shot turn via ``hermes -z``. Returns (result_text, stderr).

    ``-z`` is the pure scripting entry point: only the final response on stdout.
    ``--resume <session>`` keys the turn to a worker-owned session id: the first
    turn creates that session, later turns with the same id continue it, so the
    Hermes conversation resumes across ``hermes::run`` calls.
    ``-t <toolsets>`` narrows the enabled toolsets for the turn (the Hermes
    default enables ~17, most pure context cost for a headless code turn).
    """
    argv = [hermes, "-z", prompt]
    if session:
        argv += ["--resume", session]
    if toolsets:
        argv += ["-t", toolsets]
    code, out, err = await _run(argv, cwd=cwd, env=_model_env(model), stdin=None)
    if code != 0:
        raise RuntimeError(f"hermes -z exited {code}: {err.strip() or out.strip()}")
    return out.strip(), err


async def send(hermes: str, platform: str, message: str) -> str:
    """Outbound message to a gateway platform via ``hermes send --to <platform>``."""
    code, out, err = await _run([hermes, "send", "--to", platform], cwd=None, env=None, stdin=message)
    if code != 0:
        raise RuntimeError(f"hermes send --to {platform} exited {code}: {err.strip() or out.strip()}")
    return out.strip()


async def sessions_list(hermes: str) -> str:
    """Raw ``hermes sessions list`` output (text)."""
    code, out, err = await _run([hermes, "sessions", "list"], cwd=None, env=None, stdin=None)
    if code != 0:
        raise RuntimeError(f"hermes sessions list exited {code}: {err.strip() or out.strip()}")
    return out.strip()
