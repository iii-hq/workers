"""Guidance injection: pure mutation logic + the presence-gated hook bind."""

from __future__ import annotations

import asyncio
from typing import Any

from src import guidance

READY = {"triggers": [{"id": guidance.HOOK_TRIGGER_TYPE}, {"id": "http"}]}
NOT_READY = {"triggers": [{"id": "http"}]}


class FakeIII:
    """Duck-types the slice of IIIClient that guidance.setup touches."""

    def __init__(self, triggers_resp: Any = None, fail_hook_binds: int = 0):
        self.functions: dict[str, dict[str, Any]] = {}
        self.handlers: dict[str, Any] = {}
        self.trigger_binds: list[dict[str, Any]] = []
        self.triggers_resp = triggers_resp
        self.fail_hook_binds = fail_hook_binds

    def register_function(self, function_id, handler, **kwargs):
        self.functions[function_id] = kwargs
        self.handlers[function_id] = handler

    def register_trigger(self, spec):
        if spec["type"] == guidance.HOOK_TRIGGER_TYPE and self.fail_hook_binds > 0:
            self.fail_hook_binds -= 1
            raise RuntimeError("engine hiccup")
        self.trigger_binds.append(spec)

    def trigger(self, request):
        assert request["function_id"] == "engine::triggers::list"
        assert request["payload"] == {"include_internal": True}
        if self.triggers_resp is None:
            raise RuntimeError("engine down")
        return self.triggers_resp

    async def trigger_async(self, request):
        return self.trigger(request)

    def hook_binds(self):
        return [b for b in self.trigger_binds if b["type"] == guidance.HOOK_TRIGGER_TYPE]


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


def test_hook_type_present():
    assert guidance.hook_type_present(READY)
    assert not guidance.hook_type_present(NOT_READY)
    assert not guidance.hook_type_present({})
    assert not guidance.hook_type_present({"triggers": "nope"})
    assert not guidance.hook_type_present(None)


# ---- setup + bind lifecycle --------------------------------------------------


def test_setup_warm_start_binds_hook_when_harness_ready():
    iii = FakeIII(triggers_resp=READY)
    guidance.setup(iii)

    assert set(iii.functions) == {guidance.HOOK_ID, guidance.ON_REGISTRY_CHANGED_ID}
    for kwargs in iii.functions.values():
        assert kwargs["request_format"].get("type"), "untyped request schema"
        assert kwargs["response_format"].get("type"), "untyped response schema"

    registry_binds = [b for b in iii.trigger_binds if b["type"] in guidance.REGISTRY_TRIGGER_TYPES]
    assert {b["type"] for b in registry_binds} == set(guidance.REGISTRY_TRIGGER_TYPES)
    assert all(b["function_id"] == guidance.ON_REGISTRY_CHANGED_ID for b in registry_binds)

    hooks = iii.hook_binds()
    assert len(hooks) == 1
    assert hooks[0]["function_id"] == guidance.HOOK_ID
    assert hooks[0]["config"] == {"on_error": "fail_open"}


def test_setup_defers_bind_until_registry_event_says_ready():
    iii = FakeIII(triggers_resp=NOT_READY)
    guidance.setup(iii)
    assert iii.hook_binds() == []

    on_changed = iii.handlers[guidance.ON_REGISTRY_CHANGED_ID]
    iii.triggers_resp = READY
    assert asyncio.run(on_changed({})) == {"ok": True}
    assert len(iii.hook_binds()) == 1

    # Further events are no-ops: the bind is claimed exactly once.
    asyncio.run(on_changed({}))
    assert len(iii.hook_binds()) == 1


def test_setup_survives_engine_down_at_boot():
    iii = FakeIII(triggers_resp=None)  # probe raises
    guidance.setup(iii)
    assert iii.hook_binds() == []


def test_failed_hook_bind_releases_the_claim_for_a_retry():
    iii = FakeIII(triggers_resp=READY, fail_hook_binds=1)
    guidance.setup(iii)  # warm-start bind attempt raises → claim released
    assert iii.hook_binds() == []

    on_changed = iii.handlers[guidance.ON_REGISTRY_CHANGED_ID]
    asyncio.run(on_changed({}))
    assert len(iii.hook_binds()) == 1
