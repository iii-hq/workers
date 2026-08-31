import importlib.util
import tarfile
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[3] / "scrapling" / "scripts" / "build_release_runtime.py"
SPEC = importlib.util.spec_from_file_location("build_release_runtime", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_scrapling_dependency_layer_is_deterministic_and_regular_only(tmp_path: Path) -> None:
    site = tmp_path / ".venv/lib/python3.12/site-packages"
    (site / "dependency").mkdir(parents=True)
    (site / "dependency/__init__.py").write_text("value = 1\n")
    (site / "dependency/native.so").write_bytes(b"native")
    (site / "dependency/__pycache__").mkdir()
    (site / "dependency/__pycache__/ignored.pyc").write_bytes(b"unstable")
    first = tmp_path / "first.tar.gz"
    second = tmp_path / "second.tar.gz"

    MODULE.build(tmp_path / ".venv", first)
    MODULE.build(tmp_path / ".venv", second)

    assert first.read_bytes() == second.read_bytes()
    with tarfile.open(first, "r:gz") as archive:
        assert [member.name for member in archive.getmembers()] == [
            "dependency/__init__.py",
            "dependency/native.so",
        ]
        assert all(member.isfile() and member.mtime == 0 for member in archive.getmembers())
