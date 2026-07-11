from src.main import _print_connected


def test_print_connected_only_on_connected_state(capsys) -> None:
    for state in ("connecting", "reconnecting", "disconnected", "failed", "connected"):
        _print_connected(state, "ws://x")
    out = capsys.readouterr().out
    assert out.count("scrapling worker connected to ws://x") == 1
