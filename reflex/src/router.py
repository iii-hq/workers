import os
import threading
import time

from src.schemas import fingerprint, slim_schema

SELF_PREFIX = "reflex::"


class Router:
    def __init__(self, client, index_path=".index/functions.idx", refresh_debounce_s=5):
        self.client = client
        self.index_path = index_path
        self.refresh_debounce_s = refresh_debounce_s
        self.lock = threading.Lock()
        self.agent = None
        self.tools = []
        self.fingerprint = None
        self.last_init_ms = None
        self.routes_served = 0
        self.refresh_timer = None

    def fetch_catalog(self):
        listing = self.client.trigger({"function_id": "engine::functions::list", "payload": {}})
        functions = [f for f in listing["functions"] if not f["function_id"].startswith(SELF_PREFIX)]
        tools = []
        for f in functions:
            try:
                info = self.client.trigger(
                    {"function_id": "engine::functions::info", "payload": {"function_id": f["function_id"]}}
                )
                tools.append(slim_schema(f["function_id"], info.get("description"), info.get("request_schema")))
            except Exception:
                tools.append(slim_schema(f["function_id"], f.get("description"), None))
        return tools

    def rebuild(self):
        from needle import Needle

        tools = self.fetch_catalog()
        fp = fingerprint(tools)
        if fp == self.fingerprint and self.agent is not None:
            return False
        index_dir = os.path.dirname(self.index_path)
        if index_dir:
            os.makedirs(index_dir, exist_ok=True)
        with self.lock:
            t0 = time.time()
            self.agent = Needle(tools=tools, tool_index_path=self.index_path)
            self.last_init_ms = round((time.time() - t0) * 1000)
            self.tools = tools
            self.fingerprint = fp
        print(f"reflex: index built, {len(tools)} functions, {self.last_init_ms}ms")
        return True

    def schedule_refresh(self):
        if self.refresh_timer:
            self.refresh_timer.cancel()

        def refresh():
            try:
                self.rebuild()
            except Exception as exc:
                print(f"reflex: refresh failed: {exc}")

        self.refresh_timer = threading.Timer(self.refresh_debounce_s, refresh)
        self.refresh_timer.daemon = True
        self.refresh_timer.start()

    def route(self, payload):
        objective = (payload or {}).get("objective")
        if not objective:
            return {"error": "objective is required"}
        if self.agent is None:
            return {"error": "index not ready"}
        observation = (payload or {}).get("observation")
        if observation:
            text = (
                f"OBJECTIVE:\n{objective}\n\nLAST OBSERVATION:\n{observation}\n\n"
                f"Choose the next function call if one is clearly required."
            )
        else:
            text = objective
        with self.lock:
            t0 = time.time()
            self.agent.reset()
            result = self.agent.complete(text)
            latency_ms = round((time.time() - t0) * 1000, 1)
        self.routes_served += 1
        calls = [
            {"function": c.get("name"), "payload": c.get("arguments") or {}}
            for c in (result.get("function_calls") or [])
        ]
        kind = result.get("type")
        if kind == "call" and not calls:
            kind = "abstain"
        return {
            "type": kind,
            "calls": calls,
            "confidence": result.get("confidence"),
            "reasoning": result.get("reasoning"),
            "latency_ms": latency_ms,
        }

    def status(self):
        return {
            "functions": len(self.tools),
            "fingerprint": self.fingerprint,
            "last_init_ms": self.last_init_ms,
            "routes_served": self.routes_served,
            "model": "needle2",
            "ready": self.agent is not None,
        }
