import importlib.util
from pathlib import Path

import pytest
import yaml


ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "harness" / "tests" / "quickstart" / "configure_compose_environment.py"
SPEC = importlib.util.spec_from_file_location("configure_compose_environment", HELPER)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


COMPOSE = """\
namespace: default
containers:
  llm-router:
    worker: package://llm-router
    version: "1.4.14"
    start_after:
      - state
  harness:
    worker: package://harness
    version: "0.8.1"
"""


def test_declares_literal_credentials_only_on_llm_router():
    configured = MODULE.configure_compose_environment(COMPOSE)
    document = yaml.safe_load(configured)

    assert document["containers"]["llm-router"]["environment"] == {
        "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}",
        "OPENAI_API_KEY": "${OPENAI_API_KEY}",
    }
    assert "environment" not in document["containers"]["harness"]
    assert configured.count("ANTHROPIC_API_KEY") == 2
    assert configured.count("OPENAI_API_KEY") == 2


def test_is_idempotent_and_preserves_other_router_environment():
    original = COMPOSE.replace(
        "    start_after:\n",
        "    environment:\n"
        "      ROUTER_LOG: info\n"
        "      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}\n"
        "      OPENAI_API_KEY: ${OPENAI_API_KEY}\n"
        "    start_after:\n",
    )

    assert MODULE.configure_compose_environment(original) == original


@pytest.mark.parametrize(
    "compose, message",
    [
        (COMPOSE.replace("  llm-router:\n", ""), "exactly one llm-router"),
        (
            COMPOSE.replace(
                "  harness:\n",
                "  harness:\n    environment:\n      OPENAI_API_KEY: ${OPENAI_API_KEY}\n",
            ),
            "must only be declared on llm-router",
        ),
        (
            COMPOSE.replace(
                "    start_after:\n",
                "    environment:\n      OPENAI_API_KEY: materialized-secret\n    start_after:\n",
            ),
            "literal placeholder",
        ),
    ],
)
def test_rejects_ambiguous_or_secret_materializing_compose(compose: str, message: str):
    with pytest.raises(ValueError, match=message):
        MODULE.configure_compose_environment(compose)
