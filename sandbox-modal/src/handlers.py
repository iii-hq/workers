"""Handler coroutines for every `sandbox::modal::*` function.

Each coroutine parses untyped JSON, does config-side validation
(allowlist, concurrency cap), then delegates to `ModalClient`. Failures
surface as `SandboxError` whose stringification carries the leading
`[Sxxx]` so callers can pattern-match on the S-code.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any

from .config import Config
from .modal_client import ModalClient
from .sandbox import (
    CAPABILITIES,
    S_CONCURRENCY_CAP,
    S_IMAGE_NOT_ALLOWED,
    S_PROVIDER_UNAVAILABLE,
    SandboxError,
)


@dataclass
class HandlerCtx:
    config: Config
    client: ModalClient
    in_flight: int = 0
    lock: asyncio.Lock | None = None

    def __post_init__(self) -> None:
        if self.lock is None:
            self.lock = asyncio.Lock()


def _require_str(payload: dict[str, Any], key: str) -> str:
    val = payload.get(key)
    if not isinstance(val, str) or not val:
        raise SandboxError(S_PROVIDER_UNAVAILABLE, f'missing string field "{key}"')
    return val


async def do_create(ctx: HandlerCtx, payload: dict[str, Any]) -> dict[str, Any]:
    image = _require_str(payload, "image")
    if not ctx.config.image_allowed(image):
        raise SandboxError(S_IMAGE_NOT_ALLOWED, f"image not in allowlist: {image}")

    cpus = int(payload.get("cpus") or ctx.config.default_cpus)
    memory_mb = int(payload.get("memory_mb") or ctx.config.default_memory_mb)
    idle = int(payload.get("idle_timeout_secs") or ctx.config.default_idle_timeout_secs)

    assert ctx.lock is not None
    async with ctx.lock:
        if ctx.in_flight >= ctx.config.max_concurrent_sandboxes:
            raise SandboxError(S_CONCURRENCY_CAP, f"concurrency cap reached ({ctx.in_flight} active)")
        ctx.in_flight += 1

    try:
        created = await ctx.client.create(image, cpus, memory_mb, idle)
        return {
            "sandbox_id": created.sandbox_id,
            "image": created.image,
            "started_at": created.started_at,
            "capabilities": list(CAPABILITIES),
        }
    except BaseException:
        async with ctx.lock:
            ctx.in_flight = max(0, ctx.in_flight - 1)
        raise


async def do_exec(ctx: HandlerCtx, payload: dict[str, Any]) -> dict[str, Any]:
    sandbox_id = _require_str(payload, "sandbox_id")
    cmd = _require_str(payload, "cmd")
    args = list(payload.get("args") or [])
    timeout_ms = payload.get("timeout_ms")
    result = await ctx.client.exec(sandbox_id, cmd, [str(a) for a in args], timeout_ms)
    return {
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
    }


async def do_stop(ctx: HandlerCtx, payload: dict[str, Any]) -> dict[str, Any]:
    sandbox_id = _require_str(payload, "sandbox_id")
    await ctx.client.stop(sandbox_id)
    assert ctx.lock is not None
    async with ctx.lock:
        ctx.in_flight = max(0, ctx.in_flight - 1)
    return {}


async def do_list(ctx: HandlerCtx, _payload: dict[str, Any]) -> dict[str, Any]:
    # Best-effort upstream fetch. Local capacity envelope always returned.
    sandboxes: list[dict[str, object]] = []
    try:
        sandboxes = await ctx.client.list()
    except SandboxError:
        sandboxes = []
    cap = ctx.config.max_concurrent_sandboxes
    return {
        "sandboxes": sandboxes,
        "in_flight": ctx.in_flight,
        "cap": cap,
        "remaining": max(cap - ctx.in_flight, 0),
    }


async def do_snapshot(ctx: HandlerCtx, payload: dict[str, Any]) -> dict[str, Any]:
    sandbox_id = _require_str(payload, "sandbox_id")
    snapshot_id = await ctx.client.snapshot(sandbox_id)
    return {"snapshot_id": snapshot_id}


async def do_expose_port(ctx: HandlerCtx, payload: dict[str, Any]) -> dict[str, Any]:
    sandbox_id = _require_str(payload, "sandbox_id")
    port = payload.get("port")
    if not isinstance(port, int) or port < 1 or port > 65535:
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "invalid port")
    url = await ctx.client.expose_port(sandbox_id, port)
    return {"url": url}
