# /// script
# requires-python = ">=3.11"
# dependencies = ["ruamel.yaml==0.18.16"]
# ///
"""List the templates `iii project init` offers from a templates folder, or check one exists."""

import difflib
from pathlib import Path
import re
import sys
import textwrap

from ruamel.yaml import YAML, YAMLError


def mapping(path):
    """A YAML file's top-level mapping; {} when it is missing or broken."""
    try:
        document = YAML(typ="safe").load(path)
    except (YAMLError, OSError):
        return {}
    return document if isinstance(document, dict) else {}


def available(templates):
    """Name and description of each listed template, in the root manifest's order."""
    names = mapping(templates / "template.yaml").get("templates")
    # A listed name without its own manifest fails in the CLI too, so it is not offered.
    found = {str(name): str(mapping(templates / str(name) / "template.yaml").get("description", "")).strip()
             for name in (names if isinstance(names, list) else [])
             # init.sh's own name rule, so a manifest entry never reaches outside the folder.
             if re.fullmatch(r"[a-z0-9][a-z0-9_-]*", str(name))
             and (templates / str(name) / "template.yaml").is_file()}
    if not found:
        raise SystemExit(
            f"{templates}/template.yaml lists no templates. "
            "Point TEMPLATES_REPO at a templates checkout (the folder that holds iii/)."
        )
    return found


def show(templates, out):
    width = max(map(len, templates))
    for name, description in templates.items():
        # One line per template keeps the list scannable in an 80-column terminal.
        text = textwrap.shorten(description, 80 - width - 4, placeholder="...") if description else ""
        print(f"  {name:<{width}}  {text}", file=out)
    print("\nStart one with ./init.sh <template>, for example ./init.sh quickstart", file=out)


def main(folder, name=None):
    templates = available(folder)
    if name is None:
        show(templates, sys.stdout)
    elif name not in templates:
        close = difflib.get_close_matches(name, templates, n=1)
        hint = f" Did you mean {close[0]}?" if close else ""
        print(f"Unknown template '{name}'.{hint} Available templates:\n", file=sys.stderr)
        show(templates, sys.stderr)
        raise SystemExit(2)


if __name__ == "__main__":
    main(Path(sys.argv[1]), *sys.argv[2:3])
