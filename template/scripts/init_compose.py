# /// script
# requires-python = ">=3.11"
# dependencies = ["ruamel.yaml==0.18.16"]
# ///
"""Point a fresh `iii project init` project at this checkout's workers and free ports."""

import os
from pathlib import Path, PurePosixPath
import re
import socket
import sys

from ruamel.yaml import YAML, YAMLError

from sync_template import configure_local_workers

ENGINE = re.compile(r"(ws://127\.0\.0\.1:)(\d+)")
FIRST_ENGINE_PORT = 49334
# Config key and default port of each worker that opens a listener; the default is kept when free.
# shortcut: only ade and http are known; add a worker here when a template declares another listener.
LISTENERS = {"ade": ("http_port", 3113), "http": ("port", 3111)}


def claimed_ports(sandbox):
    """Ports a sibling sandbox holds or will hold once its workers bind them."""
    ports = set()
    try:
        document = YAML(typ="safe").load(sandbox / "worker-compose.yaml")
        match = ENGINE.fullmatch(str(((document or {}).get("engine") or {}).get("url", "")))
        if match:
            ports.add(int(match[2]))
        for name, (field, _) in LISTENERS.items():
            for config in sandbox.glob(f"config/*-{name}.yaml"):
                port = ((YAML(typ="safe").load(config) or {}).get("value") or {}).get(field)
                if isinstance(port, int):
                    ports.add(port)
    except (YAMLError, OSError, AttributeError):
        pass  # A broken sibling cannot start, so it holds nothing.
    return ports


def free_port(start, taken):
    """First port from `start` that no sibling claims and nothing listens on."""
    port = start
    while port in taken or listening(port):
        port += 1
    taken.add(port)
    return port


def listening(port):
    with socket.socket() as probe:
        return probe.connect_ex(("127.0.0.1", port)) == 0


def main(destination):
    compose = destination / "worker-compose.yaml"
    if not compose.is_file():
        return
    sandboxes = destination.parent
    local = [key for key, _ in configure_local_workers(compose, destination, sandboxes.parent)]

    taken = set()
    for sibling in sandboxes.glob("*/worker-compose.yaml"):
        if sibling != compose:
            taken |= claimed_ports(sibling.parent)

    yaml = YAML()
    yaml.preserve_quotes = True
    yaml.indent(mapping=2, sequence=4, offset=2)
    document = yaml.load(compose)
    engine = document.get("engine") if isinstance(document, dict) else None
    match = ENGINE.fullmatch(str((engine or {}).get("url", "")))
    if not match:
        raise SystemExit(f"{compose}: engine.url is not ws://127.0.0.1:<port>; refusing to share the root stack's engine.")
    engine_port = free_port(FIRST_ENGINE_PORT, taken)
    engine["url"] = f"{match[1]}{engine_port}"
    yaml.dump(document, compose)

    folder = os.path.relpath(destination)
    containers = document.get("containers") or {}
    declared = {PurePosixPath(str(c.get("worker", "")).split("://", 1)[-1]).name
                for c in containers.values() if isinstance(c, dict)}
    print(f"\n  {destination.name}: fresh `iii project init` in {folder}")
    # Workers install their defaults only into an empty entry, so a seed in the
    # engine's store moves a listener whether it is declared now or added later.
    namespace = document.get("namespace", "default")
    (destination / "config").mkdir(exist_ok=True)
    for name, (field, start) in LISTENERS.items():
        port = free_port(start, taken)
        entry = f"{namespace}-{name}"
        # The store requires name and description; the worker refreshes both when it registers.
        (destination / "config" / f"{entry}.yaml").write_text(
            f"id: {entry}\nname: {name}\ndescription: Port seeded by init.sh\nvalue:\n  {field}: {port}\n"
        )
        where = f"http://127.0.0.1:{port}" if name in declared else f"port {port}, used if {name} is added later"
        print(f"  {name:<7} {where}")
    print(f"  {'engine':<7} ws://127.0.0.1:{engine_port}")
    if local:
        print(f"  {'local':<7} {len(local)} workers from this checkout: {', '.join(local)}")
    print(f"\n  Run `iii trigger ...` from {folder}. Ctrl+C stops the stack.\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
