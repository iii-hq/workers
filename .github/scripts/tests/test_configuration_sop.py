"""docs/sops/configuration.md names the functions its reference workers define.

The SOP tells a worker author which function registers
`<worker>::on-config-change` and observes `configuration::on-updated`, and
cites reference workers for each reload tier. A reader who opens a cited
`src/configuration.rs` finds that function under the name the SOP gives it.
"""
from __future__ import annotations

import re
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[3]
SOP = REPOSITORY / "docs" / "sops" / "configuration.md"


def sop() -> str:
    return SOP.read_text(encoding="utf-8")


def config_observer_name(text: str) -> str:
    """The function the SOP says registers `<worker>::on-config-change`."""
    match = re.search(
        r"^- `([a-z_]+)` — register `<worker>::on-config-change`", text, re.MULTILINE
    )
    assert match, "the SOP names the function that registers <worker>::on-config-change"
    return match.group(1)


def worker_spellings(text: str) -> dict[str, str]:
    """`{worker: name}` for each worker the SOP says spells the function differently."""
    return dict(re.findall(r"`([a-z0-9-]+)`\s+names it `([a-z_]+)`", text))


def tier_references(text: str) -> dict[str, list[str]]:
    """`{tier: [worker, ...]}`: the `<worker>/src/configuration.rs` each tier header cites."""
    references: dict[str, list[str]] = {}
    for match in re.finditer(r"^\*\*Tier (\d) — .*$", text, re.MULTILINE):
        references[match.group(1)] = re.findall(
            r"\]\(\.\./\.\./([a-z0-9-]+)/src/configuration\.rs\)", match.group(0)
        )
    return references


def reload_table_workers(text: str) -> list[str]:
    """The workers listed in the "Reload tiers by worker" table."""
    section = text.split("### Reload tiers by worker", 1)[1].split("\n\n```", 1)[0]
    workers: list[str] = []
    for line in section.splitlines():
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 3 or cells[0] in {"Worker", ""} or set(cells[0]) <= {"-"}:
            continue
        workers.extend(re.findall(r"`([a-z0-9-]+)`", cells[0]))
    return workers


def defines(worker: str, name: str) -> bool:
    source = (REPOSITORY / worker / "src" / "configuration.rs").read_text(encoding="utf-8")
    return re.search(rf"^pub (?:async )?fn {re.escape(name)}\b", source, re.MULTILINE) is not None


def test_every_tier_reference_defines_the_config_observer_the_sop_names() -> None:
    text = sop()
    name = config_observer_name(text)
    spellings = worker_spellings(text)
    references = tier_references(text)

    assert set(references) == {"1", "2"}, references
    assert all(references.values()), references
    for tier, workers in references.items():
        for worker in workers:
            expected = spellings.get(worker, name)
            assert defines(worker, expected), (
                f"Tier {tier} cites {worker}/src/configuration.rs, which defines no "
                f"`pub fn {expected}`"
            )


def test_every_worker_in_the_reload_table_defines_the_config_observer() -> None:
    text = sop()
    name = config_observer_name(text)
    spellings = worker_spellings(text)
    workers = reload_table_workers(text)

    assert {"context-manager", "approval-gate", "session-manager"} <= set(workers), workers
    for worker in workers:
        expected = spellings.get(worker, name)
        assert defines(worker, expected), (
            f"the reload table lists {worker}, whose src/configuration.rs defines no "
            f"`pub fn {expected}`"
        )


def test_both_boot_orders_end_with_the_config_observer() -> None:
    text = sop()
    name = config_observer_name(text)
    boot_orders = [
        block for block in re.findall(r"```text\n(.*?)```", text, re.DOTALL)
        if "parse CLI" in block
    ]

    assert len(boot_orders) == 2, boot_orders
    for block in boot_orders:
        steps = [line for line in block.splitlines() if re.match(r"\d+\. ", line)]
        last = steps[-1].split(". ", 1)[1]
        assert last.startswith(name), last
