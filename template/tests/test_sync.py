"""Exercise the download entrypoint against local Git fixtures; no network."""

from pathlib import Path
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

TEMPLATE = Path(__file__).resolve().parents[1]


class SyncTests(unittest.TestCase):
    def setUp(self):
        """Create independent upstream and launcher directories, both with spaces."""
        self.temporary = tempfile.TemporaryDirectory(prefix="template-sync-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.upstream = self.root / "upstream repo"
        self.upstream.mkdir()
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.email", "tests@example.com")
        self.git("config", "user.name", "Template Tests")
        self.write("agents/example.md", "# Agent\n")
        self.write("skills/example.md", "# Skill\n")
        self.write("worker-compose.yaml", "containers: {harness: {worker: 'package://harness'}}\n")
        self.write("config/console.yaml", "http_port: 9999\n")
        self.write("template.yaml", "name: Harness\n")
        self.write("README.md", "# Upstream template\n")
        self.write(".env", "FAKE_TEST_KEY=example\n")
        self.write(".hidden/nested.txt", "Hidden file\n")
        self.write(".gitignore", "*.ignored\n")
        self.write("keep.ignored", "Still tracked upstream\n")
        self.write(".gitattributes", "README.md export-ignore\n")
        self.write("assets/icon.bin", b"\x00\xff\x01\x80")
        self.write("src/file with spaces.py", "print('hello')\n")
        self.marker = self.root / "script-was-executed"
        self.write("setup.sh", f"#!/bin/sh\ntouch {shlex.quote(str(self.marker))}\nexit 123\n")
        (self.upstream / "iii/harness/setup.sh").chmod(0o755)
        self.commit()
        self.launcher = self.root / "local template"
        self.destination = self.launcher / "harness"
        (self.launcher / "scripts").mkdir(parents=True)
        shutil.copy2(TEMPLATE / "sync.sh", self.launcher / "sync.sh")
        shutil.copyfile(TEMPLATE / ".gitignore", self.launcher / ".gitignore")
        shutil.copyfile(TEMPLATE / "scripts/sync_template.py", self.launcher / "scripts/sync_template.py")
        # These are deliberately not project seeds: a downloader must ignore them.
        (self.launcher / "worker-compose.yaml").write_text("# Local baseline\n")
        (self.launcher / "README.md").write_text("# Download tooling\n")

    def git(self, *args):
        """Run Git in the disposable upstream repository."""
        return subprocess.check_output(["git", "-C", str(self.upstream), *args], text=True).strip()

    def write(self, relative, content, template="harness"):
        """Write any upstream bytes, without template-specific structure rules."""
        path = self.upstream / "iii" / template / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content if isinstance(content, bytes) else content.encode())

    def commit(self):
        """Include hidden and ignored fixture files in the upstream snapshot."""
        self.git("add", "--force", ".")
        self.git("commit", "--quiet", "-m", "Update fixture")
        return self.git("rev-parse", "HEAD")

    def run_sync(self, *args, success=True, env=None, input_text=""):
        """Run from outside template/ without any stopped-stack acknowledgment."""
        result = subprocess.run(
            [str(self.launcher / "sync.sh"), "--repo", str(self.upstream), *args],
            cwd=self.root, text=True, capture_output=True, timeout=60, env=env,
            input=input_text,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.launcher / ".sync.lock").exists())
        self.assertFalse(list(self.launcher.glob(".sync-stage-*")))
        return result

    def inventory(self, root):
        """Capture contents, executable bits and symlink text without following links."""
        result = {}
        for path in root.rglob("*"):
            relative = path.relative_to(root).as_posix()
            if path.is_symlink():
                result[relative] = ("link", os.readlink(path))
            elif path.is_file():
                result[relative] = ("file", path.read_bytes(), bool(path.stat().st_mode & 0o111))
            else:
                result[relative] = ("directory",)
        return result

    def files(self):
        """Capture launcher state for non-destructive assertions."""
        return self.inventory(self.launcher)

    def test_default_download_is_verbatim_and_repeatable(self):
        """Download every byte and executable bit, not generated local substitutes."""
        baseline = self.files()
        result = self.run_sync()
        self.assertEqual(result.stdout.splitlines()[-1], "cd 'local template/harness'")
        self.assertEqual(self.inventory(self.destination), self.inventory(self.upstream / "iii/harness"))
        self.assertFalse(self.marker.exists())
        self.assertFalse((self.destination / "upstream/sync.json").exists())
        for relative, content in baseline.items():
            self.assertEqual(self.files()[relative], content)
        before = self.files()
        self.assertNotIn("Do you really want to overwrite", result.stdout)
        self.run_sync(input_text="yes\n")
        self.assertEqual(self.files(), before)

    def test_harness_kanban_download_needs_no_agents_or_skills(self):
        """Regression: a Compose/README/env-only template is a valid download."""
        for relative, content in {
            "README.md": "# Kanban\n",
            "worker-compose.yaml": "containers: {kanban: {worker: 'package://kanban'}}\n",
            ".env": "PROVIDER_KEY=\n",
            "template.yaml": "name: Harness + Kanban\n",
        }.items():
            self.write(relative, content, template="harness-kanban")
        self.commit()
        self.run_sync()
        default_before = self.inventory(self.destination)
        result = self.run_sync("--template", "harness-kanban")
        self.assertEqual(result.stdout.splitlines()[-1], "cd 'local template/harness-kanban'")
        self.assertEqual(
            self.inventory(self.launcher / "harness-kanban"),
            self.inventory(self.upstream / "iii/harness-kanban"),
        )
        self.assertEqual(self.inventory(self.destination), default_before)

    def test_arbitrary_content_requires_no_manifest_or_compose(self):
        """The downloader knows nothing about what makes a project runnable."""
        self.write("anything.dat", b"\x00arbitrary", template="minimal")
        self.commit()
        self.run_sync("--template", "minimal")
        self.assertEqual(self.inventory(self.launcher / "minimal"), self.inventory(self.upstream / "iii/minimal"))

    def test_existing_destination_overwrites_matching_files_and_keeps_local_only(self):
        """No manifest, edit check, force or stopped-stack flag is required."""
        self.destination.mkdir()
        (self.destination / "README.md").write_text("Local edit\n")
        (self.destination / "notes.txt").write_text("Local-only work\n")
        (self.destination / "engine.pid").write_text("123\n")
        result = self.run_sync(input_text="yes\n")
        self.assertIn("Back up your files before continuing.", result.stdout)
        self.assertIn("[yes/no] (default: no)", result.stdout)
        prompt = result.stdout.index("Do you really want to overwrite")
        for text in ("Template: harness;", "Upstream commit:", "Downloading ", "  README.md"):
            self.assertLess(prompt, result.stdout.index(text))
        self.assertEqual((self.destination / "README.md").read_text(), "# Upstream template\n")
        self.assertEqual((self.destination / "notes.txt").read_text(), "Local-only work\n")
        self.assertEqual((self.destination / "engine.pid").read_text(), "123\n")
        (self.destination / "config/console.yaml").write_text("Local settings\n")
        self.write("config/console.yaml", "http_port: 4000\n")
        (self.upstream / "iii/harness/agents/example.md").unlink()
        self.commit()
        self.run_sync(input_text=" Y \n")
        self.assertEqual((self.destination / "config/console.yaml").read_text(), "http_port: 4000\n")
        self.assertTrue((self.destination / "agents/example.md").exists())

    def test_overwrite_requires_explicit_yes_and_never_bypasses_with_force(self):
        """No, Enter, EOF and invalid input preserve edits, modes and local files."""
        self.run_sync()
        (self.destination / "README.md").write_text("My edited README\n")
        (self.destination / ".env").write_text("LOCAL_SECRET=keep\n")
        (self.destination / "notes.txt").write_text("My local work\n")
        before = self.files()
        for flags in ((), ("--force", "--stack-stopped")):
            for answer in ("no\n", "n\n", "\n", "", "maybe\n"):
                with self.subTest(flags=flags, answer=answer):
                    result = self.run_sync(*flags, input_text=answer, success=False)
                    self.assertIn(str(self.destination), result.stdout)
                    self.assertIn("Back up your files", result.stdout)
                    self.assertIn("Download cancelled. No files were changed.", result.stderr)
                    self.assertNotIn("Template downloaded!", result.stdout)
                    for text in ("Template: harness;", "Upstream commit:", "Downloading ", "  README.md"):
                        self.assertNotIn(text, result.stdout)
                    self.assertFalse(any(line.startswith("cd ") for line in result.stdout.splitlines()))
                    self.assertEqual(self.files(), before)
        result = self.run_sync(input_text="YES\n")
        self.assertIn("Template downloaded!", result.stdout)
        self.assertEqual((self.destination / "README.md").read_text(), "# Upstream template\n")
        self.assertEqual((self.destination / ".env").read_text(), "FAKE_TEST_KEY=example\n")
        self.assertEqual((self.destination / "notes.txt").read_text(), "My local work\n")

    def test_empty_existing_named_folder_also_requires_confirmation(self):
        """An existing directory prompts even when empty and not named harness."""
        self.write("README.md", "# Kanban\n", template="harness-kanban")
        self.commit()
        destination = self.launcher / "harness-kanban"
        destination.mkdir()
        before = self.files()
        result = self.run_sync("--template", "harness-kanban", input_text="no\n", success=False)
        self.assertIn(str(destination), result.stdout)
        self.assertEqual(self.files(), before)
        self.run_sync("--template", "harness-kanban", input_text="yes\n")
        self.assertEqual((destination / "README.md").read_text(), "# Kanban\n")
        self.assertFalse(self.destination.exists())

    def test_existing_folder_preview_needs_no_confirmation(self):
        """A read-only preview neither prompts nor changes files."""
        self.run_sync()
        before = self.files()
        result = self.run_sync("--dry-run")
        self.assertNotIn("Do you really want to overwrite", result.stdout)
        self.assertNotIn("Download cancelled", result.stderr)
        self.assertEqual(self.files(), before)

    def test_preview_and_fetch_failures_leave_destination_unchanged(self):
        """Neither a fresh preview nor a failed download leaves partial output."""
        before = self.files()
        result = self.run_sync("--dry-run")
        self.assertFalse(any(line.startswith("cd ") for line in result.stdout.splitlines()))
        self.assertEqual(self.files(), before)
        self.run_sync("--template", "missing", success=False)
        self.assertEqual(self.files(), before)
        self.run_sync()
        self.write("README.md", "# Changed\n")
        self.commit()
        before = self.files()
        self.run_sync("--dry-run")
        self.run_sync("--ref", "missing-ref", success=False)
        self.assertEqual(self.files(), before)

    def test_explicit_ref_reproduces_older_content(self):
        """Pinning a commit is independent of the current default branch."""
        old = self.git("rev-parse", "HEAD")
        self.write("README.md", "# Newer\n")
        self.commit()
        self.run_sync("--ref", old)
        self.assertEqual((self.destination / "README.md").read_text(), "# Upstream template\n")

    def test_symlinks_are_copied_without_following_them(self):
        """Copy link text, including outside targets, without modifying the target."""
        outside = self.root / "outside.txt"
        outside.write_text("Untouched\n")
        (self.upstream / "iii/harness/readme-link").symlink_to("README.md")
        (self.upstream / "iii/harness/outside-link").symlink_to(outside)
        self.commit()
        self.run_sync()
        self.assertEqual(self.inventory(self.destination), self.inventory(self.upstream / "iii/harness"))
        self.assertEqual(outside.read_text(), "Untouched\n")
        (self.destination / "README.md").unlink()
        (self.destination / "README.md").symlink_to(outside)
        self.run_sync(input_text="yes\n")
        self.assertFalse((self.destination / "README.md").is_symlink())
        self.assertEqual(outside.read_text(), "Untouched\n")

    def test_local_symlink_parents_cannot_redirect_writes(self):
        """Filesystem safeguards are not template structure or runtime checks."""
        outside = self.root / "outside"
        outside.mkdir()
        self.destination.mkdir()
        (self.destination / "config").symlink_to(outside, target_is_directory=True)
        before = self.files()
        self.run_sync(success=False)
        self.assertEqual(list(outside.iterdir()), [])
        self.assertEqual(self.files(), before)
        (self.destination / "config").unlink()
        self.destination.rmdir()
        self.destination.symlink_to(outside, target_is_directory=True)
        self.run_sync(success=False)
        self.assertEqual(list(outside.iterdir()), [])

    def test_invalid_names_cannot_escape_or_overwrite_tooling(self):
        """Names must select an isolated destination rather than a tool directory."""
        before = self.files()
        for name in ("../outside", "/tmp/outside", "iii/harness", ".", "scripts", "tests", "config", "agents", "skills", "upstream", "data", "bad name", "-harness"):
            with self.subTest(name=name):
                self.run_sync("--template", name, success=False)
        self.run_sync("--template", success=False)
        self.assertEqual(self.files(), before)

    def test_legacy_flags_are_optional_noops(self):
        """Existing invocations remain usable, but never gate the download."""
        self.run_sync("--stack-stopped", "--force")
        self.assertEqual(self.inventory(self.destination), self.inventory(self.upstream / "iii/harness"))

    def test_downloads_and_local_changes_do_not_dirty_git(self):
        """Ignored destinations include their own .gitignore and arbitrary local work."""
        def local_git(*args):
            return subprocess.check_output(["git", "-C", str(self.launcher), *args], text=True).strip()

        local_git("init", "--quiet")
        local_git("add", ".")
        local_git("-c", "user.name=Template Tests", "-c", "user.email=tests@example.com", "commit", "--quiet", "-m", "Tooling")
        self.run_sync()
        (self.destination / "notes.txt").write_text("Local work\n")
        (self.destination / ".gitignore").write_text("!*\n")
        self.write("README.md", "# Another\n", template="another")
        self.commit()
        self.run_sync("--template", "another")
        self.assertEqual(local_git("status", "--porcelain", "--untracked-files=all"), "")
        for relative in ("harness/notes.txt", "harness/.env", "harness/worker-compose.yaml", "another/README.md"):
            self.assertEqual(local_git("check-ignore", "--no-index", relative), relative)

    def python_environment(self, interpreters):
        """Build an isolated PATH so installed host aliases cannot mask failures."""
        binary_dir = Path(tempfile.mkdtemp(prefix="python-bin-", dir=self.root))
        trace = binary_dir / "calls.log"
        for command in ("bash", "dirname", "git", "mkdir", "mktemp", "rm", "rmdir"):
            executable = shutil.which(command)
            self.assertIsNotNone(executable, command)
            (binary_dir / command).symlink_to(executable)
        for name, behavior in interpreters.items():
            script = binary_dir / name
            code = "#!/bin/sh\n"
            code += f'printf \'%s %s\\n\' {shlex.quote(name)} "$1" >> {shlex.quote(str(trace))}\n'
            if behavior == "broken":
                code += "exit 127\n"
            elif behavior == "old":
                probe = "import sys; sys.version_info = (3, 10); exec(sys.argv[1])"
                code += f'exec {shlex.quote(sys.executable)} -c {shlex.quote(probe)} "$2"\n'
            else:
                code += f'exec {shlex.quote(sys.executable)} "$@"\n'
            script.write_text(code)
            script.chmod(0o755)
        return {**os.environ, "PATH": str(binary_dir)}, trace

    def test_python_command_selection_uses_the_validated_interpreter(self):
        """Prefer python3, falling back to python if missing, old or broken."""
        cases = [
            ({"python3": "valid"}, "python3"),
            ({"python": "valid"}, "python"),
            ({"python3": "valid", "python": "valid"}, "python3"),
            ({"python3": "old", "python": "valid"}, "python"),
            ({"python3": "broken", "python": "valid"}, "python"),
        ]
        for interpreters, selected in cases:
            with self.subTest(interpreters=interpreters):
                env, trace = self.python_environment(interpreters)
                self.run_sync("--dry-run", env=env)
                calls = trace.read_text().splitlines()
                self.assertIn(f"{selected} -c", calls)
                self.assertEqual(calls[-1], f"{selected} {self.launcher / 'scripts/sync_template.py'}")
                if selected == "python3":
                    self.assertFalse(any(call.startswith("python ") for call in calls))
                self.assertFalse(self.destination.exists())

    def test_missing_or_incompatible_python_fails_before_fetch(self):
        """A clear dependency error must not modify the workspace or fetch Git."""
        before = self.files()
        for interpreters in ({}, {"python": "old"}, {"python3": "old", "python": "old"}):
            with self.subTest(interpreters=interpreters):
                env, _ = self.python_environment(interpreters)
                result = self.run_sync("--repo", str(self.root / "nonexistent-repo"), env=env, success=False)
                self.assertIn("Python 3.11+ is required", result.stderr)
                self.assertIn("Neither python3 nor python", result.stderr)
                self.assertEqual(self.files(), before)

    def test_missing_importer_reports_checkout_problem_before_fetch(self):
        """Do not mistake a missing Python source file for a missing interpreter."""
        (self.launcher / "scripts/sync_template.py").unlink()
        before = self.files()
        env, _ = self.python_environment({})
        result = self.run_sync("--repo", str(self.root / "nonexistent-repo"), env=env, success=False)
        self.assertIn("Sync importer is missing or unreadable", result.stderr)
        self.assertIn("Restore template/scripts/sync_template.py", result.stderr)
        self.assertNotIn("Python 3.11+ is required", result.stderr)
        self.assertEqual(self.files(), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
