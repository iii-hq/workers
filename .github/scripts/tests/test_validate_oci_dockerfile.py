from pathlib import Path

import pytest

from validate_oci_dockerfile import validate

REPO_ROOT = Path(__file__).resolve().parents[3]


def test_release_oci_dockerfile_has_only_immutable_inputs() -> None:
    for worker in ("hermes", "scrapling"):
        validate(REPO_ROOT / worker / "Dockerfile")


@pytest.mark.parametrize(
    "dockerfile, message",
    [
        ("FROM python:3.11-slim\n", "pinned by sha256"),
        (
            "FROM scratch\nRUN curl -fsSL https://example.test/install.sh | bash\n",
            "network installer pipe",
        ),
        (
            "FROM scratch\nADD https://example.test/source.tar.gz /tmp/source.tar.gz\n",
            "remote ADD must declare",
        ),
        ("FROM scratch\nRUN uv sync\n", "uv sync must use"),
    ],
)
def test_mutable_oci_inputs_fail_closed(tmp_path: Path, dockerfile: str, message: str) -> None:
    path = tmp_path / "Dockerfile"
    path.write_text(dockerfile, encoding="utf-8")

    with pytest.raises(SystemExit, match=message):
        validate(path)
