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


async def run_turn(hermes: str, prompt: str, *, cwd: str = "", model: str = "", session: str = "") -> tuple[str, str]:
    """One-shot turn via ``hermes -z``. Returns (result_text, stderr).

    ``-z`` is the pure scripting entry point: only the final response on stdout.
    ``--resume <session>`` keys the turn to a worker-owned session id: the first
    turn creates that session, later turns with the same id continue it, so the
    Hermes conversation resumes across ``hermes::run`` calls.
    """
    argv = [hermes, "-z", prompt]
    if session:
        argv += ["--resume", session]
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
