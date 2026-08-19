#!/usr/bin/env python3
"""Deterministic public-wrapper differential corpus for HTML, CSS, and XPath."""

from __future__ import annotations

import argparse
import asyncio
import copy
import json
import random
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable

SEED = 0x31213
ROOT = Path(__file__).resolve().parent.parent
STANDALONE = ROOT.parent / "scrapling"

HTML_TAGS = ["div", "span", "p", "a", "b", "i", "table", "tr", "td", "template", "svg", "math"]
HTML_ATTRS = ["", " id=x", " class='a b'", " disabled", " data-x=", " a=1 a=2", " checked=''", " title='&amp;'"]
HTML_TEXT = ["", "a", " b ", "&amp;", "&#0;", "&#x80;", "éΩ中", "<!--c-->", "<![CDATA[x]]>"]

CSS_HTML = """<!doctype html><html><body>
<main id="root"><ul class="list"><li class="item a" data-n="1"><a href="/a">A</a></li>
<li class="item b" data-n="2"><a href="/b"><span>B</span></a></li><li class="item a" data-n="3">C</li></ul>
<section><p lang="en-US">one <b>bold</b></p><p hidden>two</p></section>
<template><div class="inside">T</div></template><svg><text>S</text></svg></main></body></html>"""

CSS_ATOMS = ["*", "li", "p", "a", "#root", ".item", "li.a", "[hidden]", "[data-n='2']", "[lang|='en']", "[class~='a']"]
CSS_PSEUDOS = ["", ":first-child", ":last-child", ":nth-child(2)", ":nth-of-type(2)", ":not(.b)", ":is(.a,.b)"]
CSS_ERRORS = ["[", "li:", "li::", "li:nth-child(", "li:not(", "li > > a", "#", ".", "li,", "::attr("]

XPATH_STEPS = ["*", "li", "p", "a", "span", "text()", "@class", "@data-n"]
XPATH_AXES = ["child", "descendant", "ancestor", "following-sibling", "preceding-sibling", "following", "preceding"]
XPATH_ERRORS = ["//[", "///", "//*[(]", "//li[", "unknown()", "//li/unknown::x", "//li[@", "(", "//li |"]


def html_cases(rng: random.Random, count: int) -> Iterable[tuple[str, dict[str, Any]]]:
    for _ in range(count):
        tokens: list[str] = []
        opened: list[str] = []
        for _ in range(rng.randint(3, 12)):
            action = rng.randrange(5)
            if action <= 1:
                tag = rng.choice(HTML_TAGS)
                tokens.append(f"<{tag}{rng.choice(HTML_ATTRS)}>")
                opened.append(tag)
            elif action == 2 and opened:
                tag = opened.pop(rng.randrange(len(opened)))
                tokens.append(f"</{tag}>")
            elif action == 3:
                tokens.append(rng.choice(HTML_TEXT))
            else:
                tag = rng.choice(HTML_TAGS)
                tokens.append(f"<{tag}{rng.choice(HTML_ATTRS)}/>")
        if rng.getrandbits(1):
            tokens.extend(f"</{tag}>" for tag in reversed(opened[: rng.randint(0, len(opened))]))
        yield "browser::extract", {
            "html": "".join(tokens),
            "selectors": [
                {"name": "elements", "css": "*", "html": True, "all": True},
                {"name": "text", "xpath": "//text()", "all": True},
                {"name": "attrs", "xpath": "//@*", "all": True},
            ],
        }


def css_cases(rng: random.Random, count: int) -> Iterable[tuple[str, dict[str, Any]]]:
    for index in range(count):
        if index % 5 == 0:
            query = rng.choice(CSS_ERRORS)
        else:
            left = rng.choice(CSS_ATOMS) + rng.choice(CSS_PSEUDOS)
            if rng.getrandbits(1):
                right = rng.choice(CSS_ATOMS) + rng.choice(CSS_PSEUDOS)
                query = left + rng.choice([" ", " > ", " + ", " ~ ", ", "]) + right
            else:
                query = left
            if rng.randrange(4) == 0:
                query += rng.choice(["::text", "::attr(class)", "::attr(href)"])
        payload: dict[str, Any] = {"html": CSS_HTML, "query": query, "first": bool(rng.getrandbits(1))}
        if rng.randrange(4) == 0:
            payload["attr"] = rng.choice(["class", "href", "missing"])
        yield "browser::css", payload


def xpath_cases(rng: random.Random, count: int) -> Iterable[tuple[str, dict[str, Any]]]:
    predicates = ["", "[1]", "[last()]", "[position()=2]", "[@class]", "[contains(@class,'a')]", "[string-length(.)>0]"]
    scalars = ["count(//li)", "string(//p[1])", "boolean(//a)", "false()", "true()", "1 + 2", "round(2.5)"]
    for index in range(count):
        choice = index % 10
        if choice == 0:
            query = rng.choice(XPATH_ERRORS)
        elif choice == 1:
            query = rng.choice(scalars)
        else:
            step = rng.choice(XPATH_STEPS)
            query = "//" + step + rng.choice(predicates)
            if rng.getrandbits(1):
                query += "/" + rng.choice(XPATH_AXES) + "::" + rng.choice(XPATH_STEPS[:5]) + rng.choice(predicates[:4])
            if rng.randrange(5) == 0:
                query = f"({query})[{rng.choice(['1', 'last()', 'position()=2'])}]"
            if rng.randrange(7) == 0:
                query += " | //p[1]"
        payload = {"html": CSS_HTML, "query": query, "first": bool(rng.getrandbits(1))}
        if rng.randrange(5) == 0:
            payload["attr"] = rng.choice(["class", "href", "missing"])
        yield "browser::xpath", payload


class Oracle:
    def __init__(self) -> None:
        sys.path.insert(0, str(STANDALONE))
        from src.handlers import create_handlers

        self.handlers = create_handlers(lambda: {})
        self.loop = asyncio.new_event_loop()

    def query(self, function: str, payload: dict[str, Any]) -> dict[str, Any]:
        name = function.removeprefix("browser::").replace("-", "_")
        try:
            return {"ok": self.loop.run_until_complete(self.handlers[name](payload))}
        except Exception as error:  # noqa: BLE001 - error text is part of the contract
            return {"err": str(error)}

    def close(self) -> None:
        self.loop.close()


class Driver:
    def __init__(self) -> None:
        subprocess.run(["cargo", "build", "--quiet", "--example", "scrapling_differential"], cwd=ROOT, check=True)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=ROOT))
        executable = Path(metadata["target_directory"]) / "debug/examples/scrapling_differential"
        self.process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def query(self, function: str, payload: dict[str, Any]) -> dict[str, Any]:
        assert self.process.stdin and self.process.stdout
        json.dump({"function": function, "payload": payload}, self.process.stdin, ensure_ascii=False, separators=(",", ":"))
        self.process.stdin.write("\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("Rust differential driver stopped")
        return json.loads(line)

    def close(self) -> None:
        self.process.terminate()
        self.process.wait()


def minimize(function: str, payload: dict[str, Any], oracle: Oracle, driver: Driver) -> dict[str, Any]:
    candidate = copy.deepcopy(payload)
    original_expected = oracle.query(function, payload)
    original_actual = driver.query(function, payload)
    outcome = (next(iter(original_expected)), next(iter(original_actual)))
    fields = ["html", "query"]
    changed = True
    while changed:
        changed = False
        for field in fields:
            value = candidate.get(field)
            if not isinstance(value, str):
                continue
            for index in range(len(value)):
                smaller = copy.deepcopy(candidate)
                smaller[field] = value[:index] + value[index + 1 :]
                expected, actual = oracle.query(function, smaller), driver.query(function, smaller)
                if (
                    expected != actual
                    and (next(iter(expected)), next(iter(actual))) == outcome
                ):
                    candidate = smaller
                    changed = True
                    break
            if changed:
                break
    return candidate


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--grammar", choices=["all", "html", "css", "xpath"], default="all")
    parser.add_argument("--cases", type=int, default=10_000, help="cases per selected grammar")
    parser.add_argument("--max-mismatches", type=int, default=20)
    parser.add_argument(
        "--oracle-check",
        choices=["full", "parser-runtime", "none"],
        default="full",
    )
    args = parser.parse_args()
    if sys.version_info[:3] != (3, 12, 13):
        parser.error(f"requires frozen CPython 3.12.13, got {sys.version.split()[0]}")
    if args.oracle_check != "none":
        command = [sys.executable, ROOT / "scripts/verify_oracle.py"]
        if args.oracle_check == "parser-runtime":
            command.append("--parser-runtime")
        subprocess.run(command, check=True)

    generators = {"html": html_cases, "css": css_cases, "xpath": xpath_cases}
    selected = generators if args.grammar == "all" else {args.grammar: generators[args.grammar]}
    oracle, driver = Oracle(), Driver()
    mismatches: list[
        tuple[str, int, dict[str, Any], dict[str, Any], dict[str, Any], dict[str, Any], dict[str, Any]]
    ] = []
    try:
        for offset, (grammar, generate) in enumerate(selected.items()):
            rng = random.Random(SEED + offset)
            for index, (function, payload) in enumerate(generate(rng, args.cases), 1):
                expected, actual = oracle.query(function, payload), driver.query(function, payload)
                if expected != actual:
                    reduced = minimize(function, payload, oracle, driver)
                    mismatches.append((grammar, index, payload, expected, actual, reduced, oracle.query(function, reduced)))
                    if len(mismatches) >= args.max_mismatches:
                        break
            if len(mismatches) >= args.max_mismatches:
                break
    finally:
        oracle.close()
        driver.close()

    for grammar, index, payload, expected, actual, reduced, reduced_expected in mismatches:
        print(f"{grammar} case={index} payload={json.dumps(reduced, ensure_ascii=False)}")
        print(f"  expected={json.dumps(reduced_expected, ensure_ascii=False)}")
        print(f"  original_payload={json.dumps(payload, ensure_ascii=False)}")
        print(f"  original_expected={json.dumps(expected, ensure_ascii=False)}")
        print(f"  original_actual={json.dumps(actual, ensure_ascii=False)}")
    if mismatches:
        print(f"FAILED: {len(mismatches)} mismatches", file=sys.stderr)
        return 1
    for grammar in selected:
        print(f"PASS {grammar}: {args.cases} cases (seed={SEED + list(selected).index(grammar):#x})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
