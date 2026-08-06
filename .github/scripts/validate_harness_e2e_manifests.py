#!/usr/bin/env python3
"""Fail fast when the Harness E2E source stack cannot build with locked manifests."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence


CORE_SOURCE_WORKERS = (
    "database",
    "state",
    "queue",
    "session-manager",
    "llm-router",
    "context-manager",
    "iii-directory",
    "cron",
    "web",
    "fp",
)
SAFE_PROVIDER = re.compile(r"^[A-Za-z0-9_-]+$")


class PreflightInputError(ValueError):
    """Raised when the requested E2E stack cannot be resolved safely."""


@dataclass(frozen=True)
class Component:
    name: str
    manifest: Path
    expected_binaries: tuple[str, ...]


@dataclass(frozen=True)
class ValidationFailure:
    component: Component
    message: str


def parse_subject_providers(raw: str) -> list[str]:
    try:
        subjects = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise PreflightInputError(f"subjects JSON is invalid: {exc}") from exc
    if not isinstance(subjects, list) or not subjects:
        raise PreflightInputError("subjects must be a non-empty JSON array")

    providers: list[str] = []
    for index, subject in enumerate(subjects):
        if not isinstance(subject, dict):
            raise PreflightInputError(f"subjects[{index}] must be an object")
        provider = subject.get("provider")
        if not isinstance(provider, str) or not SAFE_PROVIDER.fullmatch(provider):
            raise PreflightInputError(
                f"subjects[{index}].provider must match {SAFE_PROVIDER.pattern}"
            )
        providers.append(provider)
    return providers


def resolve_components(
    root: Path,
    *,
    stack_mode: str,
    subjects_json: str,
    judge_provider: str,
) -> list[Component]:
    if stack_mode not in {"source", "registry"}:
        raise PreflightInputError("stack mode must be source or registry")
    if not SAFE_PROVIDER.fullmatch(judge_provider):
        raise PreflightInputError(
            f"judge provider must match {SAFE_PROVIDER.pattern}"
        )

    components: list[Component] = []
    if stack_mode == "source":
        components.extend(
            Component(worker, root / worker / "Cargo.toml", (worker,))
            for worker in CORE_SOURCE_WORKERS
        )
        providers = sorted(set(parse_subject_providers(subjects_json)) | {judge_provider})
        components.extend(
            Component(
                f"provider-{provider}",
                root / f"provider-{provider}" / "Cargo.toml",
                (f"provider-{provider}",),
            )
            for provider in providers
        )

    # The root Harness manifest owns the runner workspace in both stack modes.
    components.append(
        Component(
            "harness",
            root / "harness" / "Cargo.toml",
            ("harness", "harness-e2e"),
        )
    )
    return components


def _metadata_binaries(metadata: dict[str, Any]) -> set[str]:
    packages = metadata.get("packages", [])
    if not isinstance(packages, list):
        return set()
    return {
        str(target.get("name"))
        for package in packages
        if isinstance(package, dict)
        for target in package.get("targets", [])
        if isinstance(target, dict)
        and isinstance(target.get("kind"), list)
        and "bin" in target["kind"]
        and target.get("name")
    }


def validate_component(
    component: Component,
    *,
    root: Path,
    cargo: str = "cargo",
) -> ValidationFailure | None:
    try:
        relative_manifest = component.manifest.relative_to(root)
    except ValueError:
        return ValidationFailure(component, "manifest resolves outside the repository")
    if not component.manifest.is_file():
        return ValidationFailure(component, f"missing manifest: {relative_manifest}")

    command = [
        cargo,
        "metadata",
        "--locked",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        str(relative_manifest),
    ]
    try:
        result = subprocess.run(
            command,
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        return ValidationFailure(component, f"cannot execute cargo metadata: {exc}")

    if result.returncode != 0:
        diagnostic = (result.stderr or result.stdout).strip()
        return ValidationFailure(
            component,
            diagnostic or f"cargo metadata exited with status {result.returncode}",
        )

    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        return ValidationFailure(component, f"cargo metadata returned invalid JSON: {exc}")
    if not isinstance(metadata, dict):
        return ValidationFailure(component, "cargo metadata did not return an object")

    available_binaries = _metadata_binaries(metadata)
    missing_binaries = sorted(set(component.expected_binaries) - available_binaries)
    if missing_binaries:
        return ValidationFailure(
            component,
            "missing expected binary target(s): " + ", ".join(missing_binaries),
        )
    return None


def validate_components(
    components: Sequence[Component],
    *,
    root: Path,
    cargo: str = "cargo",
) -> list[ValidationFailure]:
    failures = []
    for component in components:
        if failure := validate_component(component, root=root, cargo=cargo):
            failures.append(failure)
    return failures


def _command_value(value: str) -> str:
    return value.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def emit_annotations(failures: Sequence[ValidationFailure], *, root: Path) -> None:
    for failure in failures:
        try:
            manifest = failure.component.manifest.relative_to(root)
        except ValueError:
            manifest = failure.component.manifest
        print(
            "::error "
            f"title={_command_value(f'Invalid {failure.component.name} manifest')},"
            f"file={_command_value(str(manifest))}::"
            f"{_command_value(failure.message)}"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository-root", type=Path, default=Path.cwd())
    parser.add_argument("--stack-mode", choices=("source", "registry"), required=True)
    parser.add_argument("--subjects-json", required=True)
    parser.add_argument("--judge-provider", required=True)
    parser.add_argument("--cargo", default="cargo")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    root = args.repository_root.resolve()
    try:
        components = resolve_components(
            root,
            stack_mode=args.stack_mode,
            subjects_json=args.subjects_json,
            judge_provider=args.judge_provider,
        )
    except PreflightInputError as exc:
        print(f"::error title=Invalid Harness E2E stack::{_command_value(str(exc))}")
        return 2

    failures = validate_components(components, root=root, cargo=args.cargo)
    if failures:
        emit_annotations(failures, root=root)
        print(f"Harness E2E manifest preflight failed for {len(failures)} component(s).")
        return 1
    print(f"Validated {len(components)} locked Harness E2E manifest(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
