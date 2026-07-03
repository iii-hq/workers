"""`scrapling::inject-guidance` — a harness `pre_generate` hook that appends the
scrapling usage guidance to the agent system prompt, only while this worker is
connected. The binding dies with the worker, so the guidance is presence-gated
for free: a deployment without scrapling never pays for it.

The hook may only be bound after the harness has registered its
`harness::hook::pre-generate` trigger type — `register_trigger` is
fire-and-forget and the engine silently drops binds to unknown types. So
`setup()` also registers `scrapling::on-registry-changed`, bound to the engine
registry-change triggers, and (re)tries the bind on every firing, plus once at
boot for the warm-start case (harness already up → no event will fire).
Mirrors web/src/functions/inject_guidance.rs + configuration.rs.
"""

from __future__ import annotations

import logging
import threading
from typing import Any

log = logging.getLogger("scrapling.guidance")

HOOK_ID = "scrapling::inject-guidance"
ON_REGISTRY_CHANGED_ID = "scrapling::on-registry-changed"
HOOK_TRIGGER_TYPE = "harness::hook::pre-generate"
REGISTRY_TRIGGER_TYPES = ("engine::workers-available", "engine::functions-available")
PROBE_TIMEOUT_MS = 5_000

# The single canonical copy of the scrapling usage guidance (same role as
# WEB_GUIDANCE in the web worker). Pure USAGE guidance: the hook only fires
# while this worker is present, so no "look for it / install it" text.
GUIDANCE = (
    "For scraping web pages — pulling structured data (titles, prices, links, listings) out of HTML — "
    "use `scrapling::*`, never hand-rolled fetch+regex (plain JSON/REST APIs stay on `web::fetch`). "
    "Escalate fetchers only as needed: `scrapling::fetch` (fast HTTP, TLS impersonation) → "
    "`scrapling::dynamic-fetch` (real Chromium, for JS-rendered pages) → `scrapling::stealthy-fetch` "
    "(Camoufox anti-bot; pass `solve_cloudflare: true` for Cloudflare walls). ALWAYS pass inline "
    "`selectors` (`[{name, css|xpath|regex, attr?, all?}]`) so the response carries compact `extracted` "
    "fields instead of raw HTML that floods your context; set `include_html: true` only when you truly "
    "need the source. Fetch many pages in one call with `urls: [...]`. On HTML you already have, use the "
    "pure parsers `scrapling::extract` / `scrapling::css` / `scrapling::xpath` / `scrapling::regex` / "
    "`scrapling::find-similar` (no network, no approval needed). `scrapling::screenshot` returns a base64 "
    "image of the rendered page. Fetch exact request shapes via `engine::functions::info` before the "
    "first call."
)

PRE_GENERATE_REQUEST: dict[str, Any] = {
    "type": "object",
    "properties": {
        "generate": {
            "type": "object",
            "properties": {
                "system_prompt": {"type": "string", "description": "system prompt assembled so far"},
            },
        },
    },
}
PRE_GENERATE_RESPONSE: dict[str, Any] = {
    "type": "object",
    "properties": {
        "mutations": {
            "type": "object",
            "properties": {
                "system_prompt": {"type": "string", "description": "full replacement prompt (base + guidance)"},
            },
        },
    },
    "required": ["mutations"],
}
REGISTRY_CHANGED_REQUEST: dict[str, Any] = {
    "description": "Registry-change event; payload is ignored — the handler just retries the hook bind.",
    "type": ["null", "boolean", "number", "string", "array", "object"],
}
REGISTRY_CHANGED_RESPONSE: dict[str, Any] = {
    "type": "object",
    "properties": {"ok": {"type": "boolean"}},
    "required": ["ok"],
}


def mutations_for(base: str) -> dict[str, Any]:
    """Build the pre_generate mutations for a given base prompt.

    An empty base (missing/renamed `generate.system_prompt` → schema drift) must
    PRESERVE the harness's assembled prompt: emit no `system_prompt` key at all —
    the harness applies the mutation only when the key is present. For a real
    base, return the FULL prompt (the harness overwrites, it does not merge).
    """
    if not base:
        return {"mutations": {}}
    return {"mutations": {"system_prompt": f"{base}\n\n{GUIDANCE}"}}


async def inject_guidance(payload: dict[str, Any] | None) -> dict[str, Any]:
    """`pre_generate` hook entrypoint. Bound fail_open — lenient on every field."""
    generate = (payload or {}).get("generate") or {}
    base = generate.get("system_prompt") if isinstance(generate, dict) else ""
    return mutations_for(base if isinstance(base, str) else "")


def hook_type_present(resp: Any) -> bool:
    """True iff an `engine::triggers::list` response carries the harness hook type."""
    if not isinstance(resp, dict):
        return False
    triggers = resp.get("triggers")
    if not isinstance(triggers, list):
        return False
    return any(isinstance(t, dict) and t.get("id") == HOOK_TRIGGER_TYPE for t in triggers)


class BindOnce:
    """Claim/release around the one hook bind. Thread-safe because the boot-time
    attempt runs on the main thread while event retries run on the SDK loop."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._bound = False

    def is_bound(self) -> bool:
        with self._lock:
            return self._bound

    def claim(self) -> bool:
        with self._lock:
            if self._bound:
                return False
            self._bound = True
            return True

    def release(self) -> None:
        with self._lock:
            self._bound = False


def _probe_request() -> dict[str, Any]:
    return {
        "function_id": "engine::triggers::list",
        "payload": {"include_internal": True},
        "timeout_ms": PROBE_TIMEOUT_MS,
    }


def _bind_hook(iii: Any, state: BindOnce) -> None:
    if not state.claim():
        return
    try:
        # on_error fail_open is MANDATORY: pre_generate defaults fail-CLOSED, and a
        # missing guidance line must never abort a turn.
        iii.register_trigger({"type": HOOK_TRIGGER_TYPE, "function_id": HOOK_ID, "config": {"on_error": "fail_open"}})
        log.info("scrapling pre-generate hook bound (guidance injection active)")
    except Exception:
        state.release()
        log.warning("inject-guidance bind failed; retrying on the next registry-change event", exc_info=True)


def try_bind_sync(iii: Any, state: BindOnce) -> None:
    """Boot-time warm start; a probe failure just defers to the event retries."""
    if state.is_bound():
        return
    try:
        resp = iii.trigger(_probe_request())
    except Exception:
        return
    if hook_type_present(resp):
        _bind_hook(iii, state)


async def try_bind_async(iii: Any, state: BindOnce) -> None:
    if state.is_bound():
        return
    try:
        resp = await iii.trigger_async(_probe_request())
    except Exception:
        return
    if hook_type_present(resp):
        _bind_hook(iii, state)


def setup(iii: Any) -> None:
    """Register the hook + its presence-gated bind plumbing. Call once at boot."""
    state = BindOnce()

    iii.register_function(
        HOOK_ID,
        inject_guidance,
        description=(
            "Internal pre_generate hook: appends scrapling usage guidance to the agent system prompt. "
            "Bound to harness::hook::pre-generate at worker startup; not called directly."
        ),
        request_format=PRE_GENERATE_REQUEST,
        response_format=PRE_GENERATE_RESPONSE,
    )

    async def on_registry_changed(_payload: Any) -> dict[str, Any]:
        await try_bind_async(iii, state)
        return {"ok": True}

    iii.register_function(
        ON_REGISTRY_CHANGED_ID,
        on_registry_changed,
        description=(
            "Internal: on engine registry changes, bind the scrapling inject-guidance pre-generate "
            "hook once the harness has registered its trigger type. Not called directly."
        ),
        request_format=REGISTRY_CHANGED_REQUEST,
        response_format=REGISTRY_CHANGED_RESPONSE,
    )

    # Arm the event-driven retries BEFORE the warm-start probe, so a harness that
    # comes up mid-probe is still caught.
    for trigger_type in REGISTRY_TRIGGER_TYPES:
        try:
            iii.register_trigger({"type": trigger_type, "function_id": ON_REGISTRY_CHANGED_ID, "config": {}})
        except Exception:
            log.warning("registry-change bind failed for %s", trigger_type, exc_info=True)

    try_bind_sync(iii, state)
