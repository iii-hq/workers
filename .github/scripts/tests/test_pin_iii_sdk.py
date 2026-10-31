"""Tests for .github/scripts/pin_iii_sdk.py (scripts/pin-iii-sdk.sh)."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pin_iii_sdk as pin
import pytest
from _test_helpers import GIT_HERMETIC_ENV

ROOT = Path(__file__).resolve().parents[3]

FIXTURE = {
    "w/Cargo.toml": (
        '[dependencies]\n'
        'iii-sdk = "=0.26.0"\n'
        'iii-helpers = { version = "=0.26.0", features = ["otel"] }\n'
        'iii-console-ui = { path = "../crates/console-ui" }\n'
        'serde = "1"\n'
    ),
    "w/package.json": json.dumps({
        "name": "w",
        "dependencies": {"iii-sdk": "0.26.0", "@iii-dev/console-ui": "workspace:*", "@iii-dev/helpers": "^0.26.0"},
    }, indent=2) + "\n",
    "pnpm-workspace.yaml": (
        "minimumReleaseAgeExclude:\n  - '@pierre/trees@1.0.0-beta.6'\n  - 'iii-sdk@0.26.0'\n  - iii-browser-sdk@0.26.0\n"
        "  - '@iii-dev/helpers@0.26.0-rc.1 || 0.26.0'\n"
    ),
    "py/pyproject.toml": '[project]\nname = "py"\ndependencies = [\n    "iii-sdk>=0.26.0",\n    "iii-helpers[otel]==0.26.0",\n]\n',
    "go/go.mod": "module example.com/go\n\ngo 1.25.0\n\nrequire github.com/iii-hq/iii/sdk/packages/go/iii v0.26.0\n",
    ".github/workflows/ci.yml": (
        "env:\n  III_RELEASE_TAG: 'iii/v0.26.0'\njobs:\n  e2e:\n    strategy:\n      matrix:\n"
        "        engine:\n          - 'iii/v0.26.0'\n          - 'iii/v0.25.0'\n"
    ),
    "h/Dockerfile": (
        "RUN case \"${TARGETARCH}\" in \\\n"
        f"      amd64) iii_target=x86_64-unknown-linux-gnu; iii_sha={'a' * 64} ;; \\\n"
        f"      arm64) iii_target=aarch64-unknown-linux-gnu; iii_sha={'b' * 64} ;; \\\n"
        "    && iii_url=\"https://github.com/iii-hq/iii/releases/download/iii/v0.23.0-rc.7/${iii_archive}\"\n"
    ),
    "g/src/lib.rs": 'const GUEST: &str = "iii-sdk>=0.26.0";\nconst PKG: &str = "{ \\"iii-sdk\\": \\"^0.26.0\\" }";\n',
}


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    for name, text in FIXTURE.items():
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True, env=GIT_HERMETIC_ENV)
    subprocess.run(["git", "-C", str(tmp_path), "add", "-A"], check=True, env=GIT_HERMETIC_ENV)
    return tmp_path


def planned(repo: Path, version: str) -> dict[str, str]:
    files = pin.git_files(repo)
    current = set(pin.current_versions(repo, files))
    digests = {"x86_64-unknown-linux-gnu": "1" * 64, "aarch64-unknown-linux-gnu": "2" * 64}
    edits = pin.plan_edits(repo, files, pin.Target.parse(version), current, set(pin.GROUPS), digests.__getitem__)
    return {e.path: e.after for e in edits}


def test_target_spellings() -> None:
    assert pin.Target.parse("0.26.0-rc.1") == pin.Target("0.26.0-rc.1", "0.26.0rc1")
    assert pin.Target.parse("v0.26.0") == pin.Target("0.26.0", "0.26.0")
    assert pin.semver_of_pep440("0.26.0rc1") == "0.26.0-rc.1"
    with pytest.raises(ValueError):
        pin.Target.parse("0.26")


def test_every_pin_form_moves_to_the_release(repo: Path) -> None:
    after = planned(repo, "0.26.0-rc.1")
    assert 'iii-sdk = "=0.26.0-rc.1"' in after["w/Cargo.toml"]
    assert 'iii-helpers = { version = "=0.26.0-rc.1", features = ["otel"] }' in after["w/Cargo.toml"]
    assert 'serde = "1"' in after["w/Cargo.toml"]
    package = json.loads(after["w/package.json"])["dependencies"]
    assert package == {"iii-sdk": "0.26.0-rc.1", "@iii-dev/console-ui": "workspace:*", "@iii-dev/helpers": "^0.26.0-rc.1"}
    assert "'@pierre/trees@1.0.0-beta.6'" in after["pnpm-workspace.yaml"]
    assert "'iii-sdk@0.26.0-rc.1'" in after["pnpm-workspace.yaml"]
    assert "- iii-browser-sdk@0.26.0-rc.1\n" in after["pnpm-workspace.yaml"]
    assert "- '@iii-dev/helpers@0.26.0-rc.1'\n" in after["pnpm-workspace.yaml"]  # pnpm writes `a || b` itself
    assert '"iii-sdk>=0.26.0rc1"' in after["py/pyproject.toml"]
    assert '"iii-helpers[otel]==0.26.0rc1"' in after["py/pyproject.toml"]
    assert "iii v0.26.0-rc.1\n" in after["go/go.mod"]
    workflow = after[".github/workflows/ci.yml"]
    assert "III_RELEASE_TAG: 'iii/v0.26.0-rc.1'" in workflow
    assert "- 'iii/v0.26.0-rc.1'" in workflow and "- 'iii/v0.25.0'" in workflow
    dockerfile = after["h/Dockerfile"]
    assert f"iii_sha={'1' * 64}" in dockerfile and f"iii_sha={'2' * 64}" in dockerfile
    assert "/releases/download/iii/v0.26.0-rc.1/" in dockerfile
    assert '"iii-sdk>=0.26.0rc1"' in after["g/src/lib.rs"]
    assert '\\"iii-sdk\\": \\"^0.26.0-rc.1\\"' in after["g/src/lib.rs"]


def test_rc_pins_move_on_to_the_stable_release(repo: Path) -> None:
    for path, text in planned(repo, "0.26.0-rc.1").items():
        (repo / path).write_text(text)
    after = planned(repo, "0.26.0")
    assert 'iii-sdk = "=0.26.0"' in after["w/Cargo.toml"]
    assert '"iii-sdk>=0.26.0"' in after["py/pyproject.toml"]
    assert "III_RELEASE_TAG: 'iii/v0.26.0'" in after[".github/workflows/ci.yml"]
    assert '\\"iii-sdk\\": \\"^0.26.0\\"' in after["g/src/lib.rs"]


def test_mixed_pins_stop_the_run(repo: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (repo / "go/go.mod").write_text(FIXTURE["go/go.mod"].replace("v0.26.0", "v0.25.0"))
    assert pin.main(["--root", str(repo), "0.26.0-rc.1", "--dry-run"]) == 1
    assert "more than one SDK version" in capsys.readouterr().err


def test_dry_run_writes_nothing(repo: Path) -> None:
    before = {p: (repo / p).read_text() for p in FIXTURE}
    assert pin.main(["--root", str(repo), "0.26.0-rc.1", "--dry-run", "--only", "rust,node,python,go"]) == 0
    assert before == {p: (repo / p).read_text() for p in FIXTURE}


def test_lock_readers(tmp_path: Path) -> None:
    pnpm = tmp_path / "pnpm-lock.yaml"
    pnpm.write_text("packages:\n\n  iii-sdk@0.26.0-rc.1:\n    resolution: {}\n\n  '@iii-dev/helpers@0.26.0-rc.1':\n"
                    "    resolution: {}\n\nsnapshots:\n\n  iii-sdk@0.26.0-rc.1(zod@4.0.0):\n    dependencies: {}\n")
    assert pin.node_lock_versions(pnpm) == {"iii-sdk": {"0.26.0-rc.1"}, "@iii-dev/helpers": {"0.26.0-rc.1"}}
    npm = tmp_path / "package-lock.json"
    npm.write_text(json.dumps({"packages": {"": {"dependencies": {"iii-sdk": "0.26.0"}},
                                            "node_modules/iii-sdk": {"version": "0.26.0"}}}))
    assert pin.node_lock_versions(npm) == {"iii-sdk": {"0.26.0"}}
    cargo = tmp_path / "Cargo.lock"
    cargo.write_text('[[package]]\nname = "iii-sdk"\nversion = "0.25.0"\nsource = "registry+x"\n\n'
                     '[[package]]\nname = "serde"\nversion = "1.0.0"\n')
    assert pin.lock_names(cargo, pin.RUST_CRATES) == {"iii-sdk": ["0.25.0"]}


def test_repository_pins_one_sdk_version_in_supported_forms() -> None:
    files = pin.git_files(ROOT)
    assert pin.unsupported_forms(ROOT, files) == []
    assert len(pin.current_versions(ROOT, files)) == 1


def test_kit_lists_its_workers() -> None:
    entries = pin.kit_entries((ROOT / pin.KIT).read_text())
    assert {"harness", "iii-directory", "state", "kanban", "ide"} <= set(entries)
