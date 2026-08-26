from __future__ import annotations

import json
import sys

import build_skills_payload


def test_empty_worker_emits_explicit_empty_skills_snapshot(tmp_path, monkeypatch) -> None:
    worker = tmp_path / "smoke"
    worker.mkdir()
    output = tmp_path / "skills.json"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "build_skills_payload.py",
            "--worker",
            "smoke",
            "--version",
            "1.2.3",
            "--repo-root",
            str(tmp_path),
            "--out",
            str(output),
        ],
    )

    assert build_skills_payload.main() == 0
    assert json.loads(output.read_text()) == {"version": "1.2.3", "skills": {}}


def test_worker_emits_complete_nonempty_snapshot(tmp_path, monkeypatch) -> None:
    skills = tmp_path / "smoke" / "skills"
    nested = skills / "usage"
    nested.mkdir(parents=True)
    (skills / "SKILL.md").write_text("# Smoke\n")
    (nested / "run.md").write_text("Run it\n")
    output = tmp_path / "skills.json"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "build_skills_payload.py",
            "--worker",
            "smoke",
            "--version",
            "1.2.3",
            "--repo-root",
            str(tmp_path),
            "--out",
            str(output),
        ],
    )

    assert build_skills_payload.main() == 0
    assert json.loads(output.read_text()) == {
        "version": "1.2.3",
        "skills": {"SKILL.md": "# Smoke\n", "skills/usage/run.md": "Run it\n"},
    }
