"""Regression tests for the Linux IPv4 namespaced fixture's port safety."""
from __future__ import annotations

import errno
import json
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

import pytest

STACK_SCRIPT = Path(__file__).with_name("namespaced-provider-stack.py")
sys.path.insert(0, str(Path(__file__).parent))
import namespaced_provider_ports as ports  # noqa: E402
from namespaced_provider_ports import (  # noqa: E402
    is_address_in_use,
    listener_is_owned_by_process_group,
    listener_owner_pids,
    release_reservations,
    require_linux_ipv4_listener_support,
    reserve_loopback_port,
)


def launcher_environment(root: Path, **overrides: str) -> dict[str, str]:
    """Build an isolated environment for deterministic early-launcher tests."""
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("NAMESPACED_PROVIDER_") and key != "READY_FILE"
    }
    environment.update(
        {
            "NAMESPACED_PROVIDER_FIXTURE_ROOT": str(root),
            "III_BIN": sys.executable,
            "STATE_BIN": sys.executable,
            "ROUTER_BIN": sys.executable,
            "QUEUE_BIN": sys.executable,
            "HARNESS_BIN": sys.executable,
            "CONSOLE_BIN": sys.executable,
        },
    )
    environment.update(overrides)
    return environment


def write_foreign_engine(path: Path) -> None:
    """Create an engine whose foreign-session grandchild owns the engine port."""
    path.write_text(
        "#!/usr/bin/env python3\n"
        "import json, os, subprocess, sys, time\n"
        "config = json.load(open(sys.argv[sys.argv.index('--config') + 1]))\n"
        "port = config['workers'][0]['config']['port']\n"
        "probe = os.environ['PROBE_DIR']\n"
        "child = (\n"
        "  'import os,socket\\n'\n"
        "  'sock=socket.socket(socket.AF_INET,socket.SOCK_STREAM)\\n'\n"
        "  'sock.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)\\n'\n"
        "  f'sock.bind((\"127.0.0.1\",{port}))\\n'\n"
        "  'sock.listen()\\n'\n"
        "  f'open(\"{probe}/pids\",\"a\").write(str(os.getpid())+\"\\\\n\")\\n'\n"
        "  'while True:\\n'\n"
        "  ' connection,_=sock.accept()\\n'\n"
        "  f' open(\"{probe}/accepts\",\"a\").write(\"accept\\\\n\")\\n'\n"
        "  ' connection.close()\\n'\n"
        ")\n"
        "subprocess.Popen([sys.executable, '-c', child], start_new_session=True)\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    path.chmod(0o755)


def terminate_foreign_children(probe_dir: Path) -> None:
    """Terminate and verify probe grandchildren outside fixture process groups."""
    pids = probe_dir / "pids"
    if not pids.exists():
        return
    process_ids = {int(raw_pid) for raw_pid in pids.read_text(encoding="utf-8").splitlines()}
    for process_id in process_ids:
        try:
            os.kill(process_id, signal.SIGTERM)
        except ProcessLookupError:
            continue

    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        live = {
            process_id
            for process_id in process_ids
            if (Path(f"/proc/{process_id}/stat").exists()
                and Path(f"/proc/{process_id}/stat").read_text(encoding="utf-8").split()[2] != "Z")
        }
        if not live:
            return
        time.sleep(0.01)
    for process_id in live:
        try:
            os.kill(process_id, signal.SIGKILL)
        except ProcessLookupError:
            continue
    raise AssertionError(f"foreign probe children survived teardown: {sorted(live)}")


def pick_unused_port() -> int:
    """Return a test-only explicit port; the launcher owns collision detection."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def test_linux_ipv4_listener_support_rejects_non_linux(monkeypatch: pytest.MonkeyPatch) -> None:
    """The Linux-only contract fails before a fixture can reserve or spawn."""
    monkeypatch.setattr(ports.platform, "system", lambda: "Darwin")
    with pytest.raises(RuntimeError, match="requires Linux with readable /proc/net/tcp"):
        require_linux_ipv4_listener_support()


def test_linux_ipv4_listener_support_requires_proc_tcp(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A missing IPv4 listener table is rejected rather than causing a late timeout."""
    monkeypatch.setattr(ports.platform, "system", lambda: "Linux")
    monkeypatch.setattr(ports.os.path, "exists", lambda _path: False)
    with pytest.raises(RuntimeError, match="requires Linux with readable /proc/net/tcp"):
        require_linux_ipv4_listener_support()


def test_dynamic_reservations_are_distinct_and_released() -> None:
    """A single attempt cannot select the same automatic port twice."""
    first = reserve_loopback_port("FIRST", {})
    second = reserve_loopback_port("SECOND", {})
    try:
        assert first.port != second.port
        assert first.configured is False
        assert second.configured is False
    finally:
        release_reservations((first, second))


def test_explicit_occupied_port_fails_before_a_child_can_start() -> None:
    """An operator-provided port collision is reported without a readiness probe."""
    blocker = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    blocker.bind(("127.0.0.1", 0))
    try:
        port = blocker.getsockname()[1]
        with pytest.raises(RuntimeError, match=rf"FIXTURE_PORT={port} is unavailable"):
            reserve_loopback_port("FIXTURE_PORT", {"FIXTURE_PORT": str(port)})
    finally:
        blocker.close()


def test_listener_ownership_is_limited_to_the_child_process_group() -> None:
    """A pre-existing listener cannot be mistaken for a child owned by another group."""
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    unrelated = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(10)"],
        start_new_session=True,
    )
    try:
        port = listener.getsockname()[1]
        assert listener_owner_pids(port)
        assert listener_is_owned_by_process_group(port, os.getpid()) is True
        assert listener_is_owned_by_process_group(port, unrelated.pid) is False
    finally:
        unrelated.terminate()
        unrelated.wait()
        listener.close()


@pytest.mark.parametrize(
    ("error", "expected"),
    [
        (OSError(errno.EADDRINUSE, "Address already in use"), True),
        ("Error: listen EADDRINUSE: address already in use", True),
        ("connection refused", False),
    ],
)
def test_address_in_use_classification(error: BaseException | str, expected: bool) -> None:
    """Only an address-in-use signal permits the bounded dynamic retry."""
    assert is_address_in_use(error) is expected


def test_launcher_rejects_an_explicit_occupied_port_without_connecting(
    tmp_path: Path,
) -> None:
    """Preflight removes stale manifests and never probes a foreign listener."""
    root = tmp_path / "stack"
    root.mkdir()
    (root / "ready.json").write_text('{"stale":true}', encoding="utf-8")
    (root / "stopped.json").write_text('{"stale":true}', encoding="utf-8")
    foreign_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    foreign_listener.bind(("127.0.0.1", 0))
    foreign_listener.listen()
    try:
        port = foreign_listener.getsockname()[1]
        result = subprocess.run(
            [sys.executable, str(STACK_SCRIPT)],
            env=launcher_environment(
                root,
                NAMESPACED_PROVIDER_ENGINE_PORT=str(port),
            ),
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode != 0
        assert f"NAMESPACED_PROVIDER_ENGINE_PORT={port} is unavailable" in result.stderr
        assert not (root / "ready.json").exists()
        assert json.loads((root / "stopped.json").read_text(encoding="utf-8")) == {}
        foreign_listener.settimeout(0.1)
        with pytest.raises(socket.timeout):
            foreign_listener.accept()
    finally:
        foreign_listener.close()


@pytest.mark.parametrize("attempt", [-1, 2])
def test_launcher_rejects_invalid_retry_attempt_before_spawning(
    tmp_path: Path,
    attempt: int,
) -> None:
    """An inherited invalid retry counter cannot create unbounded retry cycles."""
    marker = tmp_path / "spawned"
    binary = tmp_path / "must-not-run.py"
    binary.write_text(
        "#!/usr/bin/env python3\n"
        f"open({str(marker)!r}, 'w').write('spawned')\n",
        encoding="utf-8",
    )
    binary.chmod(0o755)
    root = tmp_path / "stack"
    root.mkdir()
    (root / "ready.json").write_text('{"stale":true}', encoding="utf-8")
    result = subprocess.run(
        [sys.executable, str(STACK_SCRIPT)],
        env=launcher_environment(
            root,
            III_BIN=str(binary),
            NAMESPACED_PROVIDER_PORT_RETRY_ATTEMPT=str(attempt),
        ),
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0
    assert "NAMESPACED_PROVIDER_PORT_RETRY_ATTEMPT must be an integer from 0 to 1" in result.stderr
    assert "retrying fixture" not in result.stderr
    assert not marker.exists()
    assert not (root / "ready.json").exists()
    assert json.loads((root / "stopped.json").read_text(encoding="utf-8")) == {}


def test_launcher_missing_required_env_replaces_stale_manifests(tmp_path: Path) -> None:
    """Missing binaries fail preflight without leaving an old ready manifest alive."""
    root = tmp_path / "stack"
    root.mkdir()
    (root / "ready.json").write_text('{"stale":true}', encoding="utf-8")
    (root / "stopped.json").write_text('{"stale":true}', encoding="utf-8")
    environment = launcher_environment(root)
    environment.pop("ROUTER_BIN")
    result = subprocess.run(
        [sys.executable, str(STACK_SCRIPT)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0
    assert "ROUTER_BIN is required" in result.stderr
    assert not (root / "ready.json").exists()
    assert json.loads((root / "stopped.json").read_text(encoding="utf-8")) == {}


def test_launcher_platform_preflight_replaces_stale_manifests(tmp_path: Path) -> None:
    """A non-Linux preflight clears old readiness without spawning children."""
    root = tmp_path / "stack"
    root.mkdir()
    (root / "ready.json").write_text('{"stale":true}', encoding="utf-8")
    (root / "stopped.json").write_text('{"stale":true}', encoding="utf-8")
    patch_platform = (
        "import platform, runpy, sys; "
        "sys.path.insert(0, sys.argv[1]); "
        "platform.system = lambda: 'Darwin'; "
        "runpy.run_path(sys.argv[2], run_name='__main__')"
    )
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            patch_platform,
            str(STACK_SCRIPT.parent),
            str(STACK_SCRIPT),
        ],
        env=launcher_environment(root),
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0
    assert "requires Linux with readable /proc/net/tcp" in result.stderr
    assert not (root / "ready.json").exists()
    assert json.loads((root / "stopped.json").read_text(encoding="utf-8")) == {}


def test_launcher_retries_one_dynamic_foreign_listener_without_connecting(
    tmp_path: Path,
) -> None:
    """A foreign owner after release retries once and is never probed."""
    root = tmp_path / "stack"
    probe_dir = tmp_path / "probe"
    probe_dir.mkdir()
    foreign_engine = tmp_path / "foreign-engine.py"
    write_foreign_engine(foreign_engine)
    try:
        result = subprocess.run(
            [sys.executable, str(STACK_SCRIPT)],
            env=launcher_environment(
                root,
                III_BIN=str(foreign_engine),
                PROBE_DIR=str(probe_dir),
            ),
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode != 0
        assert result.stderr.count("retrying fixture after confirmed dynamic port collision") == 1
        assert "dynamic port collision for NAMESPACED_PROVIDER_ENGINE_PORT persisted after 2 attempts" in result.stderr
        assert not (probe_dir / "accepts").exists()
        assert len((probe_dir / "pids").read_text(encoding="utf-8").splitlines()) == 2
        assert (root / "stopped.json").is_file()
    finally:
        terminate_foreign_children(probe_dir)


def test_launcher_rejects_explicit_foreign_listener_without_retry(
    tmp_path: Path,
) -> None:
    """An explicit port collision after release fails without a readiness probe."""
    root = tmp_path / "stack"
    probe_dir = tmp_path / "probe"
    probe_dir.mkdir()
    foreign_engine = tmp_path / "foreign-engine.py"
    write_foreign_engine(foreign_engine)
    try:
        result = subprocess.run(
            [sys.executable, str(STACK_SCRIPT)],
            env=launcher_environment(
                root,
                III_BIN=str(foreign_engine),
                PROBE_DIR=str(probe_dir),
                NAMESPACED_PROVIDER_ENGINE_PORT=str(pick_unused_port()),
            ),
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode != 0
        assert "retrying fixture" not in result.stderr
        assert "NAMESPACED_PROVIDER_ENGINE_PORT=" in result.stderr
        assert "collided after launch; refusing to probe another process" in result.stderr
        assert not (probe_dir / "accepts").exists()
        assert len((probe_dir / "pids").read_text(encoding="utf-8").splitlines()) == 1
        assert (root / "stopped.json").is_file()
    finally:
        terminate_foreign_children(probe_dir)




def test_launcher_removes_stale_manifests_during_startup(tmp_path: Path) -> None:
    """Startup removes prior manifests before waiting for a non-listening engine."""
    root = tmp_path / "stack"
    root.mkdir()
    checkpoint = tmp_path / "engine-started"
    for filename in ("ready.json", "fixture-ready.json", "stopped.json"):
        (root / filename).write_text('{"stale":true}', encoding="utf-8")

    engine = tmp_path / "non-listening-engine.py"
    engine.write_text(
        "#!/usr/bin/env python3\n"
        "import os, sys, time\n"
        "from pathlib import Path\n"
        "Path(os.environ['ENGINE_STARTED_FILE']).write_text(str(os.getpid()), encoding='utf-8')\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    engine.chmod(0o755)
    launcher = subprocess.Popen(
        [sys.executable, str(STACK_SCRIPT)],
        env=launcher_environment(
            root,
            III_BIN=str(engine),
            ENGINE_STARTED_FILE=str(checkpoint),
        ),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    engine_pid: int | None = None
    try:
        deadline = time.monotonic() + 5
        while not checkpoint.is_file():
            assert launcher.poll() is None, launcher.stderr.read()
            if time.monotonic() >= deadline:
                raise AssertionError("engine checkpoint was not written during launcher startup")
            time.sleep(0.02)
        engine_pid = int(checkpoint.read_text(encoding="utf-8"))
        assert launcher.poll() is None
        assert not (root / "ready.json").exists()
        assert not (root / "fixture-ready.json").exists()
        assert not (root / "stopped.json").exists()

        launcher.send_signal(signal.SIGTERM)
        assert launcher.wait(timeout=5) == 0
        assert not (root / "ready.json").exists()
        stopped = json.loads((root / "stopped.json").read_text(encoding="utf-8"))
        assert stopped == {"engine": -signal.SIGTERM}
    finally:
        if launcher.poll() is None:
            launcher.send_signal(signal.SIGTERM)
            try:
                launcher.wait(timeout=5)
            except subprocess.TimeoutExpired:
                launcher.kill()
                launcher.wait(timeout=5)
        if engine_pid is not None:
            try:
                os.kill(engine_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        if launcher.stdout is not None:
            launcher.stdout.close()
        if launcher.stderr is not None:
            launcher.stderr.close()
def test_launcher_propagates_child_startup_failure(tmp_path: Path) -> None:
    """A failed listener child is reported rather than treated as readiness delay."""
    failed_engine = tmp_path / "failed-engine.py"
    failed_engine.write_text(
        "#!/usr/bin/env python3\n"
        "import sys\n"
        "print('deliberate engine startup failure', flush=True)\n"
        "sys.exit(23)\n",
        encoding="utf-8",
    )
    failed_engine.chmod(0o755)
    root = tmp_path / "stack"
    result = subprocess.run(
        [sys.executable, str(STACK_SCRIPT)],
        env=launcher_environment(root, III_BIN=str(failed_engine)),
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0
    assert "engine exited before readiness: deliberate engine startup failure" in result.stderr
    assert (root / "stopped.json").is_file()
