"""Exercise init.sh with a fake iii CLI on PATH; no network, no stack."""

from pathlib import Path
import os
import re
import shutil
import socket
import subprocess
import tempfile
import unittest

from ruamel.yaml import YAML

TEMPLATE = Path(__file__).resolve().parents[1]

# Records each call; `project init` writes $FAKE_COMPOSE (and $FAKE_HTTP_CONFIG, a
# template-shipped entry) like the real CLI writes a template, and fails like it
# for a template named `broken`.
FAKE_III = """#!/usr/bin/env bash
printf '%s|%s\\n' "$PWD" "$*" >> "$III_LOG"
if [[ "$1 $2" == "project init" ]]; then
  while (($#)); do [[ "$1" == -d ]] && dir=$2; [[ "$1" == -t ]] && name=$2; shift; done
  [[ "$name" != broken ]] || { echo "error: template scaffold failed" >&2; exit 1; }
  mkdir -p "$dir"
  printf '%s' "$FAKE_COMPOSE" > "$dir/worker-compose.yaml"
  if [[ -n "${FAKE_HTTP_CONFIG:-}" ]]; then
    mkdir -p "$dir/config"
    printf '%s' "$FAKE_HTTP_CONFIG" > "$dir/config/default-http.yaml"
  fi
fi
"""


def bound(port):
    """Listen on a port for the test; a port already taken elsewhere is just as busy."""
    busy = socket.socket()
    try:
        busy.bind(("127.0.0.1", port))
        busy.listen()
    except OSError:
        pass
    return busy


class InitTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="template-init-test-")
        self.addCleanup(temporary.cleanup)
        # init.sh resolves symlinks (pwd -P); macOS temp dirs sit behind /var -> /private/var.
        self.root = Path(temporary.name).resolve()
        self.launcher = self.root / "local template"
        self.launcher.mkdir()
        shutil.copy2(TEMPLATE / "init.sh", self.launcher / "init.sh")
        shutil.copytree(TEMPLATE / "scripts", self.launcher / "scripts")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        (bin_dir / "iii").write_text(FAKE_III)
        (bin_dir / "iii").chmod(0o755)
        self.log = self.root / "iii.log"
        # The templates repository init.sh clones, like iii-hq/templates; `ghost` has no manifest.
        self.templates = self.root / "templates-repo"
        manifests = {
            "template.yaml": "templates: [quickstart, harness, broken, ghost]\n",
            "quickstart/template.yaml": "description: Call a Python function from a Node worker\n",
            "harness/template.yaml": "description: Build with agents in the ADE\n",
            "broken/template.yaml": "description: Fails in the CLI\n",
        }
        for relative, content in manifests.items():
            (self.templates / "iii" / relative).parent.mkdir(parents=True, exist_ok=True)
            (self.templates / "iii" / relative).write_text(content)
        git = ["git", "-C", str(self.templates)]
        subprocess.run([*git, "init", "--quiet", "--initial-branch=main"], check=True)
        subprocess.run([*git, "add", "."], check=True)
        subprocess.run([*git, "-c", "user.name=Template Tests", "-c", "user.email=tests@example.com",
                        "commit", "--quiet", "-m", "Templates"], check=True)
        hidden = ("TEMPLATES_REPO", "III_TEMPLATE_URL_BRANCH")
        self.env = {
            **{key: value for key, value in os.environ.items() if key not in hidden},
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "III_LOG": str(self.log),
            "III_TEMPLATE_URL": self.templates.as_uri(),
            "FAKE_COMPOSE": "engine:\n  url: ws://127.0.0.1:49134\ncontainers: {}\n",
        }

    def run_init(self, *args, cwd=None):
        return subprocess.run(
            ["bash", str(self.launcher / "init.sh"), *args],
            env=self.env, capture_output=True, text=True, cwd=cwd,
        )

    def test_recreates_the_folder_and_starts_compose_on_its_own_port(self):
        destination = self.launcher / "quickstart"
        (destination / "data").mkdir(parents=True)
        (destination / "data/x").write_text("old state\n")
        (destination / "stale.txt").write_text("local edit\n")
        # A stopped run leaves an empty lock; it must not block the next start.
        (destination / ".iii/compose/default").mkdir(parents=True)
        (destination / ".iii/compose/default/engine.lock").write_text("")
        with bound(49334):
            result = self.run_init("quickstart")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((destination / "stale.txt").exists())
        self.assertFalse((destination / "data").exists())
        compose = (destination / "worker-compose.yaml").read_text()
        port = int(re.search(r"ws://127\.0\.0\.1:(\d+)", compose)[1])
        self.assertGreater(port, 49334)
        self.assertIn(f"engine  ws://127.0.0.1:{port}", result.stdout)
        # Quickstart declares no listener: the summary names the reserved ports instead of URLs.
        self.assertRegex(result.stdout, r"http +port \d+, used if http is added later")
        self.assertFalse((destination / "worker-compose.yaml.bak").exists())
        self.assertIn("Run `iii trigger ...` from", result.stdout)
        self.assertNotIn("left unchanged", result.stderr)
        # Without TEMPLATES_REPO, init.sh clones III_TEMPLATE_URL into the stage and hands the CLI that copy.
        self.assertIn(f"Fetching templates from {self.templates.as_uri()} (main)...", result.stderr)
        calls = self.log.read_text().splitlines()
        stage = rf"{re.escape(str(self.launcher))}/\.init-stage-\w+"
        self.assertRegex(calls[0].split("|", 1)[1], (
            rf"^project init -t quickstart -d {stage}/quickstart --skip-iii --template-dir {stage}/templates/iii$"
        ))
        self.assertEqual(calls[1], f"{destination}|compose --up")
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])

    def test_harness_uses_local_workers_and_ports_no_listener_or_sibling_holds(self):
        for worker in ("ade", "http"):
            (self.root / worker).mkdir()
            (self.root / worker / "iii.worker.yaml").write_text(f"name: {worker}\nlanguage: rust\nbin: {worker}\n")
            (self.root / worker / "Cargo.toml").write_text("[package]\n")
        # A sibling sandbox claims ports before its workers bind them.
        (self.launcher / "quickstart/config").mkdir(parents=True)
        (self.launcher / "quickstart/worker-compose.yaml").write_text("engine: {url: 'ws://127.0.0.1:49334'}\n")
        (self.launcher / "quickstart/config/default-ade.yaml").write_text("id: default-ade\nvalue: {http_port: 3113}\n")
        # A broken entry in a sibling must not hide that sibling's other claims.
        (self.launcher / "broken/config").mkdir(parents=True)
        (self.launcher / "broken/worker-compose.yaml").write_text("containers: {}\n")
        (self.launcher / "broken/config/default-ade.yaml").write_text("value: {http_port: [\n")
        (self.launcher / "broken/config/default-http.yaml").write_text("id: default-http\nvalue: {port: 3111}\n")
        self.env["FAKE_COMPOSE"] = (
            "engine:\n  url: ws://127.0.0.1:49134\n"
            "containers:\n"
            "  ade: # the ADE web UI\n    worker: package://ade\n"
            "  web:\n    worker: package://http\n"
            "  queue:\n    worker: package://queue\n"
        )
        self.env["TEMPLATES_REPO"] = str(self.templates)
        self.env["FAKE_HTTP_CONFIG"] = (
            "id: default-http\nname: HTTP\ndescription: Shipped by the template\n"
            "value:\n  port: 9999 # moved by init.sh\n  host: 127.0.0.1\n"
        )
        # 3111 is left unbound: only the broken sibling's http claim keeps it from being chosen.
        result = self.run_init("harness")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"--skip-iii --template-dir {self.templates}/iii", self.log.read_text())
        self.assertNotIn("Fetching templates", result.stderr)
        load = YAML(typ="safe").load
        destination = self.launcher / "harness"
        compose = load(destination / "worker-compose.yaml")
        containers = compose["containers"]
        self.assertEqual(containers["ade"]["worker"], "path://../../ade")
        self.assertEqual(containers["ade"]["scripts"]["run"], "cargo run --bin ade")
        self.assertEqual(containers["web"]["worker"], "path://../../http")
        self.assertEqual(containers["queue"]["worker"], "package://queue")
        engine = int(compose["engine"]["url"].rsplit(":", 1)[1])
        # Seeds use the engine store's entry names, so a later compose::add of http is moved too.
        ade = load(destination / "config/default-ade.yaml")["value"]["http_port"]
        shipped = load(destination / "config/default-http.yaml")
        http = shipped["value"]["port"]
        # A template's own entry keeps every field except the port.
        self.assertEqual((shipped["name"], shipped["value"]["host"]), ("HTTP", "127.0.0.1"))
        self.assertGreater(engine, 49334)
        self.assertGreater(ade, 3113)
        self.assertGreater(http, 3111)
        self.assertNotEqual(ade, http)
        self.assertIn(f"ade     http://127.0.0.1:{ade}", result.stdout)
        self.assertIn("local   2 workers from this checkout: ade, web", result.stdout)
        self.assertIn("# the ADE web UI", (destination / "worker-compose.yaml").read_text())

    def test_engine_runs_init_and_compose_on_another_iii_binary(self):
        build = self.root / "build"
        build.mkdir()
        (build / "iii-dev").write_text(f'#!/usr/bin/env bash\necho iii-dev >> "$III_LOG"\nexec "{self.root}/bin/iii" "$@"\n')
        (build / "iii-dev").chmod(0o755)
        # Relative to where init.sh is called, although compose runs from the project folder.
        result = self.run_init("quickstart", "engine=../build/iii-dev", cwd=self.launcher)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"with {build}/iii-dev...", result.stdout)
        self.assertIn(f"Run `{build}/iii-dev trigger ...` from", result.stdout)
        calls = self.log.read_text().splitlines()
        self.assertEqual(calls[0::2], ["iii-dev", "iii-dev"])
        self.assertIn("|project init -t quickstart", calls[1])
        self.assertEqual(calls[3], f"{self.launcher / 'quickstart'}|compose --up")

    def test_running_project_is_not_deleted(self):
        destination = self.launcher / "quickstart"
        (destination / ".iii/compose/default").mkdir(parents=True)
        (destination / ".iii/compose/default/engine.lock").write_text(str(os.getpid()))
        result = self.run_init("quickstart")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("still running", result.stderr)
        self.assertTrue((destination / ".iii/compose/default/engine.lock").is_file())
        self.assertFalse(self.log.exists())

    def test_help_is_shown_for_help_and_missing_arguments(self):
        shown = self.run_init("--help")
        self.assertEqual(shown.returncode, 0)
        self.assertIn("./init.sh harness", shown.stdout)
        self.assertIn("./init.sh --list", shown.stdout)
        missing = self.run_init()
        self.assertEqual(missing.returncode, 2)
        self.assertIn("Usage: ./init.sh <template>", missing.stderr)
        # A wrong argument names itself and points at --help instead of repeating it.
        for args, reason in (
            (["-x"], "Unknown option: -x"),
            (["quickstart", "port=1"], "Unknown argument: port=1"),
            (["quickstart", "engine="], "engine= needs an iii binary"),
            (["Quickstart"], "Invalid template name 'Quickstart'"),
            (["agents"], "Reserved template name 'agents'"),
        ):
            with self.subTest(args=args):
                wrong = self.run_init(*args)
                self.assertEqual(wrong.returncode, 2)
                self.assertTrue(wrong.stderr.startswith(reason), wrong.stderr)
                self.assertIn("./init.sh --help for more", wrong.stderr)
                self.assertNotIn("Every run is a first run", wrong.stderr)

    def test_invalid_or_reserved_names_never_reach_init(self):
        (self.launcher / "agents").mkdir()
        (self.launcher / "agents/keep.md").write_text("# Agent\n")
        for args in (["agents"], ["../x"], ["Missing"], ["quickstart", "port=1"], ["quickstart", "engine="]):
            with self.subTest(args=args):
                self.assertEqual(self.run_init(*args).returncode, 2)
        missing = self.run_init("quickstart", "engine=/nope/iii")
        self.assertEqual(missing.returncode, 1)
        self.assertIn("engine=/nope/iii is not an executable from", missing.stderr)
        self.assertIn("cargo build -p iii", missing.stderr)
        self.assertTrue((self.launcher / "agents/keep.md").is_file())
        self.assertFalse(self.log.exists())

    def test_list_shows_each_template_with_its_description(self):
        result = self.run_init("--list")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r"(?m)^  quickstart +Call a Python function from a Node worker$")
        self.assertRegex(result.stdout, r"(?m)^  harness +Build with agents in the ADE$")
        # A listed name without a manifest would fail in the CLI, so it is not offered.
        self.assertNotIn("ghost", result.stdout)
        self.assertIn("Start one with ./init.sh <template>", result.stdout)
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertFalse(self.log.exists())
        self.assertEqual(self.run_init("--list", "quickstart").returncode, 2)

    def test_unknown_template_lists_the_templates_before_init_runs(self):
        (self.launcher / "nope").mkdir()
        (self.launcher / "nope/keep.txt").write_text("previous run\n")
        for name, hint in (("nope", "Unknown template 'nope'. Available templates:"),
                           ("harnes", "Unknown template 'harnes'. Did you mean harness?"),
                           ("ghost", "Unknown template 'ghost'.")):
            with self.subTest(name=name):
                result = self.run_init(name)
                self.assertEqual(result.returncode, 2)
                self.assertIn(hint, result.stderr)
                self.assertRegex(result.stderr, r"(?m)^  quickstart +Call a Python")
                self.assertNotIn("Creating", result.stdout)
                self.assertNotIn("left unchanged", result.stderr)
        self.assertTrue((self.launcher / "nope/keep.txt").is_file())
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertFalse(self.log.exists())

    def test_unreachable_templates_repository_fails_before_init(self):
        self.env["III_TEMPLATE_URL"] = (self.root / "no-such-repo").as_uri()
        result = self.run_init("quickstart")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Could not clone", result.stderr)
        self.assertIn("set TEMPLATES_REPO", result.stderr)
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertFalse(self.log.exists())

    def test_failed_init_keeps_the_previous_folder(self):
        (self.launcher / "broken").mkdir()
        (self.launcher / "broken/keep.txt").write_text("previous run\n")
        result = self.run_init("broken")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("scaffold failed", result.stderr)
        self.assertIn("template/broken was left unchanged.", result.stderr)
        self.assertTrue((self.launcher / "broken/keep.txt").is_file())
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertNotIn("compose --up", self.log.read_text())

    def test_rejected_template_keeps_the_previous_folder_and_writes_nothing_outside(self):
        (self.launcher / "quickstart").mkdir()
        (self.launcher / "quickstart/keep.txt").write_text("previous run\n")
        engine = "engine:\n  url: ws://127.0.0.1:49134\ncontainers: {}\n"
        for compose, reason in (
            ("engine:\n  url: ws://example.com:49134\ncontainers: {}\n", "refusing to share the root stack's engine"),
            # A namespace names the seed files, so a traversal must not reach outside config/.
            (f"namespace: ../../../escape\n{engine}", "may hold only a-z"),
            (f"namespace: {self.root}/escape\n{engine}", "may hold only a-z"),
        ):
            with self.subTest(reason=reason, compose=compose.split("\n", 1)[0]):
                self.env["FAKE_COMPOSE"] = compose
                result = self.run_init("quickstart")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(reason, result.stderr)
                self.assertIn("template/quickstart was left unchanged.", result.stderr)
                self.assertTrue((self.launcher / "quickstart/keep.txt").is_file())
                self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
                self.assertEqual(list(self.root.rglob("escape-*.yaml")), [])
        self.assertNotIn("compose --up", self.log.read_text())


if __name__ == "__main__":
    unittest.main()
