#!/usr/bin/env python3
"""Declare provider credentials on llm-router without materializing secrets."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


ROUTER_HEADER = "  llm-router:"
CREDENTIALS = ("ANTHROPIC_API_KEY", "OPENAI_API_KEY")
CONTAINER_HEADER = re.compile(r"^  [^ #][^:]*:\s*(?:#.*)?$")


def configure_compose_environment(text: str) -> str:
    """Return compose YAML with literal provider placeholders on llm-router."""

    trailing_newline = text.endswith("\n")
    lines = text.splitlines()
    router_headers = [index for index, line in enumerate(lines) if line == ROUTER_HEADER]
    if len(router_headers) != 1:
        raise ValueError(f"expected exactly one llm-router container, found {len(router_headers)}")

    router_start = router_headers[0]
    router_end = next(
        (index for index in range(router_start + 1, len(lines)) if CONTAINER_HEADER.match(lines[index])),
        len(lines),
    )

    for credential in CREDENTIALS:
        declaration = re.compile(rf"^\s+{re.escape(credential)}\s*:")
        for index, line in enumerate(lines):
            if declaration.match(line) and not router_start < index < router_end:
                raise ValueError(f"{credential} must only be declared on llm-router")

    environment_headers = [
        index
        for index in range(router_start + 1, router_end)
        if lines[index].startswith("    environment:")
    ]
    if len(environment_headers) > 1:
        raise ValueError("llm-router has multiple environment sections")

    expected = {credential: f"      {credential}: ${{{credential}}}" for credential in CREDENTIALS}
    missing: list[str] = []
    for credential in CREDENTIALS:
        declarations = [
            lines[index]
            for index in range(router_start + 1, router_end)
            if re.match(rf"^\s+{re.escape(credential)}\s*:", lines[index])
        ]
        if len(declarations) > 1:
            raise ValueError(f"llm-router declares {credential} more than once")
        if declarations and declarations[0] != expected[credential]:
            raise ValueError(f"llm-router must reference {credential} through its literal placeholder")
        if not declarations:
            missing.append(credential)

    if not missing:
        return text

    if environment_headers:
        environment_index = environment_headers[0]
        if lines[environment_index] != "    environment:":
            raise ValueError("llm-router environment must use a YAML mapping")
        insert_at = environment_index + 1
    else:
        insert_at = next(
            (
                index
                for index in range(router_start + 1, router_end)
                if lines[index].startswith("    start_after:")
            ),
            router_end,
        )
        lines.insert(insert_at, "    environment:")
        insert_at += 1

    lines[insert_at:insert_at] = [expected[credential] for credential in missing]
    configured = "\n".join(lines)
    return configured + ("\n" if trailing_newline else "")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("compose_file", type=Path)
    args = parser.parse_args()

    original = args.compose_file.read_text(encoding="utf-8")
    configured = configure_compose_environment(original)
    if configured != original:
        args.compose_file.write_text(configured, encoding="utf-8")


if __name__ == "__main__":
    main()
