from __future__ import annotations

import pytest

import find_harness_e2e_runner as runner_lookup


def test_finds_exact_non_expired_artifact(monkeypatch: pytest.MonkeyPatch) -> None:
    source_sha = "a" * 40

    def fake_api(url: str, _token: str) -> dict:
        if "/workflows/" in url:
            return {
                "workflow_runs": [
                    {"id": 41, "created_at": "2026-08-07T10:00:00Z"}
                ]
            }
        return {
            "artifacts": [
                {
                    "id": 99,
                    "name": f"harness-e2e-runner-{source_sha}",
                    "expired": False,
                }
            ]
        }

    monkeypatch.setattr(runner_lookup, "api_json", fake_api)

    result = runner_lookup.find_runner(
        api_url="https://api.github.test",
        repository="iii-hq/workers",
        workflow="harness-e2e-main.yml",
        source_sha=source_sha,
        token="secret",
    )

    assert result == {
        "run_id": 41,
        "artifact_id": 99,
        "artifact_name": f"harness-e2e-runner-{source_sha}",
    }


def test_rejects_expired_or_different_artifacts(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source_sha = "b" * 40

    def fake_api(url: str, _token: str) -> dict:
        if "/workflows/" in url:
            return {"workflow_runs": [{"id": 42, "created_at": "now"}]}
        return {
            "artifacts": [
                {
                    "id": 100,
                    "name": f"harness-e2e-runner-{source_sha}",
                    "expired": True,
                },
                {"id": 101, "name": "different", "expired": False},
            ]
        }

    monkeypatch.setattr(runner_lookup, "api_json", fake_api)

    with pytest.raises(runner_lookup.LookupError, match="will not compile"):
        runner_lookup.find_runner(
            api_url="https://api.github.test",
            repository="iii-hq/workers",
            workflow="harness-e2e-main.yml",
            source_sha=source_sha,
            token="secret",
        )
