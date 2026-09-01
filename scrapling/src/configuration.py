"""Integration with the builtin `configuration` worker: register the
`scrapling` config schema (+ default seed) at boot, read the authoritative
value, and bind a `configuration` trigger so `configuration:updated`
re-fetches and applies the change — which here means binding or unbinding
the `scrapling::inject-guidance` pre-generate hook at runtime.
Mirrors the Rust workers' shared plumbing (workers/crates/config-client):
same retry ladder, same case-SENSITIVE `NOT_FOUND` rule, same serialized
reload.
"""

from __future__ import annotations

import asyncio
import logging
import time
from typing import Any

from . import guidance

log = logging.getLogger("scrapling.configuration")

CONFIG_ID = "scrapling"
CONFIG_FN_ID = "scrapling::on-config-change"
CONFIG_TIMEOUT_MS = 5_000
CONFIG_RETRIES = 3
CONFIG_RETRY_BACKOFF_S = 0.25

DEFAULTS: dict[str, Any] = {"inject_guidance": True}

CONFIG_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "inject_guidance": {
            "type": "boolean",
            "default": True,
            "description": (
                "Append the scrapling usage guidance to every agent system prompt via the "
                "harness pre-generate hook. On by default; turn it off to shrink prompts "
                "(the harness's granted-functions catalog still advertises the scrapling::* "
                "ids). Hot-applies — the worker binds or unbinds the hook on change."
            ),
        },
    },
}

ON_CONFIG_CHANGE_REQUEST: dict[str, Any] = {"type": "object", "properties": {}}
ON_CONFIG_CHANGE_RESPONSE: dict[str, Any] = {
    "type": "object",
    "properties": {"ok": {"type": "boolean"}},
    "required": ["ok"],
}


def parse_config(value: Any) -> dict[str, Any]:
    """Missing keys fall back to defaults; the configuration worker stores
    the flat object the schema describes."""
    cfg = dict(DEFAULTS)
    if isinstance(value, dict) and isinstance(value.get("inject_guidance"), bool):
        cfg["inject_guidance"] = value["inject_guidance"]
    return cfg


def _is_not_found(e: Exception) -> bool:
    """The configuration worker's missing-entry code is the uppercase literal
    `NOT_FOUND`. Deliberately case-SENSITIVE: the engine's missing-FUNCTION
    code is lowercase `function_not_found` (vendored SDK iii.py), and a
    configuration worker that is absent or unroutable must surface as an
    error, never read as "nothing stored yet"."""
    return "NOT_FOUND" in str(e)


def _trigger_with_retry(iii: Any, function_id: str, payload: dict[str, Any]) -> Any:
    """Boot-path RPC with the Rust siblings' retry ladder. `NOT_FOUND` is a
    definitive answer (nothing stored yet, the normal first-ever boot), not a
    transient failure — it is raised immediately instead of retried."""
    last: Exception | None = None
    for attempt in range(1, CONFIG_RETRIES + 1):
        try:
            return iii.trigger(
                {
                    "function_id": function_id,
                    "namespace": "default",
                    "payload": payload,
                    "timeout_ms": CONFIG_TIMEOUT_MS,
                }
            )
        except Exception as e:  # noqa: BLE001 — classified below
            last = e
            if _is_not_found(e):
                raise
            if attempt < CONFIG_RETRIES:
                log.warning(
                    "configuration RPC %s failed (attempt %d); retrying: %s",
                    function_id,
                    attempt,
                    e,
                )
                time.sleep(CONFIG_RETRY_BACKOFF_S * attempt)
    raise RuntimeError(f"{function_id} failed after {CONFIG_RETRIES} attempts: {last}") from last


def _try_get_value(iii: Any) -> Any | None:
    """The stored value, or None when the entry does not exist yet."""
    try:
        resp = _trigger_with_retry(iii, "configuration::get", {"id": CONFIG_ID})
    except Exception as e:  # noqa: BLE001 — NOT_FOUND is the normal first boot
        if _is_not_found(e):
            return None
        raise
    return (resp or {}).get("value") if isinstance(resp, dict) else None


def register_config(iii: Any) -> None:
    """Register the schema, seeding the default only when nothing is stored:
    `configuration::register` REPLACES the stored value whenever
    `initial_value` is supplied, so the pre-check is what makes calling this
    every boot safe."""
    payload: dict[str, Any] = {
        "id": CONFIG_ID,
        "name": "scrapling",
        "description": (
            "scrapling worker settings — whether its usage guidance is injected into "
            "agent system prompts (on by default)."
        ),
        "schema": CONFIG_SCHEMA,
        "metadata": {"ui_form": CONFIG_ID},
    }
    if _try_get_value(iii) is None:
        payload["initial_value"] = dict(DEFAULTS)
    _trigger_with_retry(iii, "configuration::register", payload)


def fetch_config(iii: Any) -> dict[str, Any]:
    """The authoritative config, defaulted when nothing is stored."""
    return parse_config(_try_get_value(iii))


def register_config_trigger(iii: Any, state: guidance.GuidanceState) -> None:
    """Register `scrapling::on-config-change` and bind it to
    `configuration:updated` for the `scrapling` entry. Every delivery
    re-fetches the authoritative value and reconciles the guidance binding,
    serialized by one lock with the fetch INSIDE it — overlapping deliveries
    converge on the latest authoritative value instead of racing."""

    reload_lock = asyncio.Lock()

    async def on_config_change(_payload: dict[str, Any] | None) -> dict[str, Any]:
        async with reload_lock:
            try:
                resp = await iii.trigger_async(
                    {
                        "function_id": "configuration::get",
                        "namespace": "default",
                        "payload": {"id": CONFIG_ID},
                        "timeout_ms": CONFIG_TIMEOUT_MS,
                    }
                )
                cfg = parse_config((resp or {}).get("value") if isinstance(resp, dict) else None)
            except Exception as e:  # noqa: BLE001 — keep the previous config on any failure
                log.error("config-change: fetch failed; keeping previous config: %s", e)
                return {"ok": False}
            guidance.apply(iii, state, cfg["inject_guidance"])
            log.info("scrapling configuration reloaded (inject_guidance=%s)", cfg["inject_guidance"])
            return {"ok": True}

    iii.register_function(
        CONFIG_FN_ID,
        on_config_change,
        description="Internal: reload scrapling settings from the authoritative configuration on change.",
        request_format=ON_CONFIG_CHANGE_REQUEST,
        response_format=ON_CONFIG_CHANGE_RESPONSE,
        metadata={"internal": True},
    )

    iii.register_trigger(
        {
            "type": "configuration",
            "function_id": CONFIG_FN_ID,
            "config": {"configuration_id": CONFIG_ID, "event_types": ["configuration:updated"]},
        }
    )
