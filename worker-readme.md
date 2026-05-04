# Worker README guide

How to write the `README.md` for a worker in this repo.

A worker's README is consumer-facing: it lands as the `readme` field on
`POST /publish`, it is what someone reads after `iii worker info <name>`,
and it is the page on the registry. Treat it as a **how-to**, not a
reference. The reference lives in the code — rustdoc, source links,
`iii worker info`, IDE autocomplete on function ids.

## What the README answers

In order, four questions:

1. **What does this worker do?** One paragraph.
2. **How do I install it?** One command.
3. **How do I run it?** One command.
4. **How do I make my first useful call?** A short, runnable example.

Anything else is a follow-up section. The README is **not**:

- A function reference. Use rustdoc / `iii worker info` / source links.
- A request/response schema dump. Same.
- A deep architecture document. That belongs in a sibling design doc
  (`<worker>/project-*.md` or similar).
- A build-from-source recipe for end users. Source builds belong in the
  optional `## Local development & testing` section.

## Required structure

Every worker README has these sections, in this order:

1. Title + one-paragraph summary
2. `## Install`
3. `## Run`
4. `## Quickstart` (or a more specific name like `## Build a worker that …`)
5. `## Configuration`

Optional, only when relevant:

- `## Custom trigger types` — if your worker emits subscribable trigger
  types for sibling workers.
- `## Migration notes` — for renamed function ids or breaking payload
  changes in a recent release.
- `## Local development & testing` — `cargo run`, `cargo test`, BDD
  tags, end-to-end smoke instructions for contributors.

For longer READMEs (>~200 lines), add a `## Table of contents` after
the summary. Short ones don't need one.

## Section: title + summary

```markdown
# <worker>

One paragraph. What this worker does in plain English, who calls it,
and the single most important thing it gives you.
```

Lead with the user-facing value, not the implementation. If your worker
ships with a sibling that most users will install too (e.g.
[`skills`](skills) → [`mcp`](mcp)), name and link to the
sibling in the same paragraph.

## Section: Install

Always exactly one command:

````markdown
```bash
iii worker add <name>
```
````

Where `<name>` is the value of `iii.worker.yaml.name` (which equals the
folder name; see [`AGENTS-NEW-WORKER.md`](AGENTS-NEW-WORKER.md) §1).
That is the whole user-facing install — no source build, no
`sudo install`, no `--manifest | jq` verification step. `iii worker add`
fetches the binary, writes a config block into `~/.iii/config.yaml`,
and the engine starts the worker on the next `iii start`.

### Companion workers

If your worker is most useful next to a sibling — e.g. `mcp` surfaces
`skills`-registered content to MCP clients — include the companion as
a second `iii worker add` block with one sentence explaining what it
unlocks:

````markdown
```bash
iii worker add skills
```

To surface every registered skill (and slash-command prompt) to MCP
clients (Cursor, Claude Code, Codex, Claude Desktop, …), add the
[mcp](../mcp) worker as well:

```bash
iii worker add mcp
```
````

### What does NOT go here

Do not show `cargo build` / `cargo install` / `From source` blocks in
the user-facing flow. Source builds are for contributors; gate them
behind `## Local development & testing`.

Do not show binary-verification steps (`<bin> --help`,
`<bin> --manifest | jq`). The registry already verifies the manifest
on publish; the user does not need to.

## Section: Quickstart

This is the meat of the README. Get the reader from "the worker is
running" to "I made a first useful call" as fast as possible. Aim for
≤30 lines of code in the primary example.

Pick the audience-appropriate language (Rust, Node, or Python) and show
three things:

- The function id to call.
- A realistic payload.
- The expected output shape.

Example skeleton (Rust):

````markdown
## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii.trigger(TriggerRequest {
        function_id: "<worker>::<verb>".into(),
        payload: json!({ /* … */ }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("{result:#?}");
    Ok(())
}
```
````

Keep it to **1–3 functions**, picked for their introductory value, not
their breadth. If your worker has 20 functions, show the 2 most users
will hit first; the rest live in code with rustdoc.

If your worker participates in a larger flow (skills + mcp, fts +
agentmemory, …), the Quickstart can be a walkthrough of the
cross-worker handshake. [`skills/README.md`](skills/README.md)
is the canonical example: skill registration, sub-skill markdown
links, and prompt registration told as a single end-to-end story.

## Section: Configuration

Show the `config.yaml` block annotated with one-line comments. Keep it
to the fields a user is likely to tune.

````markdown
## Configuration

```yaml
data_dir: ${WORKER_DATA:~/.iii/data/<worker>}   # where to persist state
timeout_ms: 5000                                 # per-call upper bound
```
````

If your worker has many internal/advanced config keys, point at the
defaults in code rather than paginating every field:

```markdown
Other keys (and their defaults) live in [`src/config.rs`](src/config.rs).
```

## Optional: Custom trigger types

Include this section only if your worker publishes custom trigger
types other workers can subscribe to. Show a table:

```markdown
## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `<worker>::on-change` | After every mutation | `{ "op": "register" \| "unregister", "id": "..." }` |
```

This is one of the few cases the README **should** carry a reference
table — sibling-worker authors need the trigger names + payload shapes
to write `iii.register_trigger(...)` calls.

## Optional: Migration notes

If you renamed function ids, removed config keys, or changed payload
shapes between versions, document the upgrade path. Keep it terse.

```markdown
## Migration: pre-split function ids

Function ids you used to call as `mcp::register-skill` /
`mcp::register-prompt` are now `skills::register` /
`prompts::register`. Payloads are unchanged.
```

## Optional: Local development & testing

The escape hatch for contributors. Anything that uses `cargo` directly
goes here, never in `## Install` or `## Run`.

````markdown
## Local development & testing

### Run from source

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

### Tests

```bash
cargo test
```
````

If your test suite has tags or scenarios worth surfacing (BDD harness,
optional engine-required suites), document them here. See
[`skills/README.md`](skills/README.md) for a worked example.

## Skeleton

Drop this in as `<worker>/README.md` and fill in the placeholders. Use
a four-backtick outer fence (or copy the body unfenced) when pasting,
since the README itself contains triple-backtick code blocks.

````markdown
# <worker>

One paragraph: what the worker does, who calls it, and the single
most important thing it gives you.

## Install

```bash
iii worker add <worker>
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Run

```bash
iii start
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii.trigger(TriggerRequest {
        function_id: "<worker>::<verb>".into(),
        payload: json!({ "key": "value" }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("{result:#?}");
    Ok(())
}
```

## Configuration

```yaml
key: value   # one line of intent per field
```

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
cargo test
```
````

## Worked examples

- [`skills/README.md`](skills/README.md) — install (with
  companion `mcp`), end-to-end how-to, minimal configuration, dev
  section. Canonical reference for this guide.

