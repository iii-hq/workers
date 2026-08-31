"""Guidance injection: pure mutation logic + the config-driven hook bind."""

from __future__ import annotations

import asyncio
from typing import Any

from src import guidance


class FakeTrigger:
    def __init__(self):
        self.unregistered = False

    def unregister(self):
        self.unregistered = True


class FakeIII:
    """Duck-types the slice of IIIClient that guidance touches."""

    def __init__(self):
        self.functions: dict[str, dict[str, Any]] = {}
        self.handlers: dict[str, Any] = {}
        self.trigger_binds: list[dict[str, Any]] = []
        self.trigger_handles: list[FakeTrigger] = []

    def register_function(self, function_id, handler, **kwargs):
        self.functions[function_id] = kwargs
        self.handlers[function_id] = handler

    def register_trigger(self, spec):
        self.trigger_binds.append(spec)
        handle = FakeTrigger()
        self.trigger_handles.append(handle)
        return handle


# ---- pure mutation logic ----------------------------------------------------


def test_mutations_append_guidance_after_a_real_base():
    out = guidance.mutations_for("BASE PROMPT")
    sp = out["mutations"]["system_prompt"]
    assert sp.startswith("BASE PROMPT\n\n")
    assert sp.endswith(guidance.GUIDANCE)


def test_empty_base_emits_no_system_prompt_mutation():
    # Fail-open contract: the harness applies system_prompt only when the key is
    # present, so schema drift (missing base) must preserve the harness prompt.
    assert guidance.mutations_for("") == {"mutations": {}}


def test_handler_is_lenient_on_malformed_payloads():
    assert asyncio.run(guidance.inject_guidance(None)) == {"mutations": {}}
    assert asyncio.run(guidance.inject_guidance({})) == {"mutations": {}}
    assert asyncio.run(guidance.inject_guidance({"generate": {"system_prompt": 42}})) == {"mutations": {}}
    out = asyncio.run(guidance.inject_guidance({"generate": {"system_prompt": "B"}}))
    assert out["mutations"]["system_prompt"].startswith("B\n\n")


def test_guidance_names_the_full_surface():
    for needle in [
        "scrapling::fetch",
        "scrapling::dynamic-fetch",
        "scrapling::stealthy-fetch",
        "solve_cloudflare: true",
        "`selectors`",
        "`extracted`",
        "`urls: [...]`",
        "scrapling::find-similar",
        "scrapling::screenshot",
        "web::fetch",
    ]:
        assert needle in guidance.GUIDANCE, f"missing: {needle}"


# ---- config-driven bind ------------------------------------------------------


def test_register_hook_registers_the_function_without_binding():
    iii = FakeIII()
    guidance.register_hook(iii)

    assert set(iii.functions) == {guidance.HOOK_ID}
    kwargs = iii.functions[guidance.HOOK_ID]
    assert kwargs["request_format"].get("type"), "untyped request schema"
    assert kwargs["response_format"].get("type"), "untyped response schema"
    # Registering the function is inert: no binding until apply(True).
    assert iii.trigger_binds == []


def test_apply_binds_on_and_unbinds_off_idempotently():
    iii = FakeIII()
    state = guidance.GuidanceState()

    # Off with no binding: nothing happens.
    guidance.apply(iii, state, False)
    assert iii.trigger_binds == []

    # On: binds exactly once, with the declarative inject_prompt metadata.
    guidance.apply(iii, state, True)
    guidance.apply(iii, state, True)
    assert iii.trigger_binds == [
        {
            "type": guidance.HOOK_TRIGGER_TYPE,
            "function_id": guidance.HOOK_ID,
            "config": {"on_error": "fail_open"},
            "metadata": {"inject_prompt": guidance.GUIDANCE},
        }
    ]

    # Off: unregisters the live handle and empties the slot.
    guidance.apply(iii, state, False)
    assert iii.trigger_handles[0].unregistered
    assert state.trigger is None

    # Back on: binds a fresh handle.
    guidance.apply(iii, state, True)
    assert len(iii.trigger_binds) == 2
