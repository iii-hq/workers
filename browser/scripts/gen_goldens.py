#!/usr/bin/env python3
"""Golden generator for the browser worker's scrapling surface.
THE ONLY WRITER of tests/golden/**.

Run with the locked oracle environment (see oracle/README.md):

  .oracle/bin/python scripts/gen_goldens.py schemas
  .oracle/bin/python scripts/gen_goldens.py behavior

Ground truth is the Python worker source in ../scrapling (schemas.py, core.py).

Every id in this file is the PYTHON id (`scrapling::css`) because that is what
addresses the reference implementation. The browser worker namespaces its own
surface as `browser::css`, so `wire_id()` is applied at write time
only — to the emitted `function_id`/`function` fields and golden filenames.
"""

import argparse
import asyncio
import base64
import hashlib
import http.server
import json
import os
import random
import socketserver
import subprocess
import tempfile
import threading
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent            # browser/scripts
WORKER = HERE.parent                              # browser/
REPO = WORKER.parent                              # workers repo root
sys.path.insert(0, str(REPO / "scrapling"))       # `src` package of the Python worker


def wire_ids_in(value):
    """Rewrite `scrapling::` -> `browser::` in every string, recursively.

    wire_id() only fixes the function_id field; schema/description free text
    (e.g. crawl's "default scrapling::crawl" stream hint) names the worker's
    own ids too, and those must show the browser:: surface the worker actually
    serves, not the Python reference's.
    """
    if isinstance(value, str):
        return value.replace("scrapling::", "browser::")
    if isinstance(value, dict):
        return {key: wire_ids_in(inner) for key, inner in value.items()}
    if isinstance(value, list):
        return [wire_ids_in(inner) for inner in value]
    return value


def wire_id(python_id: str) -> str:
    """Python's `scrapling::<leaf>` -> this worker's root `browser::<leaf>`."""
    leaf = python_id.removeprefix("scrapling::")
    if leaf == "screenshot":
        leaf = "screenshot-url"
    return "browser::" + leaf


# In `schemas.py`'s own FUNCTIONS order — the Rust catalog() must match it.
ALL_IDS = [
    "scrapling::fetch",
    "scrapling::stealthy-fetch",
    "scrapling::dynamic-fetch",
    "scrapling::screenshot",
    "scrapling::extract",
    "scrapling::css",
    "scrapling::xpath",
    "scrapling::regex",
    "scrapling::find-similar",
    "scrapling::find",
    "scrapling::find-by-text",
    "scrapling::find-by-regex",
    "scrapling::describe",
    "scrapling::to-markdown",
    "scrapling::session-open",
    "scrapling::session-fetch",
    "scrapling::session-close",
    "scrapling::session-list",
    "scrapling::crawl",
]

def dump(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False) + "\n")


def gen_schemas(output: Path = WORKER) -> None:
    from src.schemas import FUNCTIONS  # imports only typing — safe

    by_id = {f["id"]: f for f in FUNCTIONS}
    missing = [fid for fid in ALL_IDS if fid not in by_id]
    if missing:                      # schemas.py drifted — fail loudly, don't emit half a surface
        raise SystemExit(f"ids in ALL_IDS but not in schemas.py FUNCTIONS: {missing}")
    for fid in ALL_IDS:
        spec = by_id[fid]
        dump(
            output / "tests/golden/schemas" / (wire_id(fid).replace("::", ".") + ".json"),
            wire_ids_in(
                {
                    "function_id": wire_id(spec["id"]),
                    "description": spec["description"],
                    "request_schema": spec["request"],
                    "response_schema": spec["response"],
                }
            ),
        )
    print(f"wrote {len(ALL_IDS)} schema goldens")


# ---- behavior fixtures (Task 6 fills CORPUS/MATRIX; later tasks append) ----

CORPUS: dict[str, str] = {}   # name -> html (loaded from tests/corpus/)
MATRIX: list[dict] = [        # {function, case, request:{...,"html": "<corpus name>"}}
    {"function": "scrapling::regex", "case": "smoke_all",
     "request": {"html": "basic", "pattern": r"\d+"}},
    {"function": "scrapling::regex", "case": "first_group",
     "request": {"html": "basic", "pattern": r"price (\d+)", "first": True}},
    {"function": "scrapling::regex", "case": "no_match_first",
     "request": {"html": "basic", "pattern": r"zzz", "first": True}},
    {"function": "scrapling::regex", "case": "no_match_all",
     "request": {"html": "basic", "pattern": r"zzz"}},
    {"function": "scrapling::regex", "case": "across_fragments_edge",
     "request": {"html": "edge", "pattern": r"CONDITION: \w+"}},
    {"function": "scrapling::regex", "case": "entities_edge",
     "request": {"html": "edge", "pattern": r"A&B"}},
    {"function": "scrapling::regex", "case": "two_groups",
     "request": {"html": "basic", "pattern": r"(\w+) (\d+)"}},
    {"function": "scrapling::regex", "case": "named_backreference",
     "request": {"html": "<p>ab-ab ab-ac</p>", "pattern": r"(?P<word>ab)-(?P=word)"}},
    {"function": "scrapling::regex", "case": "lookbehind_and_conditional",
     "request": {"html": "<p>abcdef ab c</p>", "pattern": r"(?:(?<=abc)def)|((a)?(?(2)b|c))"}},
    {"function": "scrapling::regex", "case": "atomic_and_possessive",
     "request": {"html": "<p>aaab aaaa</p>", "pattern": r"(?>a+)b|a++a"}},
    {"function": "scrapling::regex", "case": "unicode_name_escape",
     "request": {"html": "<p>A—B</p>", "pattern": r"\N{EM DASH}"}},
    {"function": "scrapling::find-by-regex", "case": "unicode_ignorecase_extra",
     "request": {"html": "<p>Kelvin</p>", "pattern": r"kelvin"}},
    {"function": "scrapling::regex", "case": "zero_width_findall",
     "request": {"html": "<p>ab</p>", "pattern": r"x*"}},
    {"function": "scrapling::regex", "case": "w3lib_html4_entities",
     "request": {"html": "<p>&amp;apos; &amp;Copy; &amp;#128; &amp;#129; &amp;unknown; &amp;unknown</p>",
                 "pattern": r"&(?:[A-Za-z]+|#[0-9]+);?"}},
    {"function": "scrapling::regex", "case": "invalid_unterminated_group",
     "request": {"html": "basic", "pattern": "("}},
    {"function": "scrapling::regex", "case": "invalid_range",
     "request": {"html": "basic", "pattern": "[z-a]"}},
    {"function": "scrapling::regex", "case": "invalid_global_flag_position",
     "request": {"html": "basic", "pattern": "a(?i)b"}},
    {"function": "scrapling::regex", "case": "invalid_variable_lookbehind",
     "request": {"html": "basic", "pattern": r"(?<=a*)b"}},
    {"function": "scrapling::regex", "case": "invalid_group_reference",
     "request": {"html": "basic", "pattern": r"\1"}},
    {"function": "scrapling::find-by-text", "case": "exact_default",
     "request": {"html": "basic", "text": "Apple"}},
    {"function": "scrapling::find-by-text", "case": "partial_case",
     "request": {"html": "basic", "text": "aPP", "partial": True}},
    {"function": "scrapling::find-by-text", "case": "case_sensitive_miss",
     "request": {"html": "basic", "text": "apple", "case_sensitive": True}},
    {"function": "scrapling::find-by-text", "case": "clean_match_whitespace",
     "request": {"html": "edge", "text": "bold"}},
    {"function": "scrapling::find-by-text", "case": "no_clean_exact_ws",
     "request": {"html": "edge", "text": "bold", "clean_match": False}},
    {"function": "scrapling::find-by-text", "case": "first_flag",
     "request": {"html": "basic", "text": "Apple", "first": True}},
    {"function": "scrapling::find-by-text", "case": "none",
     "request": {"html": "basic", "text": "Zebra"}},
    {"function": "scrapling::find-by-regex", "case": "default_insensitive",
     "request": {"html": "basic", "pattern": r"price \d+"}},
    {"function": "scrapling::find-by-regex", "case": "sensitive_miss",
     "request": {"html": "basic", "pattern": r"PRICE \d+", "case_sensitive": True}},
    {"function": "scrapling::find-by-regex", "case": "limit_zero",
     "request": {"html": "messy", "pattern": r"\w+", "limit": 0}},
    {"function": "scrapling::find-by-regex", "case": "messy_cards",
     "request": {"html": "messy", "pattern": r"card$"}},
    {"function": "scrapling::find", "case": "by_tag",
     "request": {"html": "basic", "tag": "a"}},
    {"function": "scrapling::find", "case": "by_tag_list",
     "request": {"html": "basic", "tag": ["h1", "p"]}},
    {"function": "scrapling::find", "case": "attrs_exact_whole_value",
     "request": {"html": "messy", "attrs": {"class": "card"}}},   # must NOT match "card wide"
    {"function": "scrapling::find", "case": "attrs_bool_coercion",
     "request": {"html": "edge", "tag": "input", "attrs": {"disabled": ""}}},
    {"function": "scrapling::find", "case": "tag_and_text_regex",
     "request": {"html": "basic", "tag": "a", "text_regex": "Ap", "first": True}},
    {"function": "scrapling::find", "case": "text_regex_only_all_elements",
     "request": {"html": "basic", "text_regex": "price"}},
    {"function": "scrapling::find", "case": "no_filters_error",
     "request": {"html": "basic"}},
    {"function": "scrapling::find", "case": "limit_clamps",
     "request": {"html": "messy", "tag": "li", "limit": 2}},
    {"function": "scrapling::find", "case": "negative_limit_empty",
     "request": {"html": "basic", "tag": "a", "limit": -1}},
    {"function": "scrapling::find-by-text", "case": "clean_trims_trailing_space",
     "request": {"html": "messy", "text": "intro paragraph with"}},
    {"function": "scrapling::find-by-text", "case": "no_clean_keeps_trailing_space",
     "request": {"html": "messy", "text": "intro paragraph with", "clean_match": False}},
    {"function": "scrapling::find", "case": "by_tag_input_attrs_map",
     "request": {"html": "edge", "tag": "input"}},
    {"function": "scrapling::find", "case": "attrs_operator_contains",
     "request": {"html": "basic", "attrs": {"href*": "/a"}}},
    {"function": "scrapling::find", "case": "attrs_operator_prefix",
     "request": {"html": "basic", "attrs": {"href^": "/"}}},
    {"function": "scrapling::find", "case": "tag_html_root",
     "request": {"html": "basic", "tag": "html"}},
    {"function": "scrapling::find", "case": "empty_text_regex_error",
     "request": {"html": "basic", "text_regex": ""}},
    {"function": "scrapling::find", "case": "star_tag_falls_through",
     "request": {"html": "basic", "tag": "*"}},
    {"function": "scrapling::css", "case": "all_default",
     "request": {"html": "basic", "query": "li a"}},
    {"function": "scrapling::css", "case": "first_text",
     "request": {"html": "basic", "query": "li a", "first": True}},
    {"function": "scrapling::css", "case": "first_attr",
     "request": {"html": "basic", "query": "li a", "first": True, "attr": "href"}},
    {"function": "scrapling::css", "case": "attr_miss_in_all",
     "request": {"html": "basic", "query": "li a", "attr": "data-x"}},
    {"function": "scrapling::css", "case": "no_match_modes",
     "request": {"html": "basic", "query": ".nope"}},
    {"function": "scrapling::css", "case": "no_match_first",
     "request": {"html": "basic", "query": ".nope", "first": True}},
    {"function": "scrapling::css", "case": "pseudo_text",
     "request": {"html": "basic", "query": "h1::text", "first": True}},
    {"function": "scrapling::css", "case": "pseudo_attr",
     "request": {"html": "basic", "query": "a::attr(href)"}},
    {"function": "scrapling::css", "case": "invalid_selector",
     "request": {"html": "basic", "query": "li:::bad"}},
    {"function": "scrapling::extract", "case": "mixed_specs",
     "request": {"html": "basic", "selectors": [
         {"name": "title", "css": "h1"},
         {"name": "links", "css": "li a", "attr": "href", "all": True},
         {"name": "names", "css": "li a", "all": True},
         {"name": "price", "regex": r"price (\d+)"},
         {"name": "first_li_html", "css": "li", "html": True},
     ]}},
    {"function": "scrapling::extract", "case": "spec_without_query",
     "request": {"html": "basic", "selectors": [{"name": "x"}, {"name": "y", "all": True}]}},
    {"function": "scrapling::extract", "case": "empty_selectors",
     "request": {"html": "basic", "selectors": []}},
    {"function": "scrapling::extract", "case": "regex_all_spec",
     "request": {"html": "basic", "selectors": [{"name": "nums", "regex": r"\d+", "all": True}]}},
    {"function": "scrapling::xpath", "case": "first_h1",
     "request": {"html": "basic", "query": "//h1", "first": True}},
    {"function": "scrapling::xpath", "case": "all_anchors_text",
     "request": {"html": "basic", "query": "//ul/li/a"}},
    {"function": "scrapling::xpath", "case": "attr_axis_terminal",
     "request": {"html": "basic", "query": "//a/@href"}},
    {"function": "scrapling::xpath", "case": "text_terminal",
     "request": {"html": "basic", "query": "//h1/text()", "first": True}},
    {"function": "scrapling::xpath", "case": "positional",
     "request": {"html": "basic", "query": "//li[2]/a", "first": True}},
    {"function": "scrapling::xpath", "case": "predicate_attr_value",
     "request": {"html": "messy", "query": "//div[@class='card']", }},
    {"function": "scrapling::xpath", "case": "contains_href",
     "request": {"html": "basic", "query": "//a[contains(@href, 'b')]", "first": True}},
    {"function": "scrapling::xpath", "case": "union_doc_order",
     "request": {"html": "basic", "query": "//h1 | //p"}},
    {"function": "scrapling::xpath", "case": "attr_param_on_elements",
     "request": {"html": "basic", "query": "//li/a", "attr": "href"}},
    {"function": "scrapling::xpath", "case": "invalid_syntax",
     "request": {"html": "basic", "query": "//["}},
    {"function": "scrapling::extract", "case": "xpath_specs",
     "request": {"html": "basic", "selectors": [
         {"name": "first_link", "xpath": "//ul/li/a", "attr": "href"},
         {"name": "all_text", "xpath": "//li/a/text()", "all": True},
     ]}},
    # Task 13.5: dom/parser parity batch (blank text, template contents, //
    # axis, empty attr).
    {"function": "scrapling::xpath", "case": "text_runs_main_messy",
     "request": {"html": "messy", "query": "//main/text()"}},
    {"function": "scrapling::xpath", "case": "text_runs_body_messy",
     "request": {"html": "messy", "query": "//body/text()"}},
    {"function": "scrapling::css", "case": "text_pseudo_messy_main",
     "request": {"html": "messy", "query": "main::text"}},
    {"function": "scrapling::xpath", "case": "template_child_step",
     "request": {"html": "messy", "query": "//template/p"}},
    {"function": "scrapling::css", "case": "template_child_css",
     "request": {"html": "messy", "query": "template > p", "first": True}},
    {"function": "scrapling::xpath", "case": "explicit_axis_after_slashslash",
     "request": {"html": "basic", "query": "//descendant::li[2]"}},
    {"function": "scrapling::css", "case": "empty_attr_falls_back_to_text",
     "request": {"html": "basic", "query": "li a", "first": True, "attr": ""}},
    {"function": "scrapling::xpath", "case": "textarea_blank_body_kept",
     "request": {"html": "edge", "query": "//textarea/text()"}},
    {"function": "scrapling::describe", "case": "h1_css",
     "request": {"html": "basic", "query": "h1"}},
    {"function": "scrapling::describe", "case": "no_match",
     "request": {"html": "basic", "query": ".nope"}},
    {"function": "scrapling::describe", "case": "xpath_kind",
     "request": {"html": "basic", "query": "//li[2]/a", "kind": "xpath"}},
    {"function": "scrapling::describe", "case": "weird_kind_is_xpath",
     "request": {"html": "basic", "query": "//h1", "kind": "bogus"}},
    {"function": "scrapling::describe", "case": "text_pseudo",
     "request": {"html": "basic", "query": "h1::text"}},
    {"function": "scrapling::describe", "case": "id_shortcircuit_full",
     "request": {"html": "edge", "query": "#wrap p"}},
    # Task 15: find-similar (structural auto-match + __are_alike scoring).
    {"function": "scrapling::find-similar", "case": "list_items",
     "request": {"html": "basic", "anchor": "li"}},
    {"function": "scrapling::find-similar", "case": "subselectors",
     "request": {"html": "basic", "anchor": "li",
                 "selectors": [{"name": "href", "css": "a", "attr": "href"}]}},
    {"function": "scrapling::find-similar", "case": "anchor_missing",
     "request": {"html": "basic", "anchor": ".nope"}},
    {"function": "scrapling::find-similar", "case": "cards_attr_scoring",
     "request": {"html": "messy", "anchor": "div.card[data-id='1']"}},
    {"function": "scrapling::find-similar", "case": "cards_high_threshold",
     "request": {"html": "messy", "anchor": "div.card[data-id='1']", "similarity_threshold": 0.9}},
    {"function": "scrapling::find-similar", "case": "match_text",
     "request": {"html": "basic", "anchor": "li", "match_text": True}},
    # Fix: match_text scoring must use clean_spaces (deletes \n/\r outright,
    # never trims), not clean (\n/\r -> ' ', trims). Anchor is attribute-bare
    # so __are_alike's attrs contribution is zero either way (target empty,
    # candidate non-empty -> "nothing added") — checks come ONLY from
    # match_text, so the accept/reject verdict is a pure function of which
    # cleaner ran. Anchor's leading text is "line one\nand two " (embedded
    # newline); the sibling's is the literal already-squished "line oneand
    # two " — clean_spaces(anchor) == that string exactly (ratio 1.0);
    # clean(anchor) inserts a space and trims (ratio 0.9677 rounds to 0.97,
    # see fix report) — similarity_threshold sits strictly between the two.
    {"function": "scrapling::find-similar", "case": "match_text_multiline_leading",
     "request": {"html": "edge", "anchor": "#wrap > div", "match_text": True,
                 "similarity_threshold": 0.99}},
    # Task 16: to-markdown (Convertor._extract_content — main-content
    # sanitizer, css_selector scoping, html/markdown/text modes).
    {"function": "scrapling::to-markdown", "case": "markdown_basic",
     "request": {"html": "basic"}},
    {"function": "scrapling::to-markdown", "case": "text_basic",
     "request": {"html": "basic", "format": "text"}},
    {"function": "scrapling::to-markdown", "case": "html_roundtrip",
     "request": {"html": "basic", "format": "html"}},
    {"function": "scrapling::to-markdown", "case": "text_messy_main_only",
     "request": {"html": "messy", "format": "text", "main_content_only": True}},
    {"function": "scrapling::to-markdown", "case": "scoped_css",
     "request": {"html": "messy", "format": "text", "css_selector": "div.card"}},
    {"function": "scrapling::to-markdown", "case": "bad_format",
     "request": {"html": "basic", "format": "pdf"}},
    # Fix review: `_HIDDEN_XPATH`/`.iter()` are self-excluding (`.//`) — the
    # scope root (body, here) must not drop itself even when IT carries the
    # hidden attribute; only matching descendants would. Inline html: no
    # corpus entry names this shape.
    {"function": "scrapling::to-markdown", "case": "hidden_body_self_exempt",
     "request": {"html": '<html><head></head><body aria-hidden="true"><p>keep me</p></body></html>',
                 "format": "text", "main_content_only": True}},
    # markdownify 1.2.3 defaults, after Convertor serializes the selected
    # lxml subtree and BeautifulSoup 4.15 reparses it with `html.parser`.
    # These are strict wrapper fixtures (not calls to markdownify itself):
    # together they cover the tag conversions where htmd differs visibly.
    {"function": "scrapling::to-markdown", "case": "markdown_inline_defaults",
     "request": {"html": '''
         <h1> A *title* </h1><h2>Sub_head</h2><h3>Third\n head</h3>
         <p><strong> bold </strong> <em>em</em> <del>gone</del>
         <code>a``b</code><br><q>quote</q> H<sub>2</sub>O x<sup>2</sup></p>
         <p><a href="https://e.test/a_b">https://e.test/a_b</a>
         <a href="/x" title='a"b'> Link </a>
         <img alt="A" src="i.png" title='t"x'></p><hr>
     '''}},
    {"function": "scrapling::to-markdown", "case": "markdown_blocks_and_lists",
     "request": {"html": '''
         <div><blockquote><p>one<br>two</p><ul><li>A<ul><li>B<ul><li>C</li></ul></li></ul></li><li>D</li></ul></blockquote>
         <ol start="3"><li>Three</li><li><p>Four</p></li></ol>
         <dl><dt> Term one </dt><dd>definition\nline</dd><dt>Next</dt><dd><b>bold</b></dd></dl>
         <pre>\n  a * b\n`tick`\n</pre></div><p>after</p>
     '''}},
    {"function": "scrapling::to-markdown", "case": "markdown_table_and_video",
     "request": {"html": '''
         <figure><table><caption> Cap </caption><thead><tr><th colspan="2">Head <img alt="ALT" src="x"></th></tr></thead>
         <tbody><tr><td>A</td><td><b>B</b><br>C</td></tr></tbody></table>
         <figcaption> Figure text </figcaption></figure>
         <video poster="poster.jpg"><source src="movie.mp4">Trailer</video><video src="only.mp4">Only</video>
     '''}},
    {"function": "scrapling::to-markdown", "case": "markdown_unknown_and_noise_tags",
     "request": {"html": '''
         <!doctype html><!--before--><main data-x="1"><custom-tag> alpha   beta\n gamma </custom-tag>
         <p>before <span>mid</span> after</p><script>bad()</script><style>x{}</style>
         <template><p>T</p></template></main>
     '''}},
    # `<plaintext>` makes the second parse observable. lxml first serializes
    # the rest of the source as escaped plaintext; BeautifulSoup's
    # `html.parser` then treats the serialized closing tags as plaintext too.
    {"function": "scrapling::to-markdown", "case": "markdown_html_parser_second_parse",
     "request": {"html": "<p>before</p><plaintext><b>looks bold</b><p>tail</p></plaintext><p>after</p>"}},
    # Fix review: a `::text`/`::attr()` pseudo-selector match must render
    # through the same pipeline as an element match, not be silently dropped.
    {"function": "scrapling::to-markdown", "case": "pseudo_text_selector_text_mode",
     "request": {"html": "basic", "format": "text", "css_selector": "li a::text"}},
    {"function": "scrapling::to-markdown", "case": "pseudo_text_selector_html_mode",
     "request": {"html": "basic", "format": "html", "css_selector": "li a::text"}},
    # Final-review fix wave: detached `::text` (parsel/scrapy's descendant-
    # text idiom, "a ::text" — self + every descendant, any depth — distinct
    # from attached "a::text", own runs only) and its bare form.
    {"function": "scrapling::css", "case": "detached_text_descendants",
     "request": {"html": "basic", "query": "li ::text"}},
    {"function": "scrapling::css", "case": "detached_text_first",
     "request": {"html": "basic", "query": "h1 ::text", "first": True}},
    # Genuinely bare `::text` (empty stem, no element name at all) — distinct
    # from detached_text_first above, which still has a stem ("h1").
    {"function": "scrapling::css", "case": "bare_detached_text",
     "request": {"html": "basic", "query": "::text"}},
    {"function": "scrapling::extract", "case": "detached_text_spec",
     "request": {"html": "messy", "selectors": [{"name": "card_text", "css": "div.card ::text", "all": True}]}},
    # Libxml HTML recovery and serialization contract.
    {"function": "scrapling::find", "case": "implied_document_nodes",
     "request": {"html": "<title>T</title><p>P", "tag": ["html", "head", "body"]}},
    {"function": "scrapling::find", "case": "duplicate_and_boolean_attributes",
     "request": {"html": "<input z=1 disabled a='' z=2 checked=checked>", "tag": "input"}},
    {"function": "scrapling::extract", "case": "malformed_table_recovery",
     "request": {"html": "<table><td>A<td>B<div>C", "selectors": [{"name": "table", "css": "table", "html": True}]}},
    {"function": "scrapling::extract", "case": "misnested_formatting_recovery",
     "request": {"html": "<p><b>one<i>two</b>three</i>tail", "selectors": [{"name": "body", "css": "body", "html": True}]}},
    {"function": "scrapling::extract", "case": "foreign_content_serialization",
     "request": {"html": "<svg viewBox='0 0 1 1'><foreignObject><DIV xlink:href='x'>T</DIV></foreignObject></svg>",
                 "selectors": [{"name": "svg", "css": "svg", "html": True}]}},
    {"function": "scrapling::extract", "case": "entities_and_invalid_codepoints",
     "request": {"html": "<p>&copy; &apos; &#0; &#xD800; &notanentity;</p>",
                 "selectors": [{"name": "text", "xpath": "//p/text()", "all": True},
                               {"name": "html", "css": "p", "html": True}]}},
    {"function": "scrapling::extract", "case": "comments_removed_and_text_merged",
     "request": {"html": "<p>a<!--gone-->b<![CDATA[c]]>d</p>",
                 "selectors": [{"name": "text", "xpath": "//p/text()", "all": True},
                               {"name": "html", "css": "p", "html": True}]}},
    {"function": "scrapling::extract", "case": "template_nested_content",
     "request": {"html": "<template><table><td>T</template><p>P", "selectors": [
         {"name": "template", "css": "template", "html": True},
         {"name": "td", "xpath": "//template//td", "all": True},
     ]}},
    # CSS is translated with cssselect 1.5 then evaluated as XPath.
    {"function": "scrapling::css", "case": "sibling_text_pseudo",
     "request": {"html": "<h1>H</h1>x<p>A<b>B</b>C</p><p>D</p>", "query": "h1 + p::text"}},
    {"function": "scrapling::css", "case": "general_sibling_attr_pseudo",
     "request": {"html": "<h1>H</h1><p data-x='a'>A</p><p data-x='b'>B</p>", "query": "h1 ~ p::attr(data-x)"}},
    {"function": "scrapling::css", "case": "nth_not_and_attribute_operators",
     "request": {"html": "<ul><li class='x' data-v='en-us'>A</li><li data-v='en'>B</li><li data-v='fr'>C</li></ul>",
                 "query": "li:nth-child(2):not(.x)[data-v|='en']", "first": True}},
    {"function": "scrapling::css", "case": "grouped_selector_document_order",
     "request": {"html": "<p>P</p><h1>H</h1><p>Q</p>", "query": "h1, p"}},
    {"function": "scrapling::find-similar", "case": "subselector_scope_cannot_escape",
     "request": {"html": "<main><section><a>inside</a></section></main><a>outside</a>", "anchor": "section",
                 "selectors": [{"name": "escaped", "css": "body a", "all": True},
                               {"name": "inside", "css": "a", "all": True}]}},
    # XPath 1.0 axes, functions, coercion, scalar quirks, and errors.
    {"function": "scrapling::xpath", "case": "ancestor_axis_reverse_position",
     "request": {"html": "<main id='m'><section id='s'><p>P</p></section></main>", "query": "//p/ancestor::*[1]/@id"}},
    {"function": "scrapling::xpath", "case": "preceding_axis_reverse_position",
     "request": {"html": "<p id='a'>A</p><p id='b'>B</p><p id='c'>C</p>", "query": "//p[@id='c']/preceding::p[1]/@id"}},
    {"function": "scrapling::xpath", "case": "following_axis_document_order",
     "request": {"html": "<div><i>A</i></div><p>B</p><p>C</p>", "query": "//i/following::p/text()"}},
    {"function": "scrapling::xpath", "case": "attribute_wildcard_order",
     "request": {"html": "<p z='1' a='2' m='3'>P</p>", "query": "//p/@*"}},
    {"function": "scrapling::xpath", "case": "predicate_string_functions",
     "request": {"html": "<p>  Alpha beta  </p><p>Gamma</p>",
                 "query": "//p[starts-with(normalize-space(.), 'Alpha') and string-length(normalize-space(.)) = 10]"}},
    {"function": "scrapling::xpath", "case": "predicate_arithmetic_and_round",
     "request": {"html": "<i>1</i><i>2</i><i>3</i><i>4</i>", "query": "//i[position() = round(last() div 2)]"}},
    {"function": "scrapling::xpath", "case": "global_parenthesized_position",
     "request": {"html": "<div><p>A</p><p>B</p></div><section><p>C</p></section>", "query": "(//p)[2]"}},
    {"function": "scrapling::xpath", "case": "string_scalar_splits_into_text_nodes",
     "request": {"html": "<p>Alpha</p>", "query": "string(//p)"}},
    {"function": "scrapling::xpath", "case": "false_scalar_becomes_empty",
     "request": {"html": "<p>Alpha</p>", "query": "boolean(//nope)"}},
    {"function": "scrapling::xpath", "case": "true_scalar_type_error",
     "request": {"html": "<p>Alpha</p>", "query": "boolean(//p)"}},
    {"function": "scrapling::xpath", "case": "number_scalar_type_error",
     "request": {"html": "<p>Alpha</p>", "query": "count(//p)"}},
    {"function": "scrapling::xpath", "case": "unknown_function_error",
     "request": {"html": "<p>Alpha</p>", "query": "no-such-function()"}},
]


def gen_behavior(output: Path = WORKER) -> None:
    from src.handlers import create_handlers

    handlers = create_handlers(lambda: {})
    for name in ("basic", "edge", "messy"):
        CORPUS[name] = (WORKER / "tests/corpus" / f"{name}.html").read_text()
    n = 0
    for entry in MATRIX:
        req = dict(entry["request"])
        # Named corpus file, or (to-markdown's scope-root/pseudo-selector
        # fixtures) inline HTML that names no corpus entry at all.
        req["html"] = CORPUS.get(req["html"], req["html"])
        fn = entry["function"]
        try:
            resp = {"ok": asyncio.run(handlers[fn.split("::", 1)[1].replace("-", "_")](req))}
        except Exception as exc:  # noqa: BLE001 — error text is part of the contract
            resp = {"err": str(exc)}
        dump(
            output / "tests/golden/behavior" / fn.split("::")[1] / (entry["case"] + ".json"),
            {
                "function": wire_id(fn),
                "case": entry["case"],
                "request": req,
                **resp,
            },
        )
        n += 1
    print(f"wrote {n} behavior fixtures")


class _BrowserFixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
        body = (WORKER / "tests/corpus/browser_visual.html").read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args) -> None:
        pass


class _ThreadingServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


def gen_browser(output: Path = WORKER) -> None:
    """Generate deterministic screenshot fixtures through the public wrapper."""
    from PIL import Image
    from src.handlers import create_handlers

    server = _ThreadingServer(("127.0.0.1", 0), _BrowserFixtureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    origin = f"http://127.0.0.1:{server.server_port}"
    cfg = {"defaults": {"headless": True, "network_idle": False, "proxy": "", "include_html": False}}
    handler = create_handlers(lambda: cfg)["screenshot"]
    cases = [
        ("dynamic-viewport-png", {"fetcher": "dynamic", "format": "png", "full_page": False}),
        ("dynamic-full-png", {"fetcher": "dynamic", "format": "png", "full_page": True}),
        ("stealthy-viewport-png", {"fetcher": "stealthy", "format": "png", "full_page": False}),
        ("stealthy-full-jpeg", {"fetcher": "stealthy", "format": "jpeg", "full_page": True}),
    ]
    destination = output / "tests/golden/browser"
    destination.mkdir(parents=True, exist_ok=True)
    records = []
    try:
        for name, options in cases:
            request = {"url": f"{origin}/visual", "retries": 1, **options}
            response = asyncio.run(handler(request))
            blocks = []
            image_index = 0
            for block in response["content"]:
                if block["type"] != "image":
                    blocks.append({**block, "text": block["text"].replace(origin, "{origin}")})
                    continue
                image_index += 1
                data = base64.b64decode(block["data"])
                suffix = "jpg" if block["mime"] == "image/jpeg" else "png"
                filename = f"{name}-{image_index}.{suffix}"
                (destination / filename).write_bytes(data)
                with Image.open(destination / filename) as image:
                    dimensions = [image.width, image.height]
                blocks.append({
                    "type": "image",
                    "mime": block["mime"],
                    "file": filename,
                    "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "dimensions": dimensions,
                })
            records.append({
                "case": name,
                "request": {**request, "url": "{origin}/visual"},
                "response": {
                    "content": blocks,
                    "url": response["url"].replace(origin, "{origin}"),
                    "mime": response["mime"],
                },
            })
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    dump(destination / "manifest.json", {"cases": records})
    print(f"wrote {len(cases)} browser fixtures")


def check_browser() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        output = Path(tmp)
        gen_browser(output)
        fresh = output / "tests/golden/browser"
        committed = WORKER / "tests/golden/browser"
        names = {path.name for path in fresh.iterdir()} | {path.name for path in committed.iterdir()}
        drift = [name for name in names if not (fresh / name).exists() or not (committed / name).exists()
                 or (fresh / name).read_bytes() != (committed / name).read_bytes()]
        if drift:
            raise SystemExit(f"browser goldens are stale: {sorted(drift)}")
    print("browser goldens are current")


def check() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        output = Path(tmp)
        gen_schemas(output)
        gen_behavior(output)
        generated = output / "tests/golden"
        committed = WORKER / "tests/golden"
        drift = []
        for fresh in generated.rglob("*.json"):
            relative = fresh.relative_to(generated)
            checked_in = committed / relative
            if not checked_in.exists() or fresh.read_bytes() != checked_in.read_bytes():
                drift.append(str(relative))
        generated_behavior = {
            path.relative_to(generated / "behavior") for path in (generated / "behavior").rglob("*.json")
        }
        committed_behavior = {
            path.relative_to(committed / "behavior") for path in (committed / "behavior").rglob("*.json")
        }
        drift.extend(str(path) for path in generated_behavior ^ committed_behavior)
        if drift:
            raise SystemExit(f"goldens are stale: {sorted(set(drift))}")
    print("goldens are current")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "mode",
        choices=["schemas", "behavior", "browser", "check", "browser-check"],
        nargs="?",
        default="schemas",
    )
    parser.add_argument(
        "--parser-runtime",
        action="store_true",
        help="verify immutable parser inputs but not host fonts, locale, or timezone",
    )
    args = parser.parse_args()
    random.seed(0)
    if os.environ.get("PYTHONHASHSEED") != "0":
        os.execve(sys.executable, [sys.executable, *sys.argv], {**os.environ, "PYTHONHASHSEED": "0"})
    verify_command = [sys.executable, HERE / "verify_oracle.py"]
    if args.parser_runtime:
        verify_command.append("--parser-runtime")
    subprocess.run(verify_command, check=True)
    {
        "schemas": gen_schemas,
        "behavior": gen_behavior,
        "browser": gen_browser,
        "check": check,
        "browser-check": check_browser,
    }[args.mode]()
