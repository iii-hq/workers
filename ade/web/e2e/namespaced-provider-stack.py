#!/usr/bin/env python3
"""Linux IPv4 namespaced provider stack for CI fixtures.

This fixture requires Linux with readable `/proc/net/tcp` and IPv4 TCP
listeners reachable through loopback. It deliberately does not support macOS,
other non-Linux hosts, or IPv6/dual-stack listeners because readiness verifies
listener owners through Linux's IPv4 TCP table before connecting.

Required environment:
  III_BIN
  STATE_BIN
  ROUTER_BIN
  QUEUE_BIN
  HARNESS_BIN
  CONSOLE_BIN
Optional environment:
  NAMESPACED_PROVIDER_FIXTURE_ROOT
  NAMESPACED_PROVIDER_ENGINE_PORT
  NAMESPACED_PROVIDER_STREAM_PORT
  NAMESPACED_PROVIDER_CONSOLE_PORT
  NAMESPACED_PROVIDER_SPA_PORT
  NAMESPACED_PROVIDER_CONFIGURATION_ID
  NAMESPACED_PROVIDER_PROVIDER_ID
  READY_FILE
  PROVIDER_FIXTURE_CMD
  NODE_BIN
  VITE_BIN
  WEB_DIR

The launcher owns every child process and writes READY_FILE only after the
engine, state, router, harness, provider fixture, and current Console SPA are reachable.

"""
from __future__ import annotations

import json
import os
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import urllib.request
import time
from pathlib import Path

from namespaced_provider_ports import (
    LoopbackPortReservation,
    is_address_in_use,
    listener_is_owned_by_process_group,
    listener_owner_pids,
    release_reservations,
    require_linux_ipv4_listener_support,
    reserve_loopback_port,
)

HERE = Path(__file__).resolve().parent
ROOT = Path(os.environ.get("NAMESPACED_PROVIDER_FIXTURE_ROOT", tempfile.mkdtemp(prefix="namespaced-provider-"))).resolve()
ROOT.mkdir(parents=True, exist_ok=True)
ready_file = Path(os.environ.get("READY_FILE", str(ROOT / "ready.json"))).resolve()
stopped_file = ROOT / "stopped.json"


def reset_manifests() -> None:
    """Remove manifests left by any prior attempt before preflight can fail."""
    for stale in (ready_file, ROOT / "fixture-ready.json", stopped_file):
        stale.unlink(missing_ok=True)


def write_stopped_manifest(
    entries: list[tuple[str, subprocess.Popen[bytes]]],
) -> None:
    """Record the current attempt's child exits, including an empty preflight."""
    stopped_file.write_text(
        json.dumps({name: child.returncode for name, child in entries}, indent=2),
        encoding="utf-8",
    )


reset_manifests()
for name in ("config", "data", "logs"):
    (ROOT / name).mkdir(exist_ok=True)


def required(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"{name} is required")
    return value


PORT_RETRY_ATTEMPT_ENV = "NAMESPACED_PROVIDER_PORT_RETRY_ATTEMPT"
MAX_DYNAMIC_PORT_RETRIES = 1


class PortCollision(RuntimeError):
    """A child could not own a port selected for this fixture attempt."""

    def __init__(self, reservation: LoopbackPortReservation, detail: str) -> None:
        super().__init__(detail)
        self.reservation = reservation


def retry_attempt() -> int:
    """Read the bounded dynamic-port retry counter from this launcher process."""
    raw_value = os.environ.get(PORT_RETRY_ATTEMPT_ENV, "0")
    try:
        attempt = int(raw_value)
    except ValueError as error:
        raise RuntimeError(
            f"{PORT_RETRY_ATTEMPT_ENV} must be an integer from 0 to {MAX_DYNAMIC_PORT_RETRIES}"
        ) from error
    if not 0 <= attempt <= MAX_DYNAMIC_PORT_RETRIES:
        raise RuntimeError(
            f"{PORT_RETRY_ATTEMPT_ENV} must be an integer from 0 to {MAX_DYNAMIC_PORT_RETRIES}"
        )
    return attempt


reservations: dict[str, LoopbackPortReservation] = {}
try:
    require_linux_ipv4_listener_support()
    retry_attempt()
    engine_binary = required("III_BIN")
    state_bin = required("STATE_BIN")
    router_binary = required("ROUTER_BIN")
    queue_bin = required("QUEUE_BIN")
    harness_bin = required("HARNESS_BIN")
    console_binary = required("CONSOLE_BIN")
    for reservation_name in ("ENGINE", "STREAM", "CONSOLE", "SPA"):
        environment_name = f"NAMESPACED_PROVIDER_{reservation_name}_PORT"
        reservations[reservation_name.lower()] = reserve_loopback_port(
            environment_name,
            os.environ,
        )
except BaseException:
    release_reservations(tuple(reservations.values()))
    ready_file.unlink(missing_ok=True)
    write_stopped_manifest([])
    raise
all_reservations = tuple(reservations.values())
engine_port = reservations["engine"].port
stream_port = reservations["stream"].port
console_port = reservations["console"].port
spa_port = reservations["spa"].port
engine_url = f"ws://127.0.0.1:{engine_port}"
stream_url = f"ws://127.0.0.1:{stream_port}"
backend_url = f"http://127.0.0.1:{console_port}"
console_url = f"http://127.0.0.1:{spa_port}"
web_dir = Path(os.environ.get("WEB_DIR", str(HERE.parent))).resolve()
vite_bin = os.environ.get("VITE_BIN", str(web_dir / "node_modules" / ".bin" / "vite"))
configuration_id = os.environ.get("NAMESPACED_PROVIDER_CONFIGURATION_ID", "default-llm-router")
provider_id = os.environ.get("NAMESPACED_PROVIDER_PROVIDER_ID", "openai-codex")

(ROOT / "config" / "default-state.yaml").write_text(
    "\n".join(
        [
            "id: default-state",
            "name: State",
            "description: namespaced state fixture",
            "metadata:",
            "  ui_form: state",
            "value:",
            "  adapter:",
            "    config:",
            "      store_method: in_memory",
            "    name: kv",
            "",
        ]
    ),
    encoding="utf-8",
)
config_path = ROOT / "config" / f"{configuration_id}.yaml"
(ROOT / "config" / "iii-stream.yaml").write_text(
    "\n".join(
        [
            "id: iii-stream",
            "name: Stream",
            "description: namespaced stream fixture",
            "value:",
            "  auth_function: null",
            "  host: 127.0.0.1",
            f"  port: {stream_port}",
            "",
        ]
    ),
    encoding="utf-8",
)
(ROOT / "config" / "default-ade.yaml").write_text(
    "\n".join(
        [
            "id: default-ade",
            "name: ADE fixture backend",
            "description: namespaced ADE backend fixture",
            "metadata:",
            "  ui_form: console",
            "value:",
            f"  http_port: {console_port}",
            "  data_dir: data/ade",
            "",
        ]
    ),
    encoding="utf-8",
)
config_path.write_text(
    "\n".join(
        [
            f"id: {configuration_id}",
            "name: namespaced provider router",
            "description: deterministic provider configuration fixture",
            "metadata:",
            "  ui_form: llm-router",
            "value:",
            "  providers:",
            f"    {provider_id}:",
            "      api_url: https://provider-fixture.example",
            "      max_tokens: 42",
            "",
        ]
    ),
    encoding="utf-8",
)
state_seed = ROOT / "state-seed.json"
state_seed.write_text(
    json.dumps({"adapter": {"name": "kv", "config": {"store_method": "in_memory"}}}),
    encoding="utf-8",
)
engine_config = ROOT / "engine.json"
engine_config.write_text(
    json.dumps(
        {
            "workers": [
                {
                    "name": "iii-worker-manager",
                    "config": {"host": "127.0.0.1", "port": engine_port},
                },
                {
                    "name": "configuration",
                    "config": {
                        "adapter": {
                            "name": "fs",
                            "config": {"directory": str(ROOT / "config")},
                        }
                    },
                },
                {
                    "name": "iii-stream",
                    "config": {"host": "127.0.0.1", "port": stream_port},
                },
            ]
        },
        indent=2,
    ),
    encoding="utf-8",
)

children: list[tuple[str, subprocess.Popen[bytes]]] = []
blocked_env = {
    "III_ENGINE_URL",
    "III_CONSOLE_URL",
    "CONSOLE_E2E_URL",
    "CONSOLE_E2E_READY_FILE",
    "III_CONFIG_NAME",
    "VITE_ENGINE_WS_URL",
}
env = {name: value for name, value in os.environ.items() if name not in blocked_env}
env.update(
    {
        "HOME": str(ROOT / "data"),
        "III_TELEMETRY_ENABLED": "false",
        "III_URL": engine_url,
        "III_ENGINE_URL": engine_url,
        "NAMESPACED_PROVIDER_FIXTURE_ROOT": str(ROOT),
        "NAMESPACED_PROVIDER_PROVIDER_ID": provider_id,
        "NAMESPACED_PROVIDER_CONFIGURATION_ID": configuration_id,
    }
)


def spawn(
    name: str,
    command: list[str],
    extra_env: dict[str, str] | None = None,
    cwd: Path | None = None,
    release: tuple[LoopbackPortReservation, ...] = (),
) -> subprocess.Popen[bytes]:
    """Start one owned child after releasing only its listener reservations.

    The process group isolates cleanup and is also the authority checked by
    readiness probes. Callers release a reservation at the latest supported
    handoff point; this fixture does not pass inherited listening sockets.
    """
    release_reservations(release)
    log = (ROOT / "logs" / f"{name}.log").open("wb")
    child = subprocess.Popen(
        command,
        cwd=cwd or ROOT,
        env={**env, **(extra_env or {})},
        stdout=log,
        stderr=subprocess.STDOUT,
        # Each child owns an isolated process group. Cleanup targets only these
        # groups, never another task's processes that happen to use a port.
        start_new_session=True,
    )
    children.append((name, child))
    return child


def child_log_tail(name: str) -> str:
    """Return enough child output to classify a failed listener startup."""
    path = ROOT / "logs" / f"{name}.log"
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-4000:]
    except OSError:
        return "child log unavailable"


def wait_tcp(
    url: str,
    name: str,
    child: subprocess.Popen[bytes],
    reservation: LoopbackPortReservation,
    timeout: float = 60.0,
) -> None:
    """Wait for a TCP listener owned by `child`, never an ambient service."""
    host_port = url.split("://", 1)[-1].split("/", 1)[0]
    host, raw_port = host_port.rsplit(":", 1)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        owners = listener_owner_pids(reservation.port)
        if owners and not listener_is_owned_by_process_group(reservation.port, child.pid):
            raise PortCollision(
                reservation,
                f"{name} does not own listening port {reservation.port}; owners={sorted(owners)}",
            )
        if child.poll() is not None:
            detail = child_log_tail(name)
            if is_address_in_use(detail):
                raise PortCollision(
                    reservation,
                    f"{name} exited after losing port {reservation.port}: {detail}",
                )
            raise RuntimeError(f"{name} exited before readiness: {detail}")
        if listener_is_owned_by_process_group(reservation.port, child.pid):
            try:
                with socket.create_connection((host, int(raw_port)), timeout=0.3):
                    return
            except OSError:
                pass
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {name} to own {url}")


def wait_file(path: Path, timeout: float = 60.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.is_file() and path.stat().st_size > 0:
            return
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {path}")


def wait_http(
    url: str,
    name: str,
    child: subprocess.Popen[bytes],
    reservation: LoopbackPortReservation,
    timeout: float = 60.0,
) -> None:
    """Wait for HTML only after each request confirms child listener ownership."""
    deadline = time.monotonic() + timeout
    last_error = "unknown error"
    while time.monotonic() < deadline:
        try:
            wait_tcp(url, name, child, reservation, timeout=0.3)
        except PortCollision:
            raise
        except RuntimeError as error:
            last_error = str(error)
            if not last_error.startswith("timed out waiting"):
                raise
            time.sleep(0.2)
            continue
        try:
            with urllib.request.urlopen(url, timeout=1.0) as response:
                if response.status == 200:
                    body = response.read(4096).decode("utf-8", errors="ignore")
                    if "<html" in body.lower() or "<!doctype" in body.lower():
                        return
                    last_error = f"{url} returned non-HTML content"
                else:
                    last_error = f"{url} returned HTTP {response.status}"
        except Exception as error:
            last_error = str(error)
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for HTTP {url}: {last_error}")


def wait_contract(timeout: float = 60.0) -> None:
    probe = r'''
import { registerWorker } from 'iii-browser-sdk'
const sdk = registerWorker(process.env.III_ENGINE_URL, { metadata: { name: 'provider-configuration-readiness-probe' } })
const id = process.env.NAMESPACED_PROVIDER_CONFIGURATION_ID
const provider = process.env.NAMESPACED_PROVIDER_PROVIDER_ID
try {
  const workersResponse = await sdk.trigger({ function_id: 'engine::workers::list', payload: {} })
  const workers = Array.isArray(workersResponse) ? workersResponse : workersResponse.workers ?? []
  if (!workers.some((worker) => worker.name === 'harness')) throw new Error('harness worker missing')
  const listed = await sdk.trigger({ function_id: 'configuration::list', payload: {} })
  const entries = Array.isArray(listed) ? listed : listed.configurations ?? []
  if (!entries.some((entry) => entry.id === id)) throw new Error(`configuration ${id} missing`)
  const raw = await sdk.trigger({ function_id: 'configuration::get', payload: { id, raw: true } })
  if (raw?.id !== id) throw new Error(`configuration::get returned ${raw?.id ?? 'no id'}`)
  const providersResponse = await sdk.trigger({ function_id: 'router::provider::list', payload: {} })
  const providers = Array.isArray(providersResponse) ? providersResponse : providersResponse.providers ?? []
  if (!providers.some((entry) => entry.id === provider)) throw new Error(`provider ${provider} missing`)
  const modelsResponse = await sdk.trigger({ function_id: 'router::models::list', payload: {} })
  const models = Array.isArray(modelsResponse) ? modelsResponse : modelsResponse.models ?? []
  if (!models.some((model) => model.provider === provider)) throw new Error(`model for ${provider} missing`)
  await sdk.trigger({
    function_id: 'state::set',
    payload: {
      scope: 'provider-configuration-readiness',
      key: `probe-${process.pid}-${Date.now()}`,
      value: { ok: true, harness: true, provider, models: models.length },
    },
  })
} finally {
  await sdk.shutdown()
}
'''
    deadline = time.monotonic() + timeout
    last_error = "unknown error"
    while time.monotonic() < deadline:
        try:
            result = subprocess.run(
                [node_bin, "--input-type=module", "-e", probe],
                cwd=web_dir,
                env=env,
                capture_output=True,
                text=True,
                timeout=10,
            )
            if result.returncode == 0:
                return
            last_error = (result.stderr or result.stdout or "probe failed")[-2000:]
        except Exception as error:
            last_error = str(error)
        time.sleep(0.5)
    raise RuntimeError(f"contract readiness probe failed: {last_error}")

stopped = False


def stop(*_args: object) -> None:
    """Terminate only this fixture's process groups and release reservations."""
    global stopped
    if stopped:
        return
    stopped = True
    release_reservations(all_reservations)
    try:
        for _, child in reversed(children):
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        deadline = time.monotonic() + 8
        for _, child in reversed(children):
            remaining = max(0.0, deadline - time.monotonic())
            try:
                child.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
    finally:
        ready_file.unlink(missing_ok=True)
        write_stopped_manifest(children)


def exit_on_signal(*_args: object) -> None:
    stop()
    raise SystemExit(0)


signal.signal(signal.SIGTERM, exit_on_signal)
signal.signal(signal.SIGINT, exit_on_signal)
try:
    engine = spawn(
        "engine",
        [engine_binary, "--no-update-check", "--config", str(engine_config)],
        release=(reservations["engine"], reservations["stream"]),
    )
    wait_tcp(engine_url, "engine", engine, reservations["engine"])
    wait_tcp(stream_url, "stream", engine, reservations["stream"])
    spawn("state", [state_bin, "--url", engine_url, "--config", str(state_seed)])
    spawn(
        "router",
        [router_binary, "--url", engine_url],
        {"III_CONFIG_NAME": configuration_id},
    )
    wait_tcp(engine_url, "engine", engine, reservations["engine"])
    node_bin = os.environ.get("NODE_BIN", "node")
    fixture_command = os.environ.get(
        "PROVIDER_FIXTURE_CMD",
        f"{shlex.quote(node_bin)} {shlex.quote(str(HERE / 'provider-fixture.mjs'))}",
    )
    spawn(
        "providers",
        shlex.split(fixture_command),
        {"III_ENGINE_URL": engine_url},
        cwd=web_dir,
    )
    console_child = spawn(
        "console",
        [console_binary, "--url", engine_url, "--http-port", str(console_port)],
        {"III_CONFIG_NAME": "default-ade"},
        release=(reservations["console"],),
    )
    wait_http(backend_url, "console", console_child, reservations["console"])
    spawn("queue", [queue_bin, "--url", engine_url])
    spawn("harness", [harness_bin, "--url", engine_url])
    spa_child = spawn(
        "spa",
        [
            vite_bin,
            "--host",
            "127.0.0.1",
            "--port",
            str(spa_port),
            "--strictPort",
        ],
        {
            "III_ENGINE_URL": engine_url,
            "III_CONSOLE_URL": backend_url,
            "VITE_ENGINE_WS_URL": engine_url,
            "VITE_CJS_IGNORE_WARNING": "true",
        },
        cwd=web_dir,
        release=(reservations["spa"],),
    )
    # The current worktree SPA, not a prebuilt Console asset, is the URL used by Playwright.
    wait_file(ROOT / "fixture-ready.json")
    # The current worktree SPA, not a prebuilt Console asset, is the URL used by Playwright.
    wait_http(console_url, "spa", spa_child, reservations["spa"])
    wait_contract()
    ready_file.parent.mkdir(parents=True, exist_ok=True)
    ready_file.write_text(
        json.dumps(
            {
                "schema_version": "1",
                "engine_url": engine_url,
                "console_url": console_url,
                "console_backend_url": backend_url,
                "stream_url": f"ws://127.0.0.1:{stream_port}",
                "configuration_id": configuration_id,
                "provider_id": provider_id,
                "root": str(ROOT),
                "ready_file": str(ready_file),
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    print(json.dumps({"ready_file": str(ready_file), "engine_url": engine_url, "console_url": console_url}), flush=True)
    while True:
        dead = [(name, child.returncode) for name, child in children if child.poll() is not None]
        if dead:
            raise RuntimeError(f"stack process exited: {dead}")
        time.sleep(1)
except PortCollision as error:
    reservation = error.reservation
    if reservation.configured:
        raise RuntimeError(
            f"{reservation.name}={reservation.port} collided after launch; refusing to probe another process"
        ) from error
    attempt = retry_attempt()
    if attempt >= MAX_DYNAMIC_PORT_RETRIES:
        raise RuntimeError(
            f"dynamic port collision for {reservation.name} persisted after {attempt + 1} attempts"
        ) from error
    print(
        f"retrying fixture after confirmed dynamic port collision for {reservation.name}",
        file=sys.stderr,
        flush=True,
    )
    stop()
    retry_env = os.environ.copy()
    retry_env[PORT_RETRY_ATTEMPT_ENV] = str(attempt + 1)
    os.execvpe(sys.executable, [sys.executable, *sys.argv], retry_env)
finally:
    stop()
