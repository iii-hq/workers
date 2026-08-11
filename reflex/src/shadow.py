import json
import threading
import time


class Shadow:
    DISCOVERY = {"engine::functions::list", "engine::functions::info"}

    def __init__(self, router, log_path="shadow.jsonl"):
        self.router = router
        self.log_path = log_path
        self.log_lock = threading.Lock()

    def log(self, row):
        row["ts"] = round(time.time(), 3)
        with self.log_lock:
            with open(self.log_path, "a") as fh:
                fh.write(json.dumps(row) + "\n")

    @staticmethod
    def turn_key(data):
        return f"{data.get('session_id')}:{data.get('turn_id')}:{data.get('step')}"

    @staticmethod
    def block_text(msg):
        parts = [b.get("text", "") for b in msg.get("content") or [] if isinstance(b, dict) and b.get("type") == "text"]
        return "\n".join(p for p in parts if p).strip()

    @classmethod
    def decision_input(cls, messages):
        objective = None
        observation = None
        for msg in reversed(messages or []):
            role = msg.get("role")
            if role == "user" and objective is None:
                objective = cls.block_text(msg) or None
                break
            if role == "function_result" and observation is None:
                text = cls.block_text(msg)
                observation = f"{msg.get('function_id')} returned: {text}" if text else None
        return objective, observation

    def pre_generate(self, data):
        gen = (data or {}).get("generate") or {}
        text, observation = self.decision_input(gen.get("messages"))
        if not text:
            return {}
        key = self.turn_key(data)
        frontier_model = gen.get("model")

        def predict():
            try:
                request = {"objective": text[:2000]}
                if observation:
                    request["observation"] = observation[:1500]
                proposal = self.router.route(request)
                self.log(
                    {
                        "kind": "proposal",
                        "key": key,
                        "session_id": data.get("session_id"),
                        "turn_id": data.get("turn_id"),
                        "step": data.get("step"),
                        "frontier_model": frontier_model,
                        "objective": text[:500],
                        "type": proposal.get("type"),
                        "calls": proposal.get("calls"),
                        "confidence": proposal.get("confidence"),
                        "latency_ms": proposal.get("latency_ms"),
                    }
                )
            except Exception as exc:
                self.log({"kind": "proposal_error", "key": key, "error": str(exc)})

        threading.Thread(target=predict, daemon=True).start()
        return {}

    @staticmethod
    def extract_calls(message):
        calls = []
        for b in message.get("content") or []:
            if not (isinstance(b, dict) and b.get("type") == "function_call"):
                continue
            function = b.get("function_id")
            payload = b.get("arguments")
            if function == "agent_trigger" and isinstance(payload, dict) and payload.get("function"):
                calls.append(
                    {"function": payload["function"], "payload": payload.get("payload"), "via": "agent_trigger"}
                )
            else:
                calls.append({"function": function, "payload": payload})
        return calls

    def post_generate(self, data):
        message = ((data or {}).get("generated") or {}).get("message") or {}
        self.log(
            {
                "kind": "actual",
                "key": self.turn_key(data),
                "session_id": data.get("session_id"),
                "turn_id": data.get("turn_id"),
                "step": data.get("step"),
                "stop_reason": message.get("stop_reason"),
                "model": message.get("model"),
                "calls": self.extract_calls(message),
            }
        )
        return {}

    def report(self):
        proposals, actuals = [], []
        try:
            with open(self.log_path) as fh:
                for line in fh:
                    row = json.loads(line)
                    if row.get("kind") == "proposal":
                        proposals.append(row)
                    elif row.get("kind") == "actual":
                        actuals.append(row)
        except FileNotFoundError:
            pass

        turns = {}
        for act in actuals:
            turn_key = f"{act.get('session_id')}:{act.get('turn_id')}"
            turn = turns.setdefault(turn_key, {"functions": set(), "discovery_steps": 0, "steps": 0})
            turn["steps"] += 1
            for call in act.get("calls") or []:
                fn = call.get("function")
                if fn in self.DISCOVERY:
                    turn["discovery_steps"] += 1
                elif fn:
                    turn["functions"].add(fn)

        buckets = {}
        scored = 0
        for prop in proposals:
            turn = turns.get(f"{prop.get('session_id')}:{prop.get('turn_id')}")
            if turn is None:
                continue
            scored += 1
            p_calls = prop.get("calls") or []
            p_fn = p_calls[0].get("function") if p_calls else None
            if p_fn and p_fn in turn["functions"]:
                outcome = "match"
            elif p_fn:
                outcome = "mismatch"
            elif turn["functions"]:
                outcome = "abstained"
            else:
                outcome = "both_idle"
            conf = prop.get("confidence") or 0
            label = f"{min(int(conf * 5), 4) * 0.2:.1f}"
            bucket = buckets.setdefault(label, {"n": 0, "match": 0, "mismatch": 0, "abstained": 0, "both_idle": 0})
            bucket["n"] += 1
            bucket[outcome] += 1

        discovery_turns = sum(1 for t in turns.values() if t["discovery_steps"])
        return {
            "turns": len(turns),
            "proposals_scored": scored,
            "turns_with_discovery_steps": discovery_turns,
            "discovery_steps_total": sum(t["discovery_steps"] for t in turns.values()),
            "buckets": dict(sorted(buckets.items())),
        }
