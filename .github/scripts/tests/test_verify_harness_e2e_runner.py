from __future__ import annotations

import hashlib
import io
import json
import tarfile
from pathlib import Path

import pytest

from verify_harness_e2e_runner import VerificationError, verify_runner


def bundle(tmp_path: Path, *, source_sha: str = "a" * 40) -> tuple[Path, Path]:
    archive = tmp_path / "harness-e2e-runner.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        contents = b"runner"
        member = tarfile.TarInfo("harness-e2e")
        member.mode = 0o755
        member.size = len(contents)
        tar.addfile(member, io.BytesIO(contents))
    metadata = tmp_path / "harness-e2e-runner.json"
    metadata.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "source_sha": source_sha,
                "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                "runner_version": "1.8.0",
                "scenario_ids": ["direct_answer"],
            }
        )
    )
    return metadata, archive


def test_accepts_exact_runner_bundle(tmp_path: Path) -> None:
    metadata, archive = bundle(tmp_path)

    result = verify_runner(
        metadata_path=metadata, archive_path=archive, source_sha="a" * 40
    )

    assert result["runner_version"] == "1.8.0"


def test_rejects_wrong_source_sha(tmp_path: Path) -> None:
    metadata, archive = bundle(tmp_path)

    with pytest.raises(VerificationError, match="source SHA"):
        verify_runner(
            metadata_path=metadata, archive_path=archive, source_sha="b" * 40
        )


def test_rejects_tampered_archive(tmp_path: Path) -> None:
    metadata, archive = bundle(tmp_path)
    archive.write_bytes(archive.read_bytes() + b"tampered")

    with pytest.raises(VerificationError, match="checksum"):
        verify_runner(
            metadata_path=metadata, archive_path=archive, source_sha="a" * 40
        )
