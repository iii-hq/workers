from src.router import Router


class StubClient:
    def __init__(self, listing, infos):
        self.listing = listing
        self.infos = infos

    def trigger(self, request):
        if request["function_id"] == "engine::functions::list":
            return self.listing
        info = self.infos.get(request["payload"]["function_id"])
        if isinstance(info, Exception):
            raise info
        return info


def test_route_requires_objective_and_index():
    router = Router(client=None)
    assert router.route({}) == {"error": "objective is required"}
    assert router.route({"objective": "list workers"}) == {"error": "index not ready"}


def test_fetch_catalog_excludes_self_and_survives_info_errors():
    listing = {
        "functions": [
            {"function_id": "worker::list", "description": "short"},
            {"function_id": "reflex::route", "description": "self"},
            {"function_id": "state::get", "description": "fallback"},
        ]
    }
    infos = {
        "worker::list": {
            "description": "List workers",
            "request_schema": {"properties": {"running_only": {"type": "boolean"}}},
        },
        "state::get": RuntimeError("info unavailable"),
    }
    router = Router(client=StubClient(listing, infos))
    tools = router.fetch_catalog()
    names = [t["name"] for t in tools]
    assert names == ["worker::list", "state::get"]
    assert tools[0]["parameters"]["properties"]["running_only"] == {"type": "boolean"}
    assert tools[1]["description"] == "fallback"
