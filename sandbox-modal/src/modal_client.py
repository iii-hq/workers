"""Narrow wrapper around the Modal Python SDK.

Modal is gRPC-only; the `modal` package is the only supported transport,
so this worker imports it inside method bodies (lazy-loaded). The handlers
talk to this wrapper, never to `modal` directly. Tests mock this class.

v0 ships stub bodies that raise `SandboxError(S502, "TODO ...")` — the
ABI is stable but the SDK calls land in a follow-up commit.
"""

from __future__ import annotations

from .sandbox import (
    S_PROVIDER_UNAVAILABLE,
    CreatedSandbox,
    ExecResult,
    SandboxError,
)


class ModalClient:
    def __init__(self, app_name: str = "sandbox-modal") -> None:
        self.app_name = app_name

    async def create(
        self,
        image: str,
        cpus: int,
        memory_mb: int,
        idle_timeout_secs: int,
    ) -> CreatedSandbox:
        # TODO: import modal lazily, build modal.Image, attach to a long-lived
        # modal.App, call modal.Sandbox.create(...). Returns sandbox_id from
        # sandbox.object_id.
        _ = (image, cpus, memory_mb, idle_timeout_secs)
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal.Sandbox.create")

    async def exec(
        self,
        sandbox_id: str,
        cmd: str,
        args: list[str],
        timeout_ms: int | None,
    ) -> ExecResult:
        _ = (sandbox_id, cmd, args, timeout_ms)
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal sandbox.exec")

    async def stop(self, sandbox_id: str) -> None:
        _ = sandbox_id
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal sandbox.terminate")

    async def list(self) -> list[dict[str, object]]:
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal sandbox listing")

    async def snapshot(self, sandbox_id: str) -> str:
        _ = sandbox_id
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal snapshot_filesystem")

    async def expose_port(self, sandbox_id: str, port: int) -> str:
        _ = (sandbox_id, port)
        raise SandboxError(S_PROVIDER_UNAVAILABLE, "TODO: wire modal Tunnel")
