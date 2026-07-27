---
name: harness/building
description: >-
  When nothing registered fits: search and install workers from the public registry, edit code files with coder::*, and read the SDK reference before writing any worker code.
---

# Building new things

First check what already exists with `engine::functions::list` and
`engine::triggers::list`. Do not carry patterns from other ecosystems (standalone servers,
package managers, ad-hoc processes) — iii has its own way, and foreign patterns do not run
here.

If no registered function fits, search the public registry:

Step 1. Call `directory::registry::workers::list { search: "<capability>" }` to find a
worker.
Step 2. Call `directory::registry::workers::info { name: "<name>" }` to see its functions,
config, and dependencies before installing. Both registry calls are documented here, so you
do not need to fetch their contracts first.
Step 3. Installing runs new code, so say what you are about to install and why. Then install
it with `worker::add { source: { kind: "registry", name: "<name>" } }`.
Step 4. Check it worked: confirm the new function ids appear with
`engine::functions::list { prefix: "<worker>::" }`. Then fetch each contract with
`engine::functions::info` before calling. The registry detail is a preview, not the contract.

If no `directory::*` function is registered: look in `worker::list` for a stopped
directory worker and start it. If it is not installed, install it with
`worker::add { source: { kind: "registry", name: "iii-directory" } }`. If the registry is
still unreachable, tell the user and continue with what is registered.

<example>
user: Email me the weekly report.
assistant: [calls engine::functions::list { search: "email" } — nothing registered fits]
[calls directory::registry::workers::list { search: "email" } and finds "email"]
[calls directory::registry::workers::info { name: "email" } to judge fit before installing]
I am installing the "email" worker from the public registry so I can send the report.
[calls engine::functions::info { function_id: "worker::add" } for the install contract]
[calls worker::add { source: { kind: "registry", name: "email" } }]
[calls engine::functions::list { prefix: "email::" } — the new function ids appear]
[calls engine::functions::info { function_id: "email::send" } to get the contract]
[calls agent_trigger with function: "email::send", payload: { ...per the contract }]
</example>

To create, edit, move, or delete code files, use the `coder::*` functions — they are
served by the shell worker (no separate install). Confirm they are available with
`engine::functions::list { prefix: "coder::" }`. Its functions include `coder::read-file`,
`coder::search`, `coder::list-folder`, `coder::tree`, `coder::create-file`,
`coder::update-file`, `coder::move`, and `coder::delete-file` — the prefix check shows
the full inventory. Use `coder::move` for renames and moves, never delete-then-recreate. Plain
file browsing outside code work (like `shell::fs::ls`) is still fine. Fetch each contract
first, as always.

To author a worker: import ONLY `registerWorker` from the SDK. Its return value has the
methods `registerFunction`, `registerTrigger`, and `trigger` — call them as
`iii.registerFunction(...)`. They are NOT top-level exports. Destructuring them throws
`TypeError: registerFunction is not a function`. Give every function a `description`,
`request_format`, and `response_format` — that becomes the contract that
`engine::functions::info` shows to callers. Before writing code, inspect the runtime with
`engine::workers::info { name }`.

Before you write the FIRST line of worker code — a new worker, or new registrations on an
existing one — read the SDK reference for the language you will use. Do not write SDK code
from memory: names and config keys from memory are often wrong, and a trigger registered with
wrong keys never fires. Fetch the reference as Markdown.
Pick the URL for the implementation language:
- https://iii.dev/docs/reference/sdk-node — Node/TypeScript
- https://iii.dev/docs/reference/sdk-python — Python
- https://iii.dev/docs/reference/sdk-rust — Rust
- https://iii.dev/docs/reference/sdk-browser — browser
- https://iii.dev/docs/reference/engine-protocol — the raw WebSocket protocol, for any other
  language
Add `.md` to a docs URL to get the raw markdown source. If a fetch fails, use the index at
https://iii.dev/docs/llms.txt — it lists every doc page. If the docs stay unreachable,
say so and proceed with extra care: verify every registration with a real call. Do not fetch
docs for an ordinary call — `engine::functions::info` is the reference for calling
functions.
