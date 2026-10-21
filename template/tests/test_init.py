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
# for a template named `missing`.
FAKE_III = """#!/usr/bin/env bash
printf '%s|%s\\n' "$PWD" "$*" >> "$III_LOG"
if [[ "$1 $2" == "project init" ]]; then
  while (($#)); do [[ "$1" == -d ]] && dir=$2; [[ "$1" == -t ]] && name=$2; shift; done
  [[ "$name" != missing ]] || { echo "Template 'missing' not found" >&2; exit 1; }
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
        self.env = {
            **{key: value for key, value in os.environ.items() if key != "TEMPLATES_REPO"},
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "III_LOG": str(self.log),
            "FAKE_COMPOSE": "engine:\n  url: ws://127.0.0.1:49134\ncontainers: {}\n",
        }

    def run_init(self, *args):
        return subprocess.run(
            ["bash", str(self.launcher / "init.sh"), *args],
            env=self.env, capture_output=True, text=True,
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
        # Without TEMPLATES_REPO the CLI fetches iii-hq/templates itself, into a stage beside the folder.
        calls = self.log.read_text().splitlines()
        self.assertRegex(calls[0].split("|", 1)[1], (
            rf"^project init -t quickstart -d {re.escape(str(self.launcher))}/\.init-stage-\w+/quickstart --skip-iii$"
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
        self.env["FAKE_COMPOSE"] = (
            "engine:\n  url: ws://127.0.0.1:49134\n"
            "containers:\n"
            "  ade: # the ADE web UI\n    worker: package://ade\n"
            "  web:\n    worker: package://http\n"
            "  queue:\n    worker: package://queue\n"
        )
        self.env["TEMPLATES_REPO"] = "/checkout/templates"
        self.env["FAKE_HTTP_CONFIG"] = (
            "id: default-http\nname: HTTP\ndescription: Shipped by the template\n"
            "value:\n  port: 9999 # moved by init.sh\n  host: 127.0.0.1\n"
        )
        with bound(3111):
            result = self.run_init("harness")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--skip-iii --template-dir /checkout/templates/iii", self.log.read_text())
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
        missing = self.run_init()
        self.assertEqual(missing.returncode, 2)
        self.assertIn("Usage: ./init.sh <template>", missing.stderr)

    def test_invalid_or_reserved_names_never_reach_init(self):
        (self.launcher / "agents").mkdir()
        (self.launcher / "agents/keep.md").write_text("# Agent\n")
        for name in ("agents", "../x", "Missing"):
            with self.subTest(name=name):
                self.assertEqual(self.run_init(name).returncode, 2)
        self.assertTrue((self.launcher / "agents/keep.md").is_file())
        self.assertFalse(self.log.exists())

    def test_failed_init_keeps_the_previous_folder(self):
        (self.launcher / "missing").mkdir()
        (self.launcher / "missing/keep.txt").write_text("previous run\n")
        result = self.run_init("missing")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not found", result.stderr)
        self.assertTrue((self.launcher / "missing/keep.txt").is_file())
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertNotIn("compose --up", self.log.read_text())

    def test_failed_configuration_keeps_the_previous_folder(self):
        (self.launcher / "quickstart").mkdir()
        (self.launcher / "quickstart/keep.txt").write_text("previous run\n")
        self.env["FAKE_COMPOSE"] = "engine:\n  url: ws://example.com:49134\ncontainers: {}\n"
        result = self.run_init("quickstart")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to share the root stack's engine", result.stderr)
        self.assertTrue((self.launcher / "quickstart/keep.txt").is_file())
        self.assertEqual(list(self.launcher.glob(".init-stage-*")), [])
        self.assertNotIn("compose --up", self.log.read_text())


if __name__ == "__main__":
    unittest.main()
