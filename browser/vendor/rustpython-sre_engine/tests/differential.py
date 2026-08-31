#!/usr/bin/env python3
"""Deterministic CPython 3.12 differential corpus for the compatibility fork."""

from __future__ import annotations

import argparse
import json
import random
import re
import subprocess
import sys
from pathlib import Path

SEED = 0x31213

ATOMS = [
    "a", "b", ".", "[ab]", "[^x]", "[a-z]", "\\d", "\\D", "\\s",
    "\\S", "\\w", "\\W", "\\b", "\\B", "^", "$", "é", "Ω", "中",
]
TEXTS = [
    "", "a", "b", "ab", "aba", "aaab", "123", "a1 b2", " ", "\n",
    "éÉè", "ΩωΩ", "İıſK", "中a_9", "x\ny", "word-word", "١٢٣", "\u2003",
]
VALID_TEMPLATES = [
    "{}", "(?:{})", "({})", "{}*", "{}+", "{}?", "{}*?", "{}+?", "{}??",
    "{}{{0}}", "{}{{1}}", "{}{{0,2}}", "{}{{1,3}}", "(?={})", "(?!{})",
    "{}|b", "a|{}", "(?:{}a)?", "(?i:{})", "(?a:{})", "(?s:{})", "(?m:{})",
]
STRUCTURED = [
    "(a)?(?(1)b|c)", "(?P<x>a)(?P=x)", "(?<=ab)c", "(?<!ab)c",
    "(?=(a+))a", "(a*)(b?)", "((ab)+)", "(?:a?)*", "(?:a*)*",
    "(?>a*)a", "a*+a", "(?i)[a-z]+", "(?i)k", "(?i)s", "(?i)i",
    "(?i)σ", "(?i)ß", "\\A.*\\Z", "^.*$", "(?:|a)", "(?:a|)",
]
ERRORS = [
    "(", "[", "*", "+", "?", "a**", "a++?", "a{4,2}", "(?", "(?P)",
    "(?P<>)", "(?P<x>a)(?P<x>b)", "(?P=x)", "(?<x)", "(?<=a*)", "(?<!a+)",
    "(?i", "a(?i)b", "(?L)", "(?au)", "(?i-i:a)", "(?-a:a)", "(?z:a)",
    "\\", "\\x", "\\x0", "\\u123", "\\U00110000", "\\N", "\\N{}",
    "\\N{NO SUCH NAME}", "\\1", "(a)\\2", "[z-a]", "[\\w-a]", "[a-\\d]",
    "(?(0)a|b)", "(?(99)a|b)", "(a)(?(1)b|c|d)", ")", "a)", "{1}",
]


def valid_cases(rng: random.Random, count: int):
    cases: list[tuple[str, str, bool]] = []
    for atom in ATOMS:
        for template in VALID_TEMPLATES:
            pattern = template.format(atom)
            for text in TEXTS:
                cases.append((pattern, text, False))
                cases.append((pattern, text, True))
    for pattern in STRUCTURED:
        for text in TEXTS:
            cases.append((pattern, text, False))
            cases.append((pattern, text, True))
    rng.shuffle(cases)
    while len(cases) < count:
        left = rng.choice(ATOMS[:16])
        right = rng.choice(ATOMS[:16])
        pattern = rng.choice([
            f"(?:{left}{right}){{0,3}}", f"({left})({right})", f"(?={left}){right}",
            f"(?:{left}|{right})+", f"(?i:{left})(?a:{right})",
        ])
        cases.append((pattern, rng.choice(TEXTS), bool(rng.getrandbits(1))))
    return cases[:count]


def error_cases(rng: random.Random, count: int):
    cases: list[tuple[str, str, bool]] = []
    suffixes = ["", "a", "Ω", "(?:b)", "{2}"]
    for pattern in ERRORS:
        for suffix in suffixes:
            cases.append((pattern + suffix, rng.choice(TEXTS), bool(rng.getrandbits(1))))
    while len(cases) < count:
        pattern = rng.choice(ERRORS)
        cases.append((pattern, rng.choice(TEXTS), bool(rng.getrandbits(1))))
    rng.shuffle(cases)
    return cases[:count]


def encode_matches(pattern: re.Pattern[str], text: str) -> str:
    values = pattern.findall(text)
    rows: list[list[str]] = []
    for value in values:
        if pattern.groups == 0:
            rows.append([value])
        elif pattern.groups == 1:
            rows.append([value])
        else:
            rows.append(list(value))
    fields = [str(len(rows))]
    fields.extend(str(len(row)) + "".join(":" + item.encode().hex() for item in row) for row in rows)
    return "OK\t" + "\t".join(fields)


def oracle(pattern: str, text: str, ignore_case: bool) -> str:
    try:
        compiled = re.compile(pattern, re.I if ignore_case else 0)
    except re.error as error:
        return "ERR\t" + str(error).encode().hex()
    return encode_matches(compiled, text)


def minimize(pattern: str, text: str, ignore_case: bool, driver) -> tuple[str, str]:
    expected = oracle(pattern, text, ignore_case)

    def differs(pat: str, value: str) -> bool:
        return query(driver, pat, value, ignore_case) != oracle(pat, value, ignore_case)

    changed = True
    while changed:
        changed = False
        for target, value in (("pattern", pattern), ("text", text)):
            for index in range(len(value)):
                candidate = value[:index] + value[index + 1 :]
                pat, subject = (candidate, text) if target == "pattern" else (pattern, candidate)
                if differs(pat, subject):
                    pattern, text = pat, subject
                    changed = True
                    break
            if changed:
                break
    return pattern, text


def query(driver: subprocess.Popen[str], pattern: str, text: str, ignore_case: bool) -> str:
    assert driver.stdin and driver.stdout
    driver.stdin.write(f"{int(ignore_case)}\t{pattern.encode().hex()}\t{text.encode().hex()}\n")
    driver.stdin.flush()
    response = driver.stdout.readline().rstrip("\n")
    if not response:
        raise RuntimeError("Rust differential driver stopped")
    return response


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=int, default=10_000)
    parser.add_argument("--max-mismatches", type=int, default=20)
    args = parser.parse_args()
    if sys.version_info[:3] != (3, 12, 13):
        parser.error(f"requires frozen CPython 3.12.13, got {sys.version.split()[0]}")

    root = Path(__file__).resolve().parents[1]
    subprocess.run(["cargo", "build", "--quiet", "--example", "differential_driver"], cwd=root, check=True)
    metadata = json.loads(
        subprocess.check_output(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=root)
    )
    executable = Path(metadata["target_directory"]) / "debug/examples/differential_driver"
    driver = subprocess.Popen(
        [executable],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    )
    rng = random.Random(SEED)
    valid_count = args.cases * 4 // 5
    cases = valid_cases(rng, valid_count) + error_cases(rng, args.cases - valid_count)
    mismatches = []
    try:
        for index, (pattern, text, ignore_case) in enumerate(cases, 1):
            expected = oracle(pattern, text, ignore_case)
            actual = query(driver, pattern, text, ignore_case)
            if actual != expected:
                minimized = minimize(pattern, text, ignore_case, driver)
                mismatch = (index, pattern, text, ignore_case, expected, actual, *minimized)
                if mismatch[6:] not in [item[6:] for item in mismatches]:
                    mismatches.append(mismatch)
                if len(mismatches) >= args.max_mismatches:
                    break
    finally:
        driver.terminate()
        driver.wait()

    for item in mismatches:
        index, pattern, text, ignore_case, expected, actual, min_pattern, min_text = item
        print(f"case={index} ignore_case={ignore_case} pattern={pattern!r} text={text!r}")
        print(f"  expected={expected}")
        print(f"  actual  ={actual}")
        print(f"  minimized pattern={min_pattern!r} text={min_text!r}")
    if mismatches:
        print(f"FAILED: {len(mismatches)} distinct mismatches", file=sys.stderr)
        return 1
    print(f"PASS: {len(cases)} cases (seed={SEED:#x}, valid={valid_count}, errors={len(cases)-valid_count})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
