"""Narrow wrapper around the Modal Python SDK.

Modal is gRPC-only; the official `modal` package is the only supported
transport. The handlers talk to this wrapper, never to `modal` directly,
so tests can mock the boundary without forcing real Modal credentials at
import time.

The SDK call sequence (verified via context7 against the modal docs and
modal.com/docs/reference/modal.Sandbox) for each method is recorded in
its docstring. Bodies are wired against those signatures and tracked
behind a small in-process registry that maps our `sandbox_id` (which is
also Modal's `object_id`) back to the live `modal.Sandbox` handle.

Lazy import: `modal` is imported inside each method so loading the
worker module without Modal credentials still succeeds (handlers can
short-circuit on validation errors before any Modal call). When tests
mock this class, the modal import never runs.
"""

from __future__ import annotations

import asyncio
from typing import Any

from .sandbox import (
    S_AUTH_INVALID,
    S_PROVIDER_UNAVAILABLE,
    CreatedSandbox,
    ExecResult,
    SandboxError,
    map_modal_error,
)


def _import_modal() -> Any:
    try:
        import modal  # type: ignore[import-untyped]

        return modal
    except ImportError as exc:
        raise SandboxError(S_AUTH_INVALID, f"modal sdk not installed: {exc}") from exc


class ModalClient:
    """Adapter on top of `modal.Sandbox`.

    `app` is created lazily on first `create` and reused thereafter so a
    single Modal App backs every sandbox the worker spawns. The
    in-process `_handles` map is what makes `exec`/`stop`/`snapshot`
    look up the right Sandbox object from a sandbox_id passed over the
    iii trigger boundary.
    """

    def __init__(self, app_name: str = "sandbox-modal") -> None:
        self.app_name = app_name
        self._app: Any | None = None
        self._handles: dict[str, Any] = {}
        self._lock = asyncio.Lock()

    def _ensure_app(self) -> Any:
        if self._app is None:
            modal = _import_modal()
            self._app = modal.App.lookup(self.app_name, create_if_missing=True)
        return self._app

    async def create(
        self,
        image: str,
        cpus: int,
        memory_mb: int,
        idle_timeout_secs: int,
    ) -> CreatedSandbox:
        """`modal.Sandbox.create(app=, image=, timeout=, cpu=, memory=)`.

        `image` is parsed as either a registered Modal image name or a
        plain Debian rootfs ref. `timeout` is in seconds upstream and
        maps directly from `idle_timeout_secs`.
        """

        def blocking_create() -> Any:
            modal = _import_modal()
            app = self._ensure_app()
            modal_image = modal.Image.debian_slim() if image in {"", "default"} else modal.Image.from_registry(image)
            return modal.Sandbox.create(
                image=modal_image,
                app=app,
                timeout=idle_timeout_secs,
                cpu=cpus,
                memory=memory_mb,
            )

        try:
            sandbox = await asyncio.to_thread(blocking_create)
        except SandboxError:
            raise
        except BaseException as exc:
            raise map_modal_error(exc) from exc

        sandbox_id = str(sandbox.object_id)
        async with self._lock:
            self._handles[sandbox_id] = sandbox
        return CreatedSandbox(
            sandbox_id=sandbox_id,
            image=image or "modal/debian-slim",
            started_at=int(asyncio.get_event_loop().time()),
        )

    async def _handle(self, sandbox_id: str) -> Any:
        async with self._lock:
            handle = self._handles.get(sandbox_id)
        if handle is None:
            # Try to rehydrate via Modal's by-id lookup. Different SDK
            # versions expose this as `Sandbox.from_id` or similar; we
            # fall back to a plain error if the upstream surface
            # doesn't carry the sandbox anymore.
            modal = _import_modal()
            from_id = getattr(modal.Sandbox, "from_id", None)
            if from_id is None:
                raise SandboxError(
                    S_PROVIDER_UNAVAILABLE,
                    f"sandbox {sandbox_id} not in local registry and SDK lacks Sandbox.from_id",
                )
            try:
                handle = await asyncio.to_thread(from_id, sandbox_id)
            except BaseException as exc:
                raise map_modal_error(exc) from exc
            async with self._lock:
                self._handles[sandbox_id] = handle
        return handle

    async def exec(
        self,
        sandbox_id: str,
        cmd: str,
        args: list[str],
        timeout_ms: int | None,
    ) -> ExecResult:
        """`sb.exec(*argv)` returns a process with `.stdout.read()`,
        `.stderr.read()`, `.wait()` and `.returncode`.
        """
        _ = timeout_ms

        def blocking_exec() -> dict[str, Any]:
            sandbox = self._handles.get(sandbox_id)
            if sandbox is None:
                raise SandboxError(S_PROVIDER_UNAVAILABLE, f"sandbox {sandbox_id} not in registry")
            argv = [cmd, *args]
            proc = sandbox.exec(*argv)
            stdout = proc.stdout.read() if hasattr(proc, "stdout") else ""
            stderr = proc.stderr.read() if hasattr(proc, "stderr") else ""
            exit_code = proc.wait() if hasattr(proc, "wait") else 0
            return {"stdout": stdout, "stderr": stderr, "exit_code": int(exit_code or 0)}

        try:
            result = await asyncio.to_thread(blocking_exec)
        except SandboxError:
            raise
        except BaseException as exc:
            raise map_modal_error(exc) from exc
        return ExecResult(
            stdout=str(result.get("stdout") or ""),
            stderr=str(result.get("stderr") or ""),
            exit_code=int(result.get("exit_code") or 0),
            timed_out=False,
        )

    async def stop(self, sandbox_id: str) -> None:
        """`sb.terminate()`. Idempotent — calling on an already-stopped
        sandbox is allowed; we swallow exceptions whose class names
        suggest the sandbox is gone.
        """

        def blocking_stop() -> None:
            sandbox = self._handles.get(sandbox_id)
            if sandbox is None:
                return  # already gone — idempotent success
            sandbox.terminate()

        try:
            await asyncio.to_thread(blocking_stop)
        except BaseException as exc:
            name = type(exc).__name__
            if name in {"NotFoundError", "SandboxNotFoundError"}:
                return
            raise map_modal_error(exc) from exc
        async with self._lock:
            self._handles.pop(sandbox_id, None)

    async def list(self) -> list[dict[str, object]]:
        """Modal does not expose a list-all-sandboxes call in the SDK.
        Return what the worker has tracked locally; the iii worker's
        `do_list` already reconciles in_flight to the returned length.
        """
        async with self._lock:
            ids = list(self._handles.keys())
        return [{"sandbox_id": sid, "image": "", "started_at": ""} for sid in ids]

    async def snapshot(self, sandbox_id: str) -> str:
        """`sb.snapshot_filesystem()` returns a Modal Image whose
        `object_id` is the snapshot identifier.
        """

        def blocking_snapshot() -> str:
            sandbox = self._handles.get(sandbox_id)
            if sandbox is None:
                raise SandboxError(S_PROVIDER_UNAVAILABLE, f"sandbox {sandbox_id} not in registry")
            image = sandbox.snapshot_filesystem()
            return str(image.object_id)

        try:
            return await asyncio.to_thread(blocking_snapshot)
        except SandboxError:
            raise
        except BaseException as exc:
            raise map_modal_error(exc) from exc

    async def expose_port(self, sandbox_id: str, port: int) -> str:
        """Modal expects ports declared up-front via
        `encrypted_ports=[port]` on `Sandbox.create`. Once that's set,
        `sb.tunnels()[port].url` is the public URL.
        Without per-port reconfiguration on existing sandboxes we
        surface a clear error so callers know to declare ports at
        create time.
        """

        def blocking_expose() -> str:
            sandbox = self._handles.get(sandbox_id)
            if sandbox is None:
                raise SandboxError(S_PROVIDER_UNAVAILABLE, f"sandbox {sandbox_id} not in registry")
            tunnels = sandbox.tunnels()
            tunnel = tunnels.get(port)
            if tunnel is None:
                raise SandboxError(
                    S_PROVIDER_UNAVAILABLE,
                    f"port {port} not declared via encrypted_ports at create time",
                )
            return str(tunnel.url)

        try:
            return await asyncio.to_thread(blocking_expose)
        except SandboxError:
            raise
        except BaseException as exc:
            raise map_modal_error(exc) from exc
