# canvas

Diagrams an agent can come back to. A canvas stores its editable source —
mermaid text, or an excalidraw scene for a freeform whiteboard — under a
stable 8-character id, so the architecture sketch drawn in one turn can be
revised ten turns later and every earlier link to it keeps working. The
console does the drawing: a `canvas::*` call renders in chat as the live
diagram rather than a wall of source, and a canvas page lists, edits and
redraws everything stored. For generated mermaid there is a primer on the
bus (`canvas::syntax`) and a parse check (`canvas::validate`), so source
validates on the first try instead of guessing dialect details.

## Install

```bash
iii worker add canvas
iii worker add state   # required — canvas records live here
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker the next time it
boots.

### Companion workers

| Worker | Why |
|---|---|
| [`state`](https://github.com/iii-hq/workers/tree/main/state) | Required. Every canvas record lives in its `canvas` scope; the worker holds nothing in process memory, so a restart loses nothing. |
| [`console`](https://github.com/iii-hq/workers/tree/main/console) | Optional. Renders the `#/ext/canvas` page and draws `canvas::*` calls as live diagrams in chat. |

## Quickstart

Get the primer, validate, store:

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());
    let call = |id: &str, payload| iii.trigger(TriggerRequest {
        function_id: id.into(), payload, action: None, timeout_ms: Some(10_000),
    });

    // The dialect the renderer actually supports, a working example per family.
    let primer = call("canvas::syntax", json!({ "family": "sequence" })).await?;
    println!("{}", primer["families"][0]["example"]);

    let source = "sequenceDiagram\n    Client->>Engine: trigger\n    Engine-->>Client: result";

    // Parse before storing, so a broken diagram never lands in the store.
    let verdict = call("canvas::validate", json!({
        "format": "mermaid", "source": source,
    })).await?;
    // { "valid": true, "family": "sequence", "issues": [] }
    assert_eq!(verdict["valid"], true);

    let record = call("canvas::create", json!({
        "name": "Handshake", "format": "mermaid", "source": source,
    })).await?;
    // { "id": "a1b2c3d4", "name": "Handshake", "format": "mermaid",
    //   "family": "sequence", "source": "…", "created_at": …, "updated_at": … }
    println!("stored canvas {}", record["id"]);
    Ok(())
}
```

Revisions go through `canvas::update` with the id: the id never changes,
`updated_at` is stamped, and for mermaid the diagram family is re-derived
from the new source. `canvas::get` reads one back, `canvas::list` returns
everything newest first with an optional format filter, and `canvas::delete`
reports `deleted: false` on an unknown id rather than erroring.

A freeform canvas takes an excalidraw scene JSON string as its source;
`canvas::validate` checks the scene's shape the same way it parses mermaid.

## Console page

The page at `#/ext/canvas` lists every stored canvas and opens each one for
editing: mermaid source beside its live rendering, a freeform scene on a
drawable whiteboard. In chat, a `canvas::*` call renders as the diagram it
touched, not as JSON.

## Configuration

Configuration lives in the `configuration` worker under the id `canvas` and
every field hot-reloads — handlers read the live snapshot per call, so
nothing needs a restart.

```yaml
max_source_bytes: 2097152   # largest canvas source accepted, in bytes
max_list: 200               # most records canvas::list returns in one response
```

Both fields are bounds: the first keeps an oversized excalidraw scene or
generated mermaid blob off the state bus, the second caps a list response.
Defaults live in [`src/config.rs`](src/config.rs).

## Called on demand

This worker registers no harness hook and injects nothing into any prompt. A
conversation that never draws pays nothing for having it installed. An agent
finds it through the function registry and [`skills/SKILL.md`](skills/SKILL.md);
a person finds it through the console page.
