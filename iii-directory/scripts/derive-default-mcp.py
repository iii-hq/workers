#!/usr/bin/env python3
"""Derive prompts/default-mcp.md from prompts/default.md.

`default-mcp` is the `default` identity for agents that reach the engine
through the mcp-server worker (an MCP client such as Claude Code): `iii_call`
instead of `agent_call`, the `iii_observe` tools instead of
`harness::observe`, and none of the harness- or console-only parts. Every
edit below must match exactly once, so a change to default.md that moves an
anchor fails here instead of drifting silently.

    python3 scripts/derive-default-mcp.py          # write the file
    python3 scripts/derive-default-mcp.py --check  # exit 1 if it is stale
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "prompts" / "default.md"
DST = ROOT / "prompts" / "default-mcp.md"

EDITS = [
    # Frontmatter: hidden (a console session has the harness), no placeholder.
    (
        'name: Default\ndescription: Use for general questions and development tasks in your project.\ncomposer_placeholder: "Explain how this project works…"\n',
        'name: Default (MCP)\ndescription: "The default profile for agents that reach iii through the mcp-server worker (MCP clients such as Claude Code): iii_call and the iii_observe tools instead of the harness."\nhidden: true\n',
    ),
    ("You are an iii agent.\n", "You are an iii agent, connected to the engine through an MCP server (the `mcp-server` worker).\n"),
    ("Write all user-facing progress, tool descriptions, questions, event\nnotifications, and final text in it.", "Write all user-facing progress, questions,\nand final text in it."),
    (
        "You have exactly one tool: `agent_call`. It calls a function on the iii engine. It takes\nthree arguments: `function` (a namespaced id like `worker::function`), `description` (a short\nuser-facing description of the action in the response language), and `payload`\n(a JSON OBJECT with the function's arguments). Everything you do happens through\n`agent_call`.",
        "Your iii tools are the MCP tools named `iii_*`. `iii_call` calls a function on the iii engine:\nit takes `function_id` (a namespaced id like `worker::function`) and `payload` (a JSON OBJECT\nwith the function's arguments). Everything you do on the engine happens through `iii_call`;\nthe other `iii_*` tools observe functions, store secrets, and read profiles and skills.",
    ),
    (
        "When you need the user to choose between discrete options, call `harness::ask { questions: [{ header,\nquestion, multi_select?, options: [{ label, description? }] }] }` (1-4 questions, 2-4 options each; the\nUI adds a free-text \"Other\") instead of writing the question or a numbered list of choices in text, and\ndo not also write the question in text. The card is shown, your turn ends, and the answer arrives as\nthe user's next message. Sub-agents and structured-output turns cannot ask: report blocked instead.",
        "When you need the user to choose between discrete options, use your client's own way of asking\n(its question tool, if it has one); otherwise ask in text. `harness::ask` and the other\n`harness::*` functions belong to the iii harness and are not available to you: do not call them.",
    ),
    (
        "A call marked `pre-verified` by a Harness runtime block or update, with its exact id and\npayload shape, already satisfies Steps 1 and 2; call it directly without discovery or\n`engine::functions::info`.\n\n",
        "",
    ),
    ("The directory worker installs\nalongside the harness, so this is the default discovery path;", "The directory worker is normally\ninstalled, so this is the default discovery path;"),
    (
        "Step 3. Call the function. Set `description` to a concise action label in the response\nlanguage. `payload` is a JSON OBJECT,",
        "Step 3. Call the function with `iii_call { function_id, payload }`. `payload` is a JSON OBJECT,",
    ),
    ("Never resend the same `function` +\n`payload` unchanged.", "Never resend the same `function_id` +\n`payload` unchanged."),
    (
        "you are already connected to the engine, and a second client started from a worker's shell\ninherits that worker's identity and is refused. Everything goes through `agent_call`.",
        "you are already connected to the engine through this MCP server, with the access its operator\ngranted. Everything goes through `iii_call`.",
    ),
    (
        "- `harness::observe { function_id, args?, filters?, target? }` — make a function run, or\n  this session wake (omit `target`), whenever `function_id` runs; `harness::unobserve { id }`\n  removes it, and `harness::observers::list` shows this session's observers.\n  `engine::functions::info` on the observed function gives its `args_format` (the args its\n  worker reads; none when absent) and its `response_format` (the event you receive).",
        "- `iii_observe { function_id, args?, filters?, target_function_id? }` (an MCP tool) — each\n  invocation of `function_id` becomes an observation, buffered for this session and read with\n  `iii_observations_poll`, or handed to `target_function_id`, which the engine runs with it;\n  `iii_observations_sse` streams observations on the session's GET stream instead.\n  `iii_unobserve { observer_id }` removes one, and `iii_observers` lists this session's.\n  `engine::functions::info` on the observed function gives its `args_format` (the args its\n  worker reads; none when absent) and its `response_format` (the observation you receive).",
    ),
    (
        "  `engine::observers::list` shows the observers that already exist.\n",
        "  `engine::observers::list` shows the observers that already exist.\n- `fp::pipe` and `fp::batch` — higher-order functions that call other functions for you:\n  `fp::pipe` runs a chain of calls worker-side, each result passed to the next, so a large\n  value never passes through you; `fp::batch` runs independent calls at once.\n- `directory::skills::index` — the skills relevant to this system: guidance documents, not\n  one per worker. Use it sparingly, when a task needs guidance you lack.\n",
    ),
    (
        "When you mention a function in text for the user, write @fn(<function_id>), for example\n@fn(engine::functions::info); in the `function` field and inside code blocks, use the bare\nid.\n\n`#file(<path>)` in a user message is a reference to a file under the working directory, not\nits content — read it with `coder::read-file` when you need it. Only a line window\n(`#file(<path>:<from>-<to>)`) arrives with those lines already attached.\n\n`@<name>(id=\"<id>\")` in a message is a mention of one item a worker owns (a ticket, a session,\na trace…), not its content. A `<mentions>` block after the message summarizes each one and,\nwhen allowed, gives a pre-verified `details` call for the full item; `<mention_providers>`\nlists the names you may write in a reply, always with an id a function returned.\n\n",
        "When you mention a function in text for the user, write its bare id in backticks, for\nexample `engine::functions::info`.\n\n",
    ),
]


def derive(text: str) -> str:
    for old, new in EDITS:
        count = text.count(old)
        if count != 1:
            sys.exit(f"derive-default-mcp: expected one match, found {count}:\n{old[:120]!r}")
        text = text.replace(old, new, 1)
    return text


def main() -> None:
    out = derive(SRC.read_text())
    if "--check" in sys.argv:
        if not DST.exists() or DST.read_text() != out:
            sys.exit("prompts/default-mcp.md is stale: run python3 scripts/derive-default-mcp.py")
        return
    DST.write_text(out)


if __name__ == "__main__":
    main()
