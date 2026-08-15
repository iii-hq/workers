from __future__ import annotations

from pathlib import Path

import yaml


WORKFLOWS = Path(__file__).parents[2] / "workflows"


def test_cache_cleanup_uses_get_and_propagates_list_failures() -> None:
    workflow = yaml.load(
        (WORKFLOWS / "cache-cleanup.yml").read_text(),
        Loader=yaml.BaseLoader,
    )
    run = workflow["jobs"]["cleanup"]["steps"][0]["run"]

    assert 'gh api --method GET "repos/$REPOSITORY/actions/caches"' in run
    assert 'cache_ids="$(' in run
    assert 'mapfile -t ids <<<"$cache_ids"' in run
    assert '< <(gh api' not in run
