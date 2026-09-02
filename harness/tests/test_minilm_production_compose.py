#!/usr/bin/env python3
import unittest
from pathlib import Path

import yaml


HARNESS = Path(__file__).resolve().parents[1]
REPOSITORY = HARNESS.parent
COMPOSE = HARNESS / "worker-compose.minilm-production.yaml"

PROVIDER_CREDENTIALS = {
    "ANTHROPIC_API_KEY": "",
    "COMMAND_CODE_API_KEY": "",
    "DEEPSEEK_API_KEY": "",
    "GITHUB_COPILOT_OAUTH_TOKEN": "",
    "GITHUB_COPILOT_TOKEN": "",
    "LLAMACPP_API_KEY": "",
    "MOONSHOT_API_KEY": "",
    "OPENCODE_GO_API_KEY": "",
    "OPENAI_API_KEY": "",
    "OPENROUTER_API_KEY": "",
    "XAI_API_KEY": "",
    "ZAI_API_KEY": "",
}

LOCAL_CREDENTIAL_IMPORT_GUARDS = {
    "CLAUDE_CONFIG_DIR": "/nonexistent/iii-catalog-only/claude",
    "CODEX_HOME": "/nonexistent/iii-catalog-only/codex",
    "GITHUB_COPILOT_NO_LOCAL_IMPORT": "1",
}


def worker_directories():
    return {
        child.name
        for child in REPOSITORY.iterdir()
        if child.is_dir() and (child / "iii.worker.yaml").exists()
    }


class MiniLmProductionComposeTests(unittest.TestCase):
    def test_compose_enables_the_local_production_pipeline(self):
        source = COMPOSE.read_text()
        document = yaml.safe_load(COMPOSE.read_text())
        directory = document["containers"]["iii-directory"]

        self.assertEqual(document["namespace"], "minilm-production")
        self.assertIn("--features minilm-production", directory["scripts"]["run"])
        self.assertEqual(directory["config_name"], "iii-directory-minilm-production")
        self.assertEqual(directory["environment"]["ORT_PREFER_DYNAMIC_LINK"], "0")
        self.assertIn("${ORT_LIB_PATH:-$HOME/.cache/iii/", directory["scripts"]["run"])
        self.assertEqual(directory["config_override"]["function_search_mode"], "hybrid")
        self.assertEqual(
            directory["config_override"]["function_search_model_path"],
            "~/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf",
        )
        self.assertNotIn("/home/anderson", source)
        self.assertNotIn("III_CATALOG_ONLY", source)

        for worker_name in ("slack", "telegram-bot"):
            self.assertEqual(
                document["containers"][worker_name]["config_override"]["bot_token"],
                "collect:interface-collection-token",
            )

        for worker_name in ("tailscale", "computer"):
            self.assertIn(
                '--url "$III_URL"',
                document["containers"][worker_name]["scripts"]["run"],
            )

        scrapling = document["containers"]["scrapling"]
        self.assertEqual(scrapling["worker"], "path://../scrapling/src")
        self.assertEqual(scrapling["working_dir"], "../scrapling")

    def test_compose_contains_every_worker_available_in_the_repository(self):
        document = yaml.safe_load(COMPOSE.read_text())
        composed = set(document["containers"])

        self.assertEqual(len(composed), 70)
        self.assertEqual(composed, worker_directories())

    def test_catalog_workers_cannot_inherit_provider_credentials(self):
        containers = yaml.safe_load(COMPOSE.read_text())["containers"]
        catalog_workers = {
            "llm-router",
            *(name for name in containers if name.startswith("provider-")),
        }
        expected_environment = PROVIDER_CREDENTIALS | LOCAL_CREDENTIAL_IMPORT_GUARDS

        self.assertEqual(len(catalog_workers), 14)
        for worker_name in catalog_workers:
            with self.subTest(worker=worker_name):
                self.assertEqual(
                    containers[worker_name].get("environment"),
                    expected_environment,
                )


if __name__ == "__main__":
    unittest.main()
