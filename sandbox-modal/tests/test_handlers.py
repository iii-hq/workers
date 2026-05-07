"""Smoke tests for sandbox-modal handlers.

Exercises validation, concurrency tracking, and the list capacity envelope
without contacting Modal. The ModalClient stub raises SandboxError on
every method, which is exactly what the v0 stub bodies do — so the tests
also serve as a contract that the handler still releases concurrency
slots when the upstream client fails.
"""

from __future__ import annotations

import asyncio

import pytest

from src.config import Config
from src.handlers import HandlerCtx, do_create, do_exec, do_list, do_stop
from src.modal_client import ModalClient
from src.sandbox import S_IMAGE_NOT_ALLOWED, SandboxError


def make_ctx(max_concurrent: int = 10, allowlist: list[str] | None = None) -> HandlerCtx:
    cfg = Config(
        max_concurrent_sandboxes=max_concurrent,
        image_allowlist=list(allowlist or []),
    )
    return HandlerCtx(config=cfg, client=ModalClient(app_name="test"))


@pytest.mark.asyncio
async def test_create_rejects_image_not_in_allowlist() -> None:
    ctx = make_ctx(allowlist=["python:3.11"])
    with pytest.raises(SandboxError) as excinfo:
        await do_create(ctx, {"image": "node:22"})
    assert excinfo.value.code == S_IMAGE_NOT_ALLOWED


@pytest.mark.asyncio
async def test_create_rolls_back_in_flight_on_failure() -> None:
    ctx = make_ctx(max_concurrent=2)
    with pytest.raises(SandboxError):
        await do_create(ctx, {"image": "python:3.11"})
    listed = await do_list(ctx, {})
    assert listed == {"sandboxes": [], "in_flight": 0, "cap": 2, "remaining": 2}


@pytest.mark.asyncio
async def test_exec_rejects_missing_fields() -> None:
    ctx = make_ctx()
    with pytest.raises(SandboxError) as excinfo:
        await do_exec(ctx, {})
    assert "missing string field" in str(excinfo.value)


@pytest.mark.asyncio
async def test_stop_rolls_through_stub_client() -> None:
    ctx = make_ctx()
    with pytest.raises(SandboxError):
        await do_stop(ctx, {"sandbox_id": "sbx-1"})


@pytest.mark.asyncio
async def test_list_reports_capacity_envelope() -> None:
    ctx = make_ctx(max_concurrent=7)
    result = await do_list(ctx, {})
    assert result == {"sandboxes": [], "in_flight": 0, "cap": 7, "remaining": 7}


def test_event_loop_available() -> None:
    # Smoke: pytest-asyncio fixtures imply a working loop policy under all
    # supported Python versions.
    asyncio.get_event_loop_policy()
