"""Configuration-worker integration: parse defaults, seeding, retry/NOT_FOUND
classification, and the config-change handler reconciling the guidance
binding."""

from __future__ import annotations

import asyncio
from typing import Any

from src import configuration, guidance


class FakeTrigger:
    """What register_trigger hands back: a handle whose unregister must
    actually be called by the unbind path (a bare object() here once let the
    unbind half of the reconcile test pass vacuously)."""

    def __init__(self) -> None:
        self.unregistered = 0

    def unregister(self) -> None:
        self.unregistered += 1


class FakeIII:
    """Duck-types the slices of IIIClient that configuration touches. `store`
    is the configuration worker's stored value; `None` means no entry yet
    (configuration::get raises NOT_FOUND). `error` (an Exception) makes every
    RPC raise it — the config-plane-down cases."""

    def __init__(self, store: Any | None = None, error: Exception | None = None):
        self.store = store
        self.error = error
        self.calls = 0
        self.registered_payloads: list[dict[str, Any]] = []
        self.functions: dict[str, Any] = {}
        self.trigger_binds: list[dict[str, Any]] = []
        self.triggers: list[FakeTrigger] = []

    def trigger(self, req):
        self.calls += 1
        if self.error is not None:
            raise self.error
        if req["function_id"] == "configuration::get":
            if self.store is None:
                raise RuntimeError("remote error (NOT_FOUND): configuration 'scrapling' not found")
            return {"value": self.store}
        if req["function_id"] == "configuration::register":
            self.registered_payloads.append(req["payload"])
            return {}
        raise AssertionError(f"unexpected trigger: {req['function_id']}")

    async def trigger_async(self, req):
        return self.trigger(req)

    def register_function(self, function_id, handler, **kwargs):
        self.functions[function_id] = handler

    def register_trigger(self, spec):
        self.trigger_binds.append(spec)
        handle = FakeTrigger()
        self.triggers.append(handle)
        return handle


def test_parse_config_defaults_on_and_reads_the_flat_shape():
    assert configuration.parse_config(None) == {"inject_guidance": True}
    assert configuration.parse_config({}) == {"inject_guidance": True}
    # `False` is the non-default value, so this parse is discriminating.
    assert configuration.parse_config({"inject_guidance": False}) == {"inject_guidance": False}
    # Non-boolean junk keeps the default rather than propagating, and unknown
    # keys (including a worker-name wrapper, which the configuration worker
    # never stores) are ignored.
    assert configuration.parse_config({"inject_guidance": "no"}) == {"inject_guidance": True}
    assert configuration.parse_config({"scrapling": {"inject_guidance": False}}) == {"inject_guidance": True}


def test_register_config_seeds_default_only_when_nothing_stored():
    empty = FakeIII(store=None)
    configuration.register_config(empty)
    assert empty.registered_payloads[0]["initial_value"] == {"inject_guidance": True}

    populated = FakeIII(store={"inject_guidance": False})
    configuration.register_config(populated)
    assert "initial_value" not in populated.registered_payloads[0]


def test_fetch_config_defaults_when_missing_and_reads_stored():
    # NOT_FOUND is definitive: no retries burned on the normal first boot.
    missing = FakeIII(store=None)
    assert configuration.fetch_config(missing) == {"inject_guidance": True}
    assert missing.calls == 1
    assert configuration.fetch_config(FakeIII(store={"inject_guidance": False})) == {"inject_guidance": False}


def test_transient_failures_retry_then_raise(monkeypatch):
    """A config plane that is down — including the engine's lowercase
    `function_not_found` when the configuration worker is absent — must
    surface as an error after the retry ladder, never read as "nothing
    stored" (which would silently flip a stored OFF back to the default)."""
    monkeypatch.setattr(configuration, "CONFIG_RETRY_BACKOFF_S", 0)
    down = FakeIII(error=RuntimeError("remote error (function_not_found): configuration::get"))
    try:
        configuration.fetch_config(down)
    except RuntimeError as e:
        assert "function_not_found" in str(e)
    else:
        raise AssertionError("fetch_config must raise when the config plane is down")
    assert down.calls == configuration.CONFIG_RETRIES


def test_config_change_handler_reconciles_the_binding():
    iii = FakeIII(store={"inject_guidance": True})
    state = guidance.GuidanceState()
    configuration.register_config_trigger(iii, state)

    # The handler and its configuration:updated binding are registered.
    assert configuration.CONFIG_FN_ID in iii.functions
    assert iii.trigger_binds[0]["config"]["configuration_id"] == configuration.CONFIG_ID

    handler = iii.functions[configuration.CONFIG_FN_ID]

    async def scenario() -> None:
        # One loop for both deliveries: the reload lock binds to the loop
        # that first acquires it (the SDK's single loop in production).

        # Stored value on → the handler binds the guidance hook.
        assert await handler({}) == {"ok": True}
        assert state.trigger is not None
        bound = state.trigger
        assert any(b["type"] == guidance.HOOK_TRIGGER_TYPE for b in iii.trigger_binds)

        # Stored value off → the handler unbinds it — and the handle's
        # unregister really runs (not just the slot going empty).
        iii.store = {"inject_guidance": False}
        assert await handler({}) == {"ok": True}
        assert state.trigger is None
        assert bound.unregistered == 1

    asyncio.run(scenario())
