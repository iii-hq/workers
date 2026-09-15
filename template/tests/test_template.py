"""Structural checks for the local Harness template; no engine or credentials needed."""

from pathlib import Path
import hashlib
import json
import shlex
import subprocess
import tomllib
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
TEMPLATE = ROOT / "template"


def tracked_files(*paths):
    """Validate the deliverable, not additional local profiles or runtime files."""
    output = subprocess.check_output(["git", "ls-files", "-z", "--", *paths], cwd=ROOT)
    return [ROOT / name.decode() for name in output.split(b"\0") if name and (ROOT / name.decode()).is_file()]


def load_yaml(path):
    return yaml.safe_load(path.read_text(encoding="utf-8"))


class TemplateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compose = load_yaml(TEMPLATE / "worker-compose.yaml")
        cls.containers = cls.compose["containers"]

    def test_local_stack_matches_harness_and_adds_browser(self):
        base = load_yaml(ROOT / "harness/worker-compose.yaml")
        self.assertEqual(set(self.containers), set(base["containers"]) | {"browser"})
        self.assertEqual(self.compose["engine"], base["engine"])
        self.assertEqual(self.compose["startup_timeout"], base["startup_timeout"])
        for name, container in self.containers.items():
            with self.subTest(worker=name):
                self.assertEqual(container["worker"], f"path://../{name}")
                source = (TEMPLATE / container["worker"].removeprefix("path://")).resolve()
                self.assertEqual(source, ROOT / name)
                self.assertTrue((source / "iii.worker.yaml").is_file())
                manifest = tomllib.loads((source / "Cargo.toml").read_text())
                binaries = {entry["name"] for entry in manifest.get("bin", [])}
                if not binaries:
                    binaries.add(manifest["package"]["name"])
                command = shlex.split(container["scripts"]["run"])
                self.assertEqual(command[:3], ["cargo", "run", "--bin"])
                self.assertIn(command[3], binaries)
                self.assertEqual(command[3], name)

    def test_dependencies_are_declared_before_consumers(self):
        seen = set()
        for name, container in self.containers.items():
            for dependency in container.get("start_after", []):
                self.assertIn(dependency, seen, f"{name} must follow {dependency}")
            seen.add(name)
        self.assertEqual(
            set(self.containers["harness"]["start_after"]),
            set(self.containers) - {"harness"},
        )

    def test_default_env_file_exists_and_has_no_credentials(self):
        for name, container in self.containers.items():
            self.assertEqual(
                container["env_file"],
                ["${WORKERS_DEV_ENV_FILE:-workers-dev.env.example}"],
                name,
            )
        example = TEMPLATE / "workers-dev.env.example"
        self.assertTrue(example.is_file())
        active = [line for line in example.read_text().splitlines() if line.strip() and not line.startswith("#")]
        self.assertEqual(active, [], "The tracked env example must contain only comments")
        router_env = self.containers["llm-router"].get("environment", {})
        self.assertNotIn("ANTHROPIC_API_KEY", router_env, "Do not mask env_file credentials")
        self.assertNotIn("OPENAI_API_KEY", router_env, "Do not mask env_file credentials")

    def test_configuration_seeds_are_loaded_from_compose_directory(self):
        for worker, filename in [("iii-directory", "iii-directory.yaml"), ("ade", "console.yaml")]:
            command = shlex.split(self.containers[worker]["scripts"]["run"])
            self.assertEqual(command[4:], ["--", "--config", f"$III_COMPOSE_DIR/config/{filename}"])
            self.assertIsInstance(load_yaml(TEMPLATE / "config" / filename), dict)

    def test_local_skills_are_protected_from_registry_downloads(self):
        config = load_yaml(TEMPLATE / "config/iii-directory.yaml")
        self.assertEqual(config["agents_folder"], "agents")
        self.assertEqual(config["local_skills_folder"], "skills")
        self.assertEqual(config["skills_folder"], "data/skills")
        self.assertTrue(config["auto_download"])
        self.assertTrue((TEMPLATE / config["local_skills_folder"] / "harness").is_dir())
        self.assertNotEqual(config["local_skills_folder"], config["skills_folder"])

    def test_console_defaults_to_loopback(self):
        config = load_yaml(TEMPLATE / "config/console.yaml")
        self.assertEqual(config["http_host"], "127.0.0.1")
        self.assertEqual(config["http_port"], 3113)

    def test_agent_profiles_resolve_all_preloaded_skills(self):
        agents = tracked_files("template/agents/*.md")
        self.assertTrue(agents, "Run sync.sh to populate agent profiles")
        self.assertTrue((ROOT / "iii-directory/prompts/iii-minimal.md").is_file())
        for path in agents:
            with self.subTest(agent=path.stem):
                text = path.read_text(encoding="utf-8")
                self.assertTrue(text.startswith("---\n"))
                frontmatter, body = text[4:].split("\n---\n", 1)
                profile = yaml.safe_load(frontmatter)
                self.assertEqual(profile["extends"], "iii-minimal")
                self.assertTrue(profile["name"] and profile["description"] and body.strip())
                for skill in profile["skills"]:
                    self.assertTrue(skill.startswith("harness/"))
                    self.assertTrue((TEMPLATE / "skills" / f"{skill}.md").is_file(), skill)
                for function in profile["functions"]:
                    self.assertRegex(function, r"^[a-z0-9_-]+(?:::[a-z0-9_-]+)+$")

    def test_generated_snapshot_matches_sync_manifest(self):
        manifest = json.loads((TEMPLATE / "upstream/sync.json").read_text())
        self.assertRegex(manifest["commit"], r"^[0-9a-f]{40}$")
        actual = {}
        for path in tracked_files("template/agents", "template/skills", "template/upstream"):
            if path != TEMPLATE / "upstream/sync.json":
                self.assertFalse(path.is_symlink())
                actual[path.relative_to(TEMPLATE).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
        self.assertFalse(any(path.startswith("config/") for path in manifest["files"]))
        self.assertEqual(actual, manifest["files"], "Run sync.sh; do not edit generated files directly")

    def test_all_upstream_skill_documents_are_present(self):
        skills = tracked_files("template/skills/harness")
        self.assertTrue(skills, "Run sync.sh to populate upstream skills")
        for path in skills:
            self.assertTrue(path.read_text(encoding="utf-8").strip(), path)
            self.assertFalse(path.is_symlink(), path)

    def test_git_tracks_seeds_and_instructions_but_ignores_runtime_files(self):
        tracked = [
            "template/config/iii-directory.yaml", "template/config/console.yaml",
            "template/upstream/config/new-worker.yaml", "template/upstream/sync.json",
            "template/workers-dev.env.example", "template/worker-compose.yaml",
            *[str(path.relative_to(ROOT)) for path in tracked_files("template/agents")],
            *[str(path.relative_to(ROOT)) for path in tracked_files("template/skills")],
        ]
        ignored = [
            "template/.env", "template/.env.local", "template/data/skills/browser/index.md",
            "template/.sync.lock/owner", "template/.sync-stage-test/file", "template/.iii/runtime.json",
            "template/config/new-worker.yaml", "template/worker-compose.lock",
            "template/scripts/.null-ls_test_sync_template.py",
        ]
        for paths, expected in [(tracked, set()), (ignored, set(ignored))]:
            result = subprocess.run(
                ["git", "check-ignore", "--no-index", "--stdin"],
                input="\n".join(paths) + "\n", text=True, cwd=ROOT,
                capture_output=True, check=False,
            )
            self.assertIn(result.returncode, [0, 1], result.stderr)
            self.assertEqual(set(result.stdout.splitlines()), expected)


if __name__ == "__main__":
    unittest.main(verbosity=2)
