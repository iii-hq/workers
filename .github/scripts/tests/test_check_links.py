from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
CHECK_LINKS = ROOT / "scripts" / "check-links.sh"


def fake_curl(tmp_path: Path, mitigation: str) -> Path:
    curl = tmp_path / "curl"
    curl.write_text(
        """#!/usr/bin/env bash
set -euo pipefail
headers_file=
while (( $# )); do
  if [[ "$1" == "-D" ]]; then
    headers_file=$2
    shift 2
  else
    shift
  fi
done
printf 'HTTP/2 403\\r\\nx-vercel-mitigated: %s\\r\\n\\r\\n' "$FAKE_VERCEL_MITIGATION" > "$headers_file"
printf '403'
""",
        encoding="utf-8",
    )
    curl.chmod(0o755)
    return curl


def run_check(tmp_path: Path, mitigation: str) -> subprocess.CompletedProcess[str]:
    # Exercise the real script against a small repo instead of rescanning the
    # entire monorepo once per mocked 403. Repository growth and filesystem
    # speed must not determine whether this HTTP classification test times out.
    project = tmp_path / "project"
    scripts = project / "scripts"
    scripts.mkdir(parents=True)
    check_links = scripts / "check-links.sh"
    shutil.copy2(CHECK_LINKS, check_links)
    (project / "README.md").write_text(
        "https://iii.dev\nhttps://workers.iii.dev\n", encoding="utf-8"
    )
    for ignored in ("node_modules", ".git", "target", "dist"):
        directory = project / ignored
        directory.mkdir()
        (directory / "ignored.md").write_text(
            "https://ignored.iii.dev\n", encoding="utf-8"
        )
    fake_curl(tmp_path, mitigation)
    env = os.environ.copy()
    env["FAKE_VERCEL_MITIGATION"] = mitigation
    env["PATH"] = f"{tmp_path}:{env['PATH']}"
    return subprocess.run(
        [str(check_links)],
        cwd=project,
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )


def test_vercel_security_challenge_is_reachable(tmp_path: Path) -> None:
    result = run_check(tmp_path, "challenge")

    assert result.returncode == 0, result.stdout + result.stderr
    assert "OK   403  https://iii.dev (Vercel security challenge)" in result.stdout
    assert "OK   403  https://workers.iii.dev (Vercel security challenge)" in result.stdout
    assert "ignored.iii.dev" not in result.stdout


def test_vercel_deny_remains_a_failure(tmp_path: Path) -> None:
    result = run_check(tmp_path, "deny")

    assert result.returncode == 1
    assert "FAIL 403  https://iii.dev" in result.stdout
    assert "FAIL 403  https://workers.iii.dev" in result.stdout
    assert "README.md" in result.stdout
    assert "ignored.iii.dev" not in result.stdout
