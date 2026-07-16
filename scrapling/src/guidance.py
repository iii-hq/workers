"""`scrapling::inject-guidance` — a harness `pre_generate` hook that appends the
scrapling usage guidance to the agent system prompt, only while this worker is
connected. The binding dies with the worker, so the guidance is presence-gated
for free: a deployment without scrapling never pays for it.

The bind is one shot: if the harness is not up yet, the engine parks the
binding as a pending intent and activates it when the trigger type registers
(recoverable triggers, iii #1962) — and re-parks/re-activates it across
harness restarts. Nothing to watch or retry.
Mirrors web/src/functions/inject_guidance.rs + configuration.rs.
"""

from __future__ import annotations

import logging
from typing import Any

log = logging.getLogger("scrapling.guidance")

HOOK_ID = "scrapling::inject-guidance"
HOOK_TRIGGER_TYPE = "harness::hook::pre-generate"

# The single canonical copy of the scrapling usage guidance (same role as
# WEB_GUIDANCE in the web worker). Pure USAGE guidance: the hook only fires
# while this worker is present, so no "look for it / install it" text.
GUIDANCE = (
    "For scraping web pages — pulling structured data (titles, prices, links, listings) out of HTML — "
    "use `scrapling::*`, never hand-rolled fetch+regex (plain JSON/REST APIs stay on `web::fetch`). "
    "Escalate fetchers only as needed: `scrapling::fetch` (fast HTTP, TLS impersonation) → "
    "`scrapling::dynamic-fetch` (real Chromium, for JS-rendered pages) → `scrapling::stealthy-fetch` "
    "(Camoufox anti-bot; pass `solve_cloudflare: true` for Cloudflare walls). ALWAYS pass inline "
    "`selectors` (`[{name, css|xpath|regex, attr?, all?}]`) so the response carries compact `extracted` "
    "fields instead of raw HTML that floods your context; set `include_html: true` only when you truly "
    "need the source — it returns the FULL page plus headers/cookies, and NEVER re-fetch a page you "
    "already rendered just to clean or reformat it: work with what you have (trim in context, or run the "
    "pure parsers on HTML from a fetch that already included it). Fetch many pages in one call with "
    "`urls: [...]`. On HTML you already have, use the "
    "pure parsers (no network, no approval needed): `scrapling::extract` (declarative selector list), "
    "`scrapling::css` / `scrapling::xpath` / `scrapling::regex` (one query), `scrapling::find` "
    "(tag/attribute filters), `scrapling::find-by-text` / `scrapling::find-by-regex` (locate by visible "
    "text), `scrapling::find-similar` (structural auto-match from one example), and `scrapling::describe` "
    "(generated selectors + DOM context for an element). To READ a page as compact Markdown instead of "
    "raw HTML that floods context, pass `format: markdown` to any fetch — add `main_content_only: true` "
    "to strip chrome, and `css_selector` to scope the render to one subtree (e.g. a page's content div; "
    "the fix when chrome still leaks into the render — never re-fetch with `include_html` just to clean "
    "up). `scrapling::to-markdown` takes the same knobs on HTML you already have. "
    "`scrapling::screenshot` returns the rendered page as image blocks you "
    "can see directly. "
    "Fetch exact request shapes via "
    "`engine::functions::info` before the first call. For selectors that must survive a site "
    "redesign, pass `adaptive: true` (+ `adaptive_domain` = the page URL) to `scrapling::extract` / "
    "`css` / `xpath`: the first run saves each element's identity and later runs relocate it even when "
    "the CSS path changes. To crawl a site behind a login or reuse cookies/one browser across many "
    "pages, open a `scrapling::session-open` ({type: http|dynamic|stealthy}) and pass its `session_id` "
    "to `scrapling::session-fetch`; call `scrapling::session-close` when done (list live ones with "
    "`scrapling::session-list`). To walk a whole site, `scrapling::crawl` ({start_urls, selectors, "
    "max_pages, max_depth, same_domain}) BFS-follows links and streams each extracted item back."
)

PRE_GENERATE_REQUEST: dict[str, Any] = {
    "type": "object",
    "properties": {
        "generate": {
            "type": "object",
            "properties": {
                "system_prompt": {"type": "string", "description": "system prompt assembled so far"},
            },
        },
    },
}
PRE_GENERATE_RESPONSE: dict[str, Any] = {
    "type": "object",
    "properties": {
        "mutations": {
            "type": "object",
            "properties": {
                "system_prompt": {"type": "string", "description": "full replacement prompt (base + guidance)"},
            },
        },
    },
    "required": ["mutations"],
}


def mutations_for(base: str) -> dict[str, Any]:
    """Build the pre_generate mutations for a given base prompt.

    An empty base (missing/renamed `generate.system_prompt` → schema drift) must
    PRESERVE the harness's assembled prompt: emit no `system_prompt` key at all —
    the harness applies the mutation only when the key is present. For a real
    base, return the FULL prompt (the harness overwrites, it does not merge).
    """
    if not base:
        return {"mutations": {}}
    return {"mutations": {"system_prompt": f"{base}\n\n{GUIDANCE}"}}


async def inject_guidance(payload: dict[str, Any] | None) -> dict[str, Any]:
    """`pre_generate` hook entrypoint. Bound fail_open — lenient on every field."""
    generate = (payload or {}).get("generate") or {}
    base = generate.get("system_prompt") if isinstance(generate, dict) else ""
    return mutations_for(base if isinstance(base, str) else "")


def setup(iii: Any) -> None:
    """Register the hook function and bind it one-shot. Call once at boot."""
    iii.register_function(
        HOOK_ID,
        inject_guidance,
        description=(
            "Internal pre_generate hook: appends scrapling usage guidance to the agent system prompt. "
            "Bound to harness::hook::pre-generate at worker startup; not called directly."
        ),
        request_format=PRE_GENERATE_REQUEST,
        response_format=PRE_GENERATE_RESPONSE,
        metadata={"internal": True},
    )

    # on_error fail_open is MANDATORY: pre_generate defaults fail-CLOSED, and a
    # missing guidance line must never abort a turn.
    iii.register_trigger({"type": HOOK_TRIGGER_TYPE, "function_id": HOOK_ID, "config": {"on_error": "fail_open"}})
    log.info("scrapling pre-generate hook bound (guidance injection active)")
