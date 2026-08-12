import time

from iii import InitOptions, register_worker

from src.config import load
from src.router import Router
from src.schemas import EMPTY_OBJECT, REPORT_RESPONSE, ROUTE_REQUEST, ROUTE_RESPONSE, STATUS_RESPONSE
from src.shadow import Shadow


def main():
    cfg = load()
    client = register_worker(cfg["engine_url"], InitOptions(worker_name="reflex"))
    router = Router(client, index_path=cfg["index_path"], refresh_debounce_s=cfg["refresh_debounce_s"])
    shadow = Shadow(router, log_path=cfg["shadow_log"])
    hook_config = {
        "priority": cfg["shadow"]["priority"],
        "timeout_ms": cfg["shadow"]["timeout_ms"],
        "on_error": "fail_open",
    }

    client.register_function(
        "reflex::route",
        lambda data: router.route(data),
        description=(
            "Propose the next iii function call for a natural-language objective "
            "using a local on-device router model. Proposes only, never executes. "
            "Returns calls, a calibrated confidence score, and reasoning. "
            "Pass the previous result as `observation` to continue a chain."
        ),
        request_format=ROUTE_REQUEST,
        response_format=ROUTE_RESPONSE,
    )
    client.register_function(
        "reflex::index::status",
        lambda data: router.status(),
        description="Local function-index status: size, fingerprint, init latency, routes served.",
        request_format=EMPTY_OBJECT,
        response_format=STATUS_RESPONSE,
    )
    client.register_function(
        "reflex::index::refresh",
        lambda data: {"changed": router.rebuild(), **router.status()},
        description="Rebuild the local function index from the live engine catalog now.",
        request_format=EMPTY_OBJECT,
        response_format=STATUS_RESPONSE,
    )
    client.register_function(
        "reflex::on-functions-change",
        lambda data: router.schedule_refresh() or {},
        description="Internal: debounced index refresh on engine catalog changes.",
        metadata={"internal": True},
        request_format=EMPTY_OBJECT,
        response_format=EMPTY_OBJECT,
    )
    client.register_function(
        "reflex::shadow::pre-generate",
        shadow.pre_generate,
        description="Internal: shadow-mode observer; predicts the next call in the background, never mutates the turn.",
        metadata={"internal": True},
        request_format=EMPTY_OBJECT,
        response_format=EMPTY_OBJECT,
    )
    client.register_function(
        "reflex::shadow::post-generate",
        shadow.post_generate,
        description="Internal: shadow-mode observer; records the frontier model's actual calls for calibration.",
        metadata={"internal": True},
        request_format=EMPTY_OBJECT,
        response_format=EMPTY_OBJECT,
    )
    client.register_function(
        "reflex::shadow::report",
        lambda data: shadow.report(),
        description="Shadow-mode calibration report: proposal vs actual call agreement per confidence bucket.",
        request_format=EMPTY_OBJECT,
        response_format=REPORT_RESPONSE,
    )

    client.register_trigger(
        {"type": "engine::functions-available", "function_id": "reflex::on-functions-change", "config": {}}
    )
    if cfg["shadow"]["enabled"]:
        client.register_trigger(
            {
                "type": "harness::hook::pre-generate",
                "function_id": "reflex::shadow::pre-generate",
                "config": hook_config,
            }
        )
        client.register_trigger(
            {
                "type": "harness::hook::post-generate",
                "function_id": "reflex::shadow::post-generate",
                "config": hook_config,
            }
        )

    router.rebuild()
    print(f"reflex: ready, {len(router.tools)} functions indexed")

    while True:
        time.sleep(60)


if __name__ == "__main__":
    main()
