"""Exercise the real Bash entrypoint against a local Git fixture; no network."""

from pathlib import Path
import json
import shutil
import subprocess
import tempfile
import unittest

TEMPLATE = Path(__file__).resolve().parents[1]


class SyncTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="harness-sync-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.upstream = self.root / "upstream repo"
        self.upstream.mkdir()
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.email", "tests@example.com")
        self.git("config", "user.name", "Template Tests")
        self.write("agents/example.md", "---\nname: Example\n---\nAgent instructions\n")
        self.write("skills/harness/index.md", "# Initial skill\n")
        self.write("config/console.yaml", "http_host: 0.0.0.0\nhttp_port: 9999\ntheme: dark\n")
        self.write("config/new-worker.yaml", "enabled: true\n")
        self.write("worker-compose.yaml", "containers: {harness: {worker: 'package://harness'}}\n")
        self.write("template.yaml", "name: Harness\n")
        self.write("README.md", "# Upstream template\n")
        self.write(".env", "FAKE_TEST_KEY=do-not-copy\n")
        self.write("setup.sh", "exit 123\n")
        self.commit()
        self.destination = self.root / "local template"
        (self.destination / "scripts").mkdir(parents=True)
        shutil.copy2(TEMPLATE / "sync.sh", self.destination / "sync.sh")
        shutil.copyfile(TEMPLATE / "scripts/sync_template.py", self.destination / "scripts/sync_template.py")
        self.protected = {
            "worker-compose.yaml": "containers: {harness: {worker: 'path://../harness'}}\n",
            "README.md": "# Local development\n",
            "config/console.yaml": "# Local settings\nhttp_host: 127.0.0.1\nhttp_port: 3113\n",
            "config/iii-directory.yaml": "agents_folder: agents\nlocal_skills_folder: skills\n",
            ".env": "LOCAL_TEST_KEY=preserve-me\n",
            "data/skills/browser/index.md": "Local runtime cache\n",
        }
        for relative, content in self.protected.items():
            path = self.destination / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.upstream), *args], text=True).strip()

    def write(self, relative, content):
        path = self.upstream / "iii/harness" / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Update fixture")
        return self.git("rev-parse", "HEAD")

    def run_sync(self, *args, success=True):
        result = subprocess.run(
            [str(self.destination / "sync.sh"), "--repo", str(self.upstream), *args],
            cwd=self.root, text=True, capture_output=True, timeout=60,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.destination / ".sync.lock").exists())
        self.assertFalse(list(self.destination.glob(".sync-stage-*")))
        return result

    def files(self):
        return {
            path.relative_to(self.destination).as_posix(): path.read_bytes()
            for path in self.destination.rglob("*") if path.is_file()
        }

    def test_initial_sync_is_repeatable_and_preserves_local_stack(self):
        self.run_sync()
        manifest = json.loads((self.destination / "upstream/sync.json").read_text())
        self.assertEqual(manifest["commit"], self.git("rev-parse", "HEAD"))
        self.assertEqual(manifest["ref"], "main")
        self.assertEqual((self.destination / "skills/harness/index.md").read_text(), "# Initial skill\n")
        for name in ("console.yaml", "new-worker.yaml"):
            self.assertEqual(
                (self.destination / "upstream/config" / name).read_bytes(),
                (self.upstream / "iii/harness/config" / name).read_bytes(),
            )
        for name in ("README.md", "worker-compose.yaml", "template.yaml"):
            self.assertFalse((self.destination / "upstream" / name).exists())
        self.assertFalse((self.destination / "config/new-worker.yaml").exists())
        self.assertFalse(any(path.startswith("config/") for path in manifest["files"]))
        self.assertFalse((self.destination / "setup.sh").exists())
        self.assertFalse((self.destination / "upstream/.env").exists())
        for relative, expected in self.protected.items():
            self.assertEqual((self.destination / relative).read_text(), expected)
        before = self.files()
        self.run_sync()
        self.assertEqual(self.files(), before)

    def test_refresh_adds_updates_and_removes_managed_files(self):
        self.run_sync()
        (self.upstream / "iii/harness/agents/example.md").unlink()
        (self.upstream / "iii/harness/config/new-worker.yaml").unlink()
        self.write("agents/replacement.md", "# New agent\n")
        self.write("skills/harness/index.md", "# Updated skill\n")
        self.commit()
        self.run_sync()
        self.assertFalse((self.destination / "agents/example.md").exists())
        self.assertFalse((self.destination / "upstream/config/new-worker.yaml").exists())
        self.assertTrue((self.destination / "agents/replacement.md").is_file())
        self.assertEqual((self.destination / "skills/harness/index.md").read_text(), "# Updated skill\n")

    def test_dry_run_and_invalid_ref_leave_destination_unchanged(self):
        self.run_sync()
        self.write("skills/harness/index.md", "# Changed\n")
        self.commit()
        before = self.files()
        self.run_sync("--dry-run")
        self.assertEqual(self.files(), before)
        self.run_sync("--ref", "missing-ref", success=False)
        self.assertEqual(self.files(), before)

    def test_explicit_commit_can_reproduce_an_older_snapshot(self):
        previous = self.git("rev-parse", "HEAD")
        self.write("skills/harness/index.md", "# Newer\n")
        self.commit()
        self.run_sync("--ref", previous)
        self.assertEqual((self.destination / "skills/harness/index.md").read_text(), "# Initial skill\n")

    def test_local_edits_require_explicit_force(self):
        self.run_sync()
        local = self.destination / "agents/custom.md"
        local.write_text("# Local edits\n")
        before = self.files()
        result = self.run_sync(success=False)
        self.assertIn("local edits", result.stderr)
        self.assertEqual(self.files(), before)
        self.run_sync("--force")
        self.assertFalse(local.exists())
        for relative, expected in self.protected.items():
            self.assertEqual((self.destination / relative).read_text(), expected)

    def test_force_with_dry_run_preserves_local_edits(self):
        self.run_sync()
        (self.destination / "agents/custom.md").write_text("# Keep local edits\n")
        before = self.files()
        self.run_sync("--force", "--dry-run")
        self.assertEqual(self.files(), before)

    def test_incomplete_upstream_fails_without_partial_updates(self):
        self.run_sync()
        before = self.files()
        shutil.rmtree(self.upstream / "iii/harness/agents")
        self.commit()
        self.run_sync(success=False)
        self.assertEqual(self.files(), before)

    def test_symlinks_are_rejected_without_touching_the_target(self):
        self.run_sync()
        before = self.files()
        target = self.root / "outside.md"
        target.write_text("Untouched\n")
        (self.upstream / "iii/harness/skills/harness/link.md").symlink_to(target)
        self.commit()
        self.run_sync(success=False)
        self.assertEqual(target.read_text(), "Untouched\n")
        self.assertEqual(self.files(), before)
        (self.destination / "agents/link.md").symlink_to(target)
        self.run_sync("--force", success=False)
        self.assertEqual(target.read_text(), "Untouched\n")

    def test_configuration_directory_is_optional_upstream(self):
        shutil.rmtree(self.upstream / "iii/harness/config")
        self.commit()
        self.run_sync()
        self.assertFalse((self.destination / "upstream/config").exists())
        for relative, expected in self.protected.items():
            self.assertEqual((self.destination / relative).read_text(), expected)


if __name__ == "__main__":
    unittest.main(verbosity=2)
