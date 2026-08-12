import hashlib
import json

MAX_DESCRIPTION = 200
MAX_PROP_DESCRIPTION = 120

ROUTE_REQUEST = {
    "type": "object",
    "properties": {
        "objective": {
            "type": "string",
            "description": "What should happen next, in natural language",
        },
        "observation": {
            "type": "string",
            "description": "Result of the previous function call, if continuing a chain",
        },
    },
    "required": ["objective"],
}

ROUTE_RESPONSE = {
    "type": "object",
    "properties": {
        "type": {"type": "string", "enum": ["call", "respond", "abstain", "refuse"]},
        "calls": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "function": {"type": "string"},
                    "payload": {"type": "object"},
                },
            },
        },
        "confidence": {"type": "number"},
        "reasoning": {"type": "string"},
        "latency_ms": {"type": "number"},
    },
}

STATUS_RESPONSE = {
    "type": "object",
    "properties": {
        "functions": {"type": "integer"},
        "fingerprint": {"type": "string"},
        "last_init_ms": {"type": "integer"},
        "routes_served": {"type": "integer"},
        "model": {"type": "string"},
        "ready": {"type": "boolean"},
    },
}

REPORT_RESPONSE = {
    "type": "object",
    "properties": {
        "turns": {"type": "integer"},
        "proposals_scored": {"type": "integer"},
        "turns_with_discovery_steps": {"type": "integer"},
        "discovery_steps_total": {"type": "integer"},
        "buckets": {"type": "object"},
    },
}

EMPTY_OBJECT = {"type": "object", "properties": {}}


def slim_schema(function_id, description, request_schema):
    props = {}
    schema = request_schema or {}
    for key, value in (schema.get("properties") or {}).items():
        if not isinstance(value, dict):
            props[key] = {"type": "string"}
            continue
        prop = {"type": value.get("type", "string")}
        if value.get("description"):
            prop["description"] = value["description"][:MAX_PROP_DESCRIPTION]
        if "enum" in value:
            prop["enum"] = value["enum"]
        props[key] = prop
    parameters = {"type": "object", "properties": props}
    if schema.get("required"):
        parameters["required"] = schema["required"]
    return {
        "name": function_id,
        "description": (description or "")[:MAX_DESCRIPTION],
        "parameters": parameters,
    }


def fingerprint(tools):
    payload = json.dumps(sorted(tools, key=lambda t: t["name"]), sort_keys=True)
    return hashlib.sha256(payload.encode()).hexdigest()[:16]
