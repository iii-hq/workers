from __future__ import annotations

import os
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
    fake_curl(tmp_path, mitigation)
    env = os.environ.copy()
    env["FAKE_VERCEL_MITIGATION"] = mitigation
    env["PATH"] = f"{tmp_path}:{env['PATH']}"
    return subprocess.run(
        [str(CHECK_LINKS)],
        cwd=ROOT,
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )


def test_vercel_security_challenge_is_reachable(tmp_path: Path) -> None:
    result = run_check(tmp_path, "challenge")

    assert result.returncode == 0, result.stdout + result.stderr
    assert "Vercel security challenge" in result.stdout


def test_vercel_deny_remains_a_failure(tmp_path: Path) -> None:
    result = run_check(tmp_path, "deny")

    assert result.returncode == 1
    assert "FAIL 403" in result.stdout
