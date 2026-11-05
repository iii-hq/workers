"""The OCI image runs what the worker ships: the bundled iii CLI provides the
`iii call` subcommand the agent prompt uses, and the environment comes from a
uv.lock that satisfies pyproject.toml."""

import re
from pathlib import Path

from src.iii_prompt import III_CONTEXT_PROMPT

DOCKERFILE = (Path(__file__).resolve().parent.parent / "Dockerfile").read_text()


def _run_steps() -> list[str]:
    """Each RUN instruction with its continuation lines joined."""
    steps, current = [], None
    for line in DOCKERFILE.splitlines():
        stripped = line.strip()
        if current is None:
            if not stripped.startswith("RUN "):
                continue
            current = ""
        current += " " + stripped.rstrip("\\").strip()
        if not stripped.endswith("\\"):
            steps.append(current.strip())
            current = None
    return steps


def test_the_prompt_calls_functions_with_iii_call():
    commands = re.findall(r"`iii (\w[\w-]*)", III_CONTEXT_PROMPT)
    assert commands, "the prompt shows iii commands"
    assert set(commands) == {"call"}, commands


def test_the_image_fails_to_build_with_a_cli_without_call():
    (install,) = [step for step in _run_steps() if "releases/download/iii/" in step]
    assert install.index("sha256sum -c") < install.index("iii --no-update-check call --help")


def test_the_image_installs_only_a_lock_that_satisfies_pyproject():
    (sync,) = [step for step in _run_steps() if "UV_PROJECT_ENVIRONMENT=/app/.venv uv sync" in step]
    assert sync.startswith("RUN env -u UV_FROZEN uv lock --check && ")
    assert "--frozen" in sync
