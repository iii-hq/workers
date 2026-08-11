from src.config import DEFAULTS, load


def test_load_defaults_when_missing(tmp_path):
    cfg = load(path=str(tmp_path / "config.yaml"))
    assert cfg["engine_url"] == DEFAULTS["engine_url"]
    assert cfg["shadow"]["priority"] == 100


def test_load_merges_file_and_env(tmp_path, monkeypatch):
    path = tmp_path / "config.yaml"
    path.write_text("engine_url: ws://example:1\nshadow:\n  timeout_ms: 900\nunknown_key: ignored\n")
    monkeypatch.setenv("III_URL", "ws://env-wins:2")
    cfg = load(path=str(path))
    assert cfg["engine_url"] == "ws://env-wins:2"
    assert cfg["shadow"]["timeout_ms"] == 900
    assert cfg["shadow"]["priority"] == 100
    assert "unknown_key" not in cfg
