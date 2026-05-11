"""Shared error codes and capability set for sandbox-modal.

Mirrors the stable S-code space used across every sandbox provider worker
in this repo. S100-S400 mirror the libkrun-backed iii-sandbox daemon; S404
and S500-S503 are REST-/SDK-specific extensions.
"""

from __future__ import annotations

from dataclasses import dataclass

S_IMAGE_NOT_ALLOWED = "S100"
S_CONCURRENCY_CAP = "S400"
S_CAPABILITY_UNSUPPORTED = "S404"
S_RATE_LIMITED = "S500"
S_QUOTA_EXHAUSTED = "S501"
S_PROVIDER_UNAVAILABLE = "S502"
S_AUTH_INVALID = "S503"

CAPABILITIES = ["snapshot", "expose_port"]


class SandboxError(Exception):
    """Stable error type for sandbox::provider::modal::* failures.

    The S-code is embedded at the start of the message so callers that
    pattern-match on `[Sxxx]` see the same surface every other sandbox
    provider emits.
    """

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"[{code}] {message}")
        self.code = code


def map_modal_error(exc: BaseException) -> SandboxError:
    """Translate an exception raised by the modal SDK into a SandboxError.

    Modal's exception hierarchy ships a handful of named classes
    (`AuthError`, `RateLimitError`, etc.); we resolve them by name string
    so the worker does not import `modal` at module load time, which keeps
    the test suite runnable without real Modal credentials.
    """
    name = type(exc).__name__
    if name in {"AuthError", "InvalidError"}:
        return SandboxError(S_AUTH_INVALID, f"modal auth: {exc}")
    if name == "RateLimitError":
        return SandboxError(S_RATE_LIMITED, f"modal rate limited: {exc}")
    if name == "QuotaExceeded" or "quota" in str(exc).lower():
        return SandboxError(S_QUOTA_EXHAUSTED, f"modal quota: {exc}")
    return SandboxError(S_PROVIDER_UNAVAILABLE, f"modal: {exc}")


@dataclass
class CreatedSandbox:
    sandbox_id: str
    image: str
    started_at: int


@dataclass
class ExecResult:
    stdout: str
    stderr: str
    exit_code: int
    timed_out: bool
