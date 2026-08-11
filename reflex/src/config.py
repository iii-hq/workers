import os

import yaml

DEFAULTS = {
    "engine_url": "ws://127.0.0.1:49134",
    "index_path": ".index/functions.idx",
    "shadow_log": "shadow.jsonl",
    "shadow": {"enabled": True, "priority": 100, "timeout_ms": 2000},
    "refresh_debounce_s": 5,
}

ENV_OVERRIDES = {
    "engine_url": "III_URL",
    "index_path": "REFLEX_INDEX_PATH",
    "shadow_log": "REFLEX_SHADOW_LOG",
}


def load(path="config.yaml"):
    cfg = dict(DEFAULTS)
    cfg["shadow"] = dict(DEFAULTS["shadow"])
    try:
        with open(path) as fh:
            raw = yaml.safe_load(fh) or {}
    except FileNotFoundError:
        raw = {}
    for key, value in raw.items():
        if key == "shadow" and isinstance(value, dict):
            cfg["shadow"].update(value)
        elif key in cfg:
            cfg[key] = value
    for key, env in ENV_OVERRIDES.items():
        if os.environ.get(env):
            cfg[key] = os.environ[env]
    return cfg
