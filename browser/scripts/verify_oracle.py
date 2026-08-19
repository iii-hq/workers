#!/usr/bin/env python3
"""Write or verify the frozen standalone-worker oracle manifest."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import locale
import os
import re
import subprocess
import sys
from pathlib import Path


WORKER = Path(__file__).resolve().parent.parent
REPO = WORKER.parent
MANIFEST = WORKER / "oracle/manifest.json"
LOCK = WORKER / "oracle/requirements.lock"
ASSET_SUFFIXES = {".dat", ".json", ".pem", ".txt", ".xz", ".zip"}
BROWSERS = (
    (
        "chromium-linux-x64",
        "pw-chromium-1223-linux-x64.zip",
        "https://cdn.playwright.dev/builds/cft/148.0.7778.96/linux64/chrome-linux64.zip",
    ),
    (
        "chromium-headless-shell-linux-x64",
        "pw-headless-1223-linux-x64.zip",
        "https://cdn.playwright.dev/builds/cft/148.0.7778.96/linux64/chrome-headless-shell-linux64.zip",
    ),
    (
        "ffmpeg-linux-x64",
        "pw-ffmpeg-1011-linux-x64.zip",
        "https://cdn.playwright.dev/dbazure/download/playwright/builds/ffmpeg/1011/ffmpeg-linux.zip",
    ),
    (
        "chromium-linux-arm64",
        "pw-chromium-1223-linux-arm64.zip",
        "https://cdn.playwright.dev/dbazure/download/playwright/builds/chromium/1223/chromium-linux-arm64.zip",
    ),
    (
        "chromium-headless-shell-linux-arm64",
        "pw-headless-1223-linux-arm64.zip",
        "https://cdn.playwright.dev/dbazure/download/playwright/builds/chromium/1223/chromium-headless-shell-linux-arm64.zip",
    ),
    (
        "ffmpeg-linux-arm64",
        "pw-ffmpeg-1011-linux-arm64.zip",
        "https://cdn.playwright.dev/dbazure/download/playwright/builds/ffmpeg/1011/ffmpeg-linux-arm64.zip",
    ),
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_record(path: Path, name: str | None = None) -> dict[str, object]:
    return {"path": name or str(path), "size": path.stat().st_size, "sha256": sha256(path)}


def records_digest(records: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for record in records:
        digest.update(str(record["path"]).encode())
        digest.update(b"\0")
        digest.update(str(record["sha256"]).encode())
        digest.update(b"\0")
    return digest.hexdigest()


def source_version() -> str:
    # The CI bot bumps this after every scrapling merge; reading it keeps the
    # recorded version from silently lying (pyproject.toml is itself one of
    # the fingerprinted files, so a bump already forces a re-freeze).
    pyproject = (REPO / "scrapling/pyproject.toml").read_text()
    match = re.search(r'(?m)^version = "([^"]+)"$', pyproject)
    if not match:
        raise SystemExit("cannot read version from scrapling/pyproject.toml")
    return match.group(1)


def source_manifest() -> dict[str, object]:
    output = subprocess.check_output(
        ["git", "ls-files", "-z", "scrapling"], cwd=REPO
    )
    paths = [Path(item.decode()) for item in output.split(b"\0") if item]
    files = [file_record(REPO / path, str(path)) for path in paths]
    return {"version": source_version(), "sha256": records_digest(files), "files": files}


def canonical_name(value: str) -> str:
    return re.sub(r"[-_.]+", "-", value).lower()


def package_manifest() -> tuple[list[dict[str, object]], list[dict[str, object]], str]:
    packages = []
    assets = []
    runtime_digest = hashlib.sha256()
    for distribution in sorted(
        importlib.metadata.distributions(),
        key=lambda item: canonical_name(item.metadata["Name"]),
    ):
        name = canonical_name(distribution.metadata["Name"])
        files = []
        for relative in sorted(distribution.files or (), key=str):
            path = Path(distribution.locate_file(relative))
            if not path.is_file():
                continue
            record = file_record(path, str(relative))
            files.append(record)
            if path.suffix.lower() in ASSET_SUFFIXES:
                assets.append({"package": name, **record})
            relative_name = str(relative)
            if not relative_name.startswith("../../../bin/") and not relative_name.endswith(
                ".dist-info/RECORD"
            ):
                for value in (name, relative_name, str(record["sha256"])):
                    runtime_digest.update(value.encode())
                    runtime_digest.update(b"\0")
        packages.append(
            {
                "name": name,
                "version": distribution.version,
                "files": len(files),
                "bytes": sum(int(item["size"]) for item in files),
                "sha256": records_digest(files),
            }
        )
    return packages, assets, runtime_digest.hexdigest()


def font_manifest() -> list[dict[str, object]]:
    output = subprocess.check_output(["fc-list", "--format=%{file}\n"], text=True)
    paths = sorted({Path(item) for item in output.splitlines() if item})
    return [file_record(path) for path in paths]


def timezone_manifest() -> dict[str, object]:
    path = Path("/etc/localtime").resolve()
    prefix = Path("/usr/share/zoneinfo")
    try:
        name = str(path.relative_to(prefix))
    except ValueError:
        name = os.environ.get("TZ", str(path))
    return {"name": name, **file_record(path)}


def browser_manifest(archive_dir: Path) -> list[dict[str, object]]:
    records = []
    for name, filename, url in BROWSERS:
        path = archive_dir / filename
        if not path.is_file():
            raise SystemExit(f"missing browser oracle archive: {path}")
        records.append({"name": name, "url": url, **file_record(path, filename)})
    return records


def snapshot(archive_dir: Path) -> dict[str, object]:
    packages, assets, parser_runtime_sha256 = package_manifest()
    fonts = font_manifest()
    certifi = importlib.import_module("certifi")
    executable = Path(sys.executable).resolve()
    return {
        "format": 1,
        "source": source_manifest(),
        "python": {
            "version": sys.version.split()[0],
            "implementation": sys.implementation.name,
            "executable": file_record(executable),
            "parser_runtime_sha256": parser_runtime_sha256,
            "packages": packages,
            "requirements_lock": file_record(LOCK, "oracle/requirements.lock"),
        },
        "browser": {
            "playwright_revision": "1223",
            "chromium_version": "148.0.7778.96",
            "archives": browser_manifest(archive_dir),
        },
        "assets": assets,
        "host": {
            "locale": locale.setlocale(locale.LC_ALL, ""),
            "locale_environment": {
                key: os.environ.get(key, "")
                for key in ("LANG", "LC_ALL", "LC_CTYPE")
            },
            "timezone": timezone_manifest(),
            "ca_bundle": file_record(Path(certifi.where()), "certifi/cacert.pem"),
            "fonts_sha256": records_digest(fonts),
            "fonts": fonts,
        },
        "determinism": {"PYTHONHASHSEED": "0", "random_seed": 0},
    }


def verify_archives(expected: dict[str, object], archive_dir: Path) -> None:
    actual = browser_manifest(archive_dir)
    if actual != expected["browser"]["archives"]:
        raise SystemExit("browser oracle archives differ from oracle/manifest.json")


def verify(archive_dir: Path | None) -> None:
    expected = json.loads(MANIFEST.read_text())
    # Archive bytes are release inputs, not required to regenerate parse-only
    # goldens. Reuse the frozen entries while comparing everything local.
    current = snapshot(archive_dir or Path("/nonexistent")) if archive_dir else None
    if current is None:
        packages, assets, parser_runtime_sha256 = package_manifest()
        fonts = font_manifest()
        certifi = importlib.import_module("certifi")
        executable = Path(sys.executable).resolve()
        current = {
            **expected,
            "source": source_manifest(),
            "python": {
                "version": sys.version.split()[0],
                "implementation": sys.implementation.name,
                "executable": file_record(executable),
                "parser_runtime_sha256": parser_runtime_sha256,
                "packages": packages,
                "requirements_lock": file_record(LOCK, "oracle/requirements.lock"),
            },
            "assets": assets,
            "host": {
                "locale": locale.setlocale(locale.LC_ALL, ""),
                "locale_environment": {
                    key: os.environ.get(key, "")
                    for key in ("LANG", "LC_ALL", "LC_CTYPE")
                },
                "timezone": timezone_manifest(),
                "ca_bundle": file_record(Path(certifi.where()), "certifi/cacert.pem"),
                "fonts_sha256": records_digest(fonts),
                "fonts": fonts,
            },
        }
    if current != expected:
        raise SystemExit("oracle environment differs from oracle/manifest.json")
    if archive_dir:
        verify_archives(expected, archive_dir)
    print("oracle environment verified")


def verify_parser_runtime() -> None:
    """Verify inputs that can affect parse differentials, excluding host/browser data."""
    expected = json.loads(MANIFEST.read_text())
    packages, assets, parser_runtime_sha256 = package_manifest()
    current = {
        "source": source_manifest(),
        "python": {
            "version": sys.version.split()[0],
            "implementation": sys.implementation.name,
            "parser_runtime_sha256": parser_runtime_sha256,
            "packages": [
                {"name": item["name"], "version": item["version"]}
                for item in packages
            ],
            "requirements_lock": file_record(LOCK, "oracle/requirements.lock"),
        },
        "assets": assets,
    }
    frozen = {
        "source": expected["source"],
        "python": {
            "version": expected["python"]["version"],
            "implementation": expected["python"]["implementation"],
            "parser_runtime_sha256": expected["python"]["parser_runtime_sha256"],
            "packages": [
                {"name": item["name"], "version": item["version"]}
                for item in expected["python"]["packages"]
            ],
            "requirements_lock": expected["python"]["requirements_lock"],
        },
        "assets": expected["assets"],
    }
    if current != frozen:
        raise SystemExit("parser oracle runtime differs from oracle/manifest.json")
    print("parser oracle runtime verified")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--archive-dir", type=Path)
    parser.add_argument("--parser-runtime", action="store_true")
    args = parser.parse_args()
    if args.write:
        if not args.archive_dir:
            parser.error("--write requires --archive-dir")
        MANIFEST.write_text(json.dumps(snapshot(args.archive_dir), indent=2) + "\n")
        print(f"wrote {MANIFEST}")
    elif args.parser_runtime:
        if args.archive_dir:
            parser.error("--parser-runtime does not accept --archive-dir")
        verify_parser_runtime()
    else:
        verify(args.archive_dir)


if __name__ == "__main__":
    main()
