from __future__ import annotations

import json
from pathlib import Path
import stat
import subprocess


SCRIPT = Path(__file__).parents[1] / "harness_stack_bundle.sh"
STACK_BINARIES = [
    "queue",
    "iii-directory",
    "session-manager",
    "context-manager",
    "cron",
    "state",
    "database",
    "harness",
    "harness-integration",
    "console",
]
SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"


def executable(path: Path, contents: str) -> None:
    path.write_text(contents)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def build_bundle(tmp_path: Path) -> Path:
    engine = tmp_path / "iii"
    bin_dir = tmp_path / "release"
    bin_dir.mkdir()
    executable(engine, "engine")
    for binary in STACK_BINARIES:
        executable(bin_dir / binary, binary)

    archive = tmp_path / "harness-stack.tar.zst"
    subprocess.run(
        [
            SCRIPT,
            "pack",
            "--output",
            archive,
            "--source-sha",
            SOURCE_SHA,
            "--engine-version",
            "iii 0.9.0-rc.1",
            "--engine-bin",
            engine,
            "--bin-dir",
            bin_dir,
        ],
        check=True,
        text=True,
    )
    return archive


def test_bundle_round_trip_preserves_identity_and_executables(tmp_path: Path) -> None:
    archive = build_bundle(tmp_path)
    destination = tmp_path / "unpacked"

    subprocess.run(
        [
            SCRIPT,
            "unpack",
            "--archive",
            archive,
            "--destination",
            destination,
            "--expected-source-sha",
            SOURCE_SHA,
        ],
        check=True,
        text=True,
    )

    manifest = json.loads((destination / "manifest.json").read_text())
    assert manifest["schema"] == "harness-stack-bundle/v1"
    assert manifest["source_sha"] == SOURCE_SHA
    assert manifest["engine_version"] == "iii 0.9.0-rc.1"
    assert manifest["binaries"] == ["iii", *STACK_BINARIES]
    for binary in manifest["binaries"]:
        path = destination / "bin" / binary
        assert path.read_text() == ("engine" if binary == "iii" else binary)
        assert path.stat().st_mode & stat.S_IXUSR


def test_bundle_rejects_a_different_source_revision(tmp_path: Path) -> None:
    archive = build_bundle(tmp_path)
    result = subprocess.run(
        [
            SCRIPT,
            "unpack",
            "--archive",
            archive,
            "--destination",
            tmp_path / "unpacked",
            "--expected-source-sha",
            "f" * 40,
        ],
        check=False,
        text=True,
        capture_output=True,
    )

    assert result.returncode == 3
    assert "bundle source SHA does not match" in result.stderr


def test_bundle_rejects_modified_binary_contents(tmp_path: Path) -> None:
    archive = build_bundle(tmp_path)
    altered = tmp_path / "altered"
    altered.mkdir()
    subprocess.run(
        ["tar", "--zstd", "-xf", archive, "-C", altered],
        check=True,
    )
    (altered / "bin" / "harness").write_text("modified")
    tampered_archive = tmp_path / "tampered.tar.zst"
    subprocess.run(
        ["tar", "--zstd", "-cf", tampered_archive, "-C", altered, "."],
        check=True,
    )

    result = subprocess.run(
        [
            SCRIPT,
            "unpack",
            "--archive",
            tampered_archive,
            "--destination",
            tmp_path / "tampered-unpacked",
            "--expected-source-sha",
            SOURCE_SHA,
        ],
        check=False,
        text=True,
        capture_output=True,
    )

    assert result.returncode != 0
    assert "FAILED" in result.stdout
