"""Guidance injection: pure mutation logic + the one-shot hook bind."""

from __future__ import annotations

import asyncio
from typing import Any

from src import guidance


class FakeIII:
    """Duck-types the slice of IIIClient that guidance.setup touches."""

    def __init__(self):
        self.functions: dict[str, dict[str, Any]] = {}
        self.handlers: dict[str, Any] = {}
        self.trigger_binds: list[dict[str, Any]] = []

    def register_function(self, function_id, handler, **kwargs):
        self.functions[function_id] = kwargs
        self.handlers[function_id] = handler

    def register_trigger(self, spec):
        self.trigger_binds.append(spec)


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


# ---- setup: one-shot bind ----------------------------------------------------


def test_setup_registers_hook_and_binds_one_shot():
    iii = FakeIII()
    guidance.setup(iii)

    assert set(iii.functions) == {guidance.HOOK_ID}
    kwargs = iii.functions[guidance.HOOK_ID]
    assert kwargs["request_format"].get("type"), "untyped request schema"
    assert kwargs["response_format"].get("type"), "untyped response schema"

    assert iii.trigger_binds == [
        {
            "type": guidance.HOOK_TRIGGER_TYPE,
            "function_id": guidance.HOOK_ID,
            "config": {"on_error": "fail_open"},
        }
    ]
