from src.schemas import MAX_DESCRIPTION, MAX_PROP_DESCRIPTION, fingerprint, slim_schema


def test_slim_schema_shapes_and_truncates():
    schema = {
        "properties": {
            "path": {"type": "string", "description": "x" * 500},
            "mode": {"type": "string", "enum": ["a", "b"]},
            "flag": True,
        },
        "required": ["path"],
        "definitions": {"Dropped": {}},
    }
    tool = slim_schema("shell::fs::read", "d" * 500, schema)
    assert tool["name"] == "shell::fs::read"
    assert len(tool["description"]) == MAX_DESCRIPTION
    props = tool["parameters"]["properties"]
    assert len(props["path"]["description"]) == MAX_PROP_DESCRIPTION
    assert props["mode"]["enum"] == ["a", "b"]
    assert props["flag"] == {"type": "string"}
    assert tool["parameters"]["required"] == ["path"]
    assert "definitions" not in tool["parameters"]


def test_slim_schema_handles_missing_schema():
    tool = slim_schema("state::get", None, None)
    assert tool["parameters"] == {"type": "object", "properties": {}}
    assert tool["description"] == ""


def test_fingerprint_order_independent_and_content_sensitive():
    a = slim_schema("a::x", "one", None)
    b = slim_schema("b::y", "two", None)
    assert fingerprint([a, b]) == fingerprint([b, a])
    changed = slim_schema("a::x", "one changed", None)
    assert fingerprint([a, b]) != fingerprint([changed, b])
